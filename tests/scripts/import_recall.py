#!/usr/bin/env python3
"""F7 real-corpus import recall gate.

Reads a frozen JSONL set of source-verified import rows:
  {"repo": "vihap"|"kg"|"finguard", "file": "...", "line": N, "target": "..."}
and checks that `kg imports <file>` emits `file:line target`. GT was built from
the source ASTs (Python `ast`, tree-sitter Rust/TS) before measuring kg, not
from kg output.

Usage:
    python3 tests/scripts/import_recall.py \
      --pairs docs/baselines/import-gt.jsonl \
      --kg-root <kg> --vihap-root <vihap> --finguard-root <finguard> <binary>
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from precision_recall import wilson


def data_lines(stdout: str) -> list[str]:
    return [
        line
        for line in stdout.splitlines()
        if not line.startswith("kg:") and not line.startswith("showing ")
    ]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pairs", required=True)
    ap.add_argument("--kg-root", required=True)
    ap.add_argument("--vihap-root", required=True)
    ap.add_argument("--finguard-root", default="")
    ap.add_argument("--min-recall", type=float, default=0.90)
    ap.add_argument("binary")
    args = ap.parse_args()
    roots = {
        "kg": args.kg_root,
        "vihap": args.vihap_root,
        "finguard": args.finguard_root,
    }
    rows = [
        json.loads(line)
        for line in Path(args.pairs).read_text().splitlines()
        if line.strip()
    ]
    hits = 0
    missed: set[str] = set()
    for row in rows:
        root = roots.get(row["repo"], "")
        if not root:
            missed.add(f"{row['repo']}: no root supplied")
            continue
        proc = subprocess.run(
            [args.binary, "imports", row["file"], "--root", root],
            capture_output=True,
            text=True,
        )
        expected = f"{row['file']}:{row['line']} {row['target']}"
        if proc.returncode == 0 and expected in data_lines(proc.stdout):
            hits += 1
        else:
            missed.add(expected)

    recall, lo, hi = wilson(hits, len(rows))
    print("| repo | file:line -> target | ok |")
    print("|------|---------------------|----|")
    for row in rows:
        expected = f"{row['file']}:{row['line']} {row['target']}"
        print(f"| {row['repo']} | {expected} | {'FAIL' if expected in missed else 'OK'} |")
    for miss in sorted(missed):
        print(f"MISS: {miss}", file=sys.stderr)
    print(f"\nrecall: {hits}/{len(rows)} = {recall:.3f} "
          f"(Wilson 95% {lo:.3f}-{hi:.3f}; gate >= {args.min_recall:.2f})")
    return 0 if recall >= args.min_recall else 1


if __name__ == "__main__":
    sys.exit(main())
