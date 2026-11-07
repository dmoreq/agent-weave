#!/usr/bin/env python3
"""E1.2 — `aw callees` precision/recall and empty-answer semantics.

The fixture has 20 known symbols: 10 with resolved callee sets, 5 with only
unresolved call sites, and 5 with ambiguous diamond receivers. The script
copies the fixture, builds it, queries every symbol, and reports:

- micro precision/recall over the resolved rows;
- ambiguous-row hits (the `[ambiguous]` candidate must appear);
- silent-empty count: a zero result that never mentions unresolved sites.

Usage:
    python3 -I tests/scripts/callees_precision.py \
        --aw target/release/aw \
        --json docs/baselines/2026-11-05_0.4.0/e1.2.json

Exit 0 after a completed measurement; with --strict, exit 1 when the gate
thresholds fail.
"""
from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from precision_recall import wilson

ROOT = Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "tests" / "fixtures" / "callees"
POINTER_RE = re.compile(r"^(\S+?):(\d+)(?:-\d+)?\s+(.*)$")
AMBIG = "[ambiguous]"


def run_aw(aw: str, root: Path, args: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(
        [aw, "--root", str(root), "--no-journal", *args],
        capture_output=True,
        text=True,
        encoding="utf-8",
        timeout=120,
    )


def parse_callees(stdout: str) -> tuple[list[str], list[str]]:
    resolved: list[str] = []
    ambiguous: list[str] = []
    for raw in stdout.splitlines():
        line = raw.strip()
        if not line or line.startswith(("aw:", "showing ")):
            continue
        match = POINTER_RE.match(line)
        if not match:
            continue
        detail = match.group(3).strip()
        if detail.startswith(AMBIG):
            ambiguous.append(detail[len(AMBIG) :].strip())
        elif detail:
            resolved.append(detail)
    return resolved, ambiguous


def load_oracle(path: Path) -> list[dict]:
    return [
        json.loads(line)
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--aw", required=True)
    parser.add_argument("--json", required=True)
    parser.add_argument("--fixture", default=str(FIXTURE))
    parser.add_argument("--strict", action="store_true")
    args = parser.parse_args()

    fixture = Path(args.fixture)
    oracle_path = fixture / "oracle.jsonl"
    oracle = load_oracle(oracle_path)
    version = subprocess.run(
        [args.aw, "--version"], capture_output=True, text=True, encoding="utf-8", timeout=30
    )

    with tempfile.TemporaryDirectory(prefix="aw-callees-") as tmp:
        corpus = Path(tmp) / "corpus"
        shutil.copytree(fixture, corpus, ignore=shutil.ignore_patterns(".aw", "oracle.jsonl"))
        build = run_aw(args.aw, corpus, ["build"])
        if build.returncode != 0:
            print(f"build failed: {build.stderr[-500:]}", file=sys.stderr)
            return 2

        unresolved_names_proc = run_aw(args.aw, corpus, ["unresolved", "--names", "--limit", "10000"])
        unresolved_names_text = unresolved_names_proc.stdout

        resolved_rows: list[dict] = []
        ambiguous_rows: list[dict] = []
        unresolved_rows: list[dict] = []

        for row in oracle:
            symbol = row["symbol"]
            kind = row["kind"]
            proc = run_aw(args.aw, corpus, ["callees", symbol, "--limit", "1000"])
            resolved, ambiguous = parse_callees(proc.stdout)

            if kind == "resolved":
                expected = row["expect_resolved"]
                emitted = set(resolved)
                wanted = set(expected)
                correct = emitted & wanted
                resolved_rows.append(
                    {
                        "symbol": symbol,
                        "exit": proc.returncode,
                        "expected": sorted(wanted),
                        "emitted": sorted(emitted),
                        "correct": sorted(correct),
                        "false_positives": sorted(emitted - wanted),
                        "false_negatives": sorted(wanted - emitted),
                    }
                )
            elif kind == "ambiguous":
                expected = row["expect_ambiguous"]
                hits = [name for name in expected if name in ambiguous]
                ambiguous_rows.append(
                    {
                        "symbol": symbol,
                        "exit": proc.returncode,
                        "expected": expected,
                        "ambiguous": ambiguous,
                        "hits": hits,
                    }
                )
            elif kind == "unresolved":
                names = row["expect_unresolved_names"]
                lower = (proc.stdout + proc.stderr).lower()
                silent = proc.returncode == 1 and "unresolved" not in lower
                names_present = [name for name in names if name in unresolved_names_text]
                unresolved_rows.append(
                    {
                        "symbol": symbol,
                        "exit": proc.returncode,
                        "expected_unresolved_names": names,
                        "names_present": names_present,
                        "silent_empty": silent,
                        "stdout": proc.stdout.strip()[:200],
                    }
                )

        emitted_total = sum(len(row["emitted"]) for row in resolved_rows)
        expected_total = sum(len(row["expected"]) for row in resolved_rows)
        correct_total = sum(len(row["correct"]) for row in resolved_rows)
        false_positives = sum(len(row["false_positives"]) for row in resolved_rows)
        false_negatives = sum(len(row["false_negatives"]) for row in resolved_rows)
        precision = correct_total / emitted_total if emitted_total else 0.0
        recall = correct_total / expected_total if expected_total else 0.0
        precision_ci = wilson(correct_total, emitted_total)
        recall_ci = wilson(correct_total, expected_total)

        ambiguous_hits = sum(1 for row in ambiguous_rows if len(row["hits"]) == len(row["expected"]))
        silent_empty = sum(1 for row in unresolved_rows if row["silent_empty"])
        names_present = sum(
            1 for row in unresolved_rows if len(row["names_present"]) == len(row["expected_unresolved_names"])
        )

        gate = (
            precision >= 0.95
            and recall >= 0.95
            and silent_empty == 0
            and ambiguous_hits == len(ambiguous_rows)
        )

        payload = {
            "experiment": "E1.2",
            "aw_version": version.stdout.strip(),
            "fixture": str(fixture),
            "resolved": {
                "rows": resolved_rows,
                "emitted": emitted_total,
                "expected": expected_total,
                "correct": correct_total,
                "false_positives": false_positives,
                "false_negatives": false_negatives,
                "precision": round(precision, 4),
                "recall": round(recall, 4),
                "precision_wilson95": [round(x, 4) for x in precision_ci],
                "recall_wilson95": [round(x, 4) for x in recall_ci],
            },
            "ambiguous": {
                "rows": ambiguous_rows,
                "hits": ambiguous_hits,
                "total": len(ambiguous_rows),
            },
            "unresolved": {
                "rows": unresolved_rows,
                "silent_empty": silent_empty,
                "names_present": names_present,
                "total": len(unresolved_rows),
            },
            "gate": gate,
        }

        Path(args.json).parent.mkdir(parents=True, exist_ok=True)
        Path(args.json).write_text(json.dumps(payload, indent=2), encoding="utf-8")

        print(
            f"resolved: precision={precision:.3f} recall={recall:.3f} "
            f"tp={correct_total} fp={false_positives} fn={false_negatives}"
        )
        print(
            f"ambiguous: hits={ambiguous_hits}/{len(ambiguous_rows)}; "
            f"unresolved: silent_empty={silent_empty}/{len(unresolved_rows)} "
            f"names_present={names_present}/{len(unresolved_rows)}"
        )
        print(f"gate={'PASS' if gate else 'FAIL'}")

    return 0 if not args.strict or gate else 1


if __name__ == "__main__":
    sys.exit(main())
