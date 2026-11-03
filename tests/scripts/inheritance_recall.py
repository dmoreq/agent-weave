#!/usr/bin/env python3
"""F1 real-corpus inheritance recall gate.

Reads frozen JSONL rows:
  {"repo": "vihap"|"finguard", "base": "...", "child": "..."}
and checks `aw implementers <base>` lists the child qualname. Python
same-file and cross-file inheritance plus TS same-file interface extension are
covered; cross-file Rust impls and tsconfig path aliases are excluded by the
pinned F1 limits.

Usage:
    python3 tests/scripts/inheritance_recall.py \
      --pairs docs/baselines/inheritance-gt.jsonl \
      --vihap-root <vihap> --finguard-root <finguard> <binary>
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from precision_recall import wilson


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pairs", required=True)
    ap.add_argument("--vihap-root", required=True)
    ap.add_argument("--finguard-root", default="")
    ap.add_argument("--min-recall", type=float, default=1.0)
    ap.add_argument("binary")
    args = ap.parse_args()
    roots = {"vihap": args.vihap_root, "finguard": args.finguard_root}
    rows = [
        json.loads(line)
        for line in Path(args.pairs).read_text().splitlines()
        if line.strip()
    ]
    hits = 0
    missed: set[tuple[str, str]] = set()
    for row in rows:
        root = roots.get(row["repo"], "")
        if not root:
            missed.add((row["base"], row["child"]))
            continue
        proc = subprocess.run(
            [
                args.binary,
                "implementers",
                row["base"],
                "--limit",
                "10000",
                "--root",
                root,
            ],
            capture_output=True,
            text=True,
        )
        found = proc.returncode == 0 and any(
            row["child"] in line
            for line in proc.stdout.splitlines()
            if not line.startswith("aw:") and not line.startswith("showing ")
        )
        if found:
            hits += 1
        else:
            missed.add((row["base"], row["child"]))

    recall, lo, hi = wilson(hits, len(rows))
    print("| base | child | ok |")
    print("|------|-------|----|")
    for row in rows:
        key = (row["base"], row["child"])
        print(f"| {key[0]} | {key[1]} | {'FAIL' if key in missed else 'OK'} |")
    for base, child in sorted(missed):
        print(f"MISS: {base} -> {child}", file=sys.stderr)
    print(f"\nrecall: {hits}/{len(rows)} = {recall:.3f} "
          f"(Wilson 95% {lo:.3f}-{hi:.3f}; gate >= {args.min_recall:.2f})")
    return 0 if recall >= args.min_recall else 1


if __name__ == "__main__":
    sys.exit(main())
