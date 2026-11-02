#!/usr/bin/env python3
"""F8 real-corpus test-impact recall gate.

Reads frozen JSONL rows:
  {"repo": "vihap", "target": "...", "test_file": "...", "test_line": N}
Ground truth was derived from Python AST module-level imports and test calls;
function-local imports are excluded as a pinned F8 limit.

Usage:
    python3 tests/scripts/tests_recall.py \
      --pairs docs/baselines/tests-gt.jsonl \
      --vihap-root <vihap> <binary>
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
    ap.add_argument("--min-recall", type=float, default=0.90)
    ap.add_argument("--min-precision", type=float, default=0.90)
    ap.add_argument("binary")
    args = ap.parse_args()
    rows = [
        json.loads(line)
        for line in Path(args.pairs).read_text().splitlines()
        if line.strip()
    ]
    hits = 0
    missed: set[tuple[str, int]] = set()
    emitted = 0
    test_shaped = 0
    for row in rows:
        proc = subprocess.run(
            [
                args.binary,
                "tests",
                row["target"],
                "--limit",
                "10000",
                "--root",
                args.vihap_root,
            ],
            capture_output=True,
            text=True,
        )
        found = False
        if proc.returncode == 0:
            prefix = f"{row['test_file']}:{row['test_line']} "
            found = any(
                line.startswith(prefix)
                for line in proc.stdout.splitlines()
                if not line.startswith("kg:") and not line.startswith("showing ")
            )
        if found:
            hits += 1
        else:
            missed.add((row["test_file"], row["test_line"]))
        if proc.returncode == 0:
            for line in proc.stdout.splitlines():
                if line.startswith("kg:") or line.startswith("showing "):
                    continue
                parts = line.split(" ", 1)
                if len(parts) != 2 or ":" not in parts[0]:
                    continue
                qualname = parts[1].split(" ", 1)[-1]
                leaf = qualname.rsplit(".", 1)[-1]
                emitted += 1
                if leaf.startswith("test_") or (
                    leaf.startswith("test")
                    and len(leaf) > 4
                    and leaf[4].isascii()
                    and leaf[4].isupper()
                ):
                    test_shaped += 1

    precision, plo, phi = wilson(test_shaped, emitted) if emitted else (0.0, 0.0, 0.0)
    recall, lo, hi = wilson(hits, len(rows))
    print("| target | test file:line | ok |")
    print("|--------|----------------|----|")
    for row in rows:
        key = (row["test_file"], row["test_line"])
        print(f"| {row['target']} | {key[0]}:{key[1]} | {'FAIL' if key in missed else 'OK'} |")
    for path, line in sorted(missed):
        print(f"MISS: {path}:{line}", file=sys.stderr)
    print(f"\nrecall: {hits}/{len(rows)} = {recall:.3f} "
          f"(Wilson 95% {lo:.3f}-{hi:.3f}; gate >= {args.min_recall:.2f})")
    print(f"precision: {test_shaped}/{emitted} = {precision:.3f} "
          f"(Wilson 95% {plo:.3f}-{phi:.3f}; gate >= {args.min_precision:.2f})")
    return 0 if recall >= args.min_recall and precision >= args.min_precision else 1


if __name__ == "__main__":
    sys.exit(main())
