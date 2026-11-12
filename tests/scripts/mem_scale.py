#!/usr/bin/env python3
"""Scale measurement for the memory dual-FTS index.

Generates a synthetic store, imports it through `eg mem import`, measures the
v2 repair triggered by one write open, and reports write latency percentiles.
`--writes-only` measures the pre-change write baseline against a 1.0.4 binary
without touching the v2 structures.

The script is stdlib-only and uses the same temp-repo and `run_eg` helpers as
the recall benchmarks. It is evidence, not a CI gate, unless `--baseline` is
passed: then it exits 1 when import >= 1 s, repair >= 1 s, or write p95 is more
than 1 ms above the baseline.
"""
from __future__ import annotations

import argparse
import json
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from precision_recall import run_eg


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    idx = min(len(ordered) - 1, max(0, round((pct / 100.0) * (len(ordered) - 1))))
    return round(ordered[idx], 3)


def generate_notes(path: Path, count: int, seed: int) -> None:
    with path.open("w", encoding="utf-8") as f:
        for i in range(count):
            body = (
                f"Synthetic note {i} seed {seed} records resolver ranking memory "
                f"invariant for scale measurement."
            )
            f.write(
                json.dumps(
                    {
                        "key": f"note-{i}",
                        "kind": "fact",
                        "scope": "durable",
                        "body": body,
                        "tags": "scale resolver",
                        "refs": "src/memory/rank.rs",
                    },
                    ensure_ascii=False,
                )
                + "\n"
            )


def db_bytes(path: Path) -> dict[str, int]:
    conn = sqlite3.connect(path)
    try:
        total = path.stat().st_size
        legacy = 0
        v2 = 0
        try:
            legacy = int(
                conn.execute(
                    "SELECT COALESCE(SUM(pgsize),0) FROM dbstat "
                    "WHERE name GLOB 'memories_fts_*' AND name NOT GLOB 'memories_fts_v2_*'"
                ).fetchone()[0]
            )
            v2 = int(
                conn.execute(
                    "SELECT COALESCE(SUM(pgsize),0) FROM dbstat "
                    "WHERE name GLOB 'memories_fts_v2_*'"
                ).fetchone()[0]
            )
        except sqlite3.Error:
            pass
        return {"store_bytes": total, "fts_legacy_bytes": legacy, "fts_v2_bytes": v2}
    finally:
        conn.close()


def measure_writes(eg: str, root: Path, writes: int) -> list[float]:
    times: list[float] = []
    for i in range(writes):
        start = time.perf_counter()
        proc = run_eg(eg, root, ["mem", "write", f"probe-{i}", "--body", f"probe {i}", "--kind", "fact"])
        elapsed = (time.perf_counter() - start) * 1000.0
        if proc.returncode != 0:
            raise RuntimeError(f"mem write probe-{i} failed ({proc.returncode}): {proc.stderr.strip()}")
        times.append(elapsed)
    return times


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--eg", required=True)
    ap.add_argument("--notes", type=int, default=5000)
    ap.add_argument("--seed", type=int, default=20261112)
    ap.add_argument("--writes", type=int, default=100)
    ap.add_argument("--writes-only", action="store_true")
    ap.add_argument("--baseline", default=None)
    ap.add_argument("--json", default=None)
    args = ap.parse_args()

    if not (1 <= args.notes <= 5000):
        print("notes must be between 1 and 5000", file=sys.stderr)
        return 2

    root = Path(tempfile.mkdtemp(prefix="eg-mem-scale-"))
    report: dict[str, object] = {
        "notes": args.notes,
        "seed": args.seed,
        "writes": args.writes,
        "writes_only": args.writes_only,
        "import_seconds": None,
        "repair_seconds": None,
        "write_p50_ms": None,
        "write_p95_ms": None,
        "store_bytes": None,
        "fts_legacy_bytes": None,
        "fts_v2_bytes": None,
        "sqlite_version": sqlite3.sqlite_version,
    }
    try:
        subprocess.run(["git", "init", "-q", str(root)], check=True)
        notes_path = root / "notes.jsonl"
        generate_notes(notes_path, args.notes, args.seed)

        import time

        start = time.perf_counter()
        proc = run_eg(args.eg, root, ["mem", "import", str(notes_path)])
        report["import_seconds"] = round(time.perf_counter() - start, 3)
        if proc.returncode != 0:
            raise RuntimeError(f"mem import failed ({proc.returncode}): {proc.stderr.strip()}")

        if not args.writes_only:
            db = root / ".engram" / "memory.db"
            conn = sqlite3.connect(db)
            try:
                conn.execute("DROP TABLE IF EXISTS memories_fts_v2")
                conn.execute("UPDATE memories SET fts_version = 0")
                conn.execute("DELETE FROM memory_meta WHERE key = 'fts_v2_complete'")
                conn.commit()
            finally:
                conn.close()

            start = time.perf_counter()
            proc = run_eg(args.eg, root, ["mem", "write", "repair-probe", "--body", "repair probe", "--kind", "fact"])
            report["repair_seconds"] = round(time.perf_counter() - start, 3)
            if proc.returncode != 0:
                raise RuntimeError(f"repair write failed ({proc.returncode}): {proc.stderr.strip()}")

            db = root / ".engram" / "memory.db"
            report.update(db_bytes(db))
        else:
            report["repair_seconds"] = None

        times = measure_writes(args.eg, root, args.writes)
        report["write_p50_ms"] = percentile(times, 50)
        report["write_p95_ms"] = percentile(times, 95)
        db = root / ".engram" / "memory.db"
        report.update(db_bytes(db))
    finally:
        shutil.rmtree(root, ignore_errors=True)

    print(json.dumps(report, indent=2))
    if args.json:
        out = Path(args.json)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    if args.baseline:
        baseline = json.loads(Path(args.baseline).read_text(encoding="utf-8"))
        base_p95 = float(baseline.get("write_p95_ms") or 0.0)
        ok = (
            report["import_seconds"] is None or float(report["import_seconds"]) < 1.0
        ) and (
            report["repair_seconds"] is None or float(report["repair_seconds"]) < 1.0
        ) and float(report["write_p95_ms"]) <= base_p95 + 1.0
        if not ok:
            print(
                f"scale gate failed: import={report['import_seconds']} "
                f"repair={report['repair_seconds']} p95={report['write_p95_ms']} "
                f"baseline_p95={base_p95}",
                file=sys.stderr,
            )
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
