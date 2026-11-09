#!/usr/bin/env python3
"""G18 — vocabulary-gap recall (eg Amendment 14).

Runs the SAME frozen queries against two indexes built from identical source:
the new binary against a new-built index and the pre-change binary against an
old-built index (signature tokens are derived data, so each binary must query
its own build). A query improves when new recall > old recall. Gate: >= 4/6
queries improved (the frozen set's targets have all their query words in
signature tokens only, so the old index misses them).

Usage:
    python3 tests/scripts/search_gap_recall.py \
        --queries docs/baselines/search-gap-gt.jsonl \
        --new-eg <dir> --new-vihap <dir> \
        --old-eg <dir> --old-vihap <dir> \
        <new-binary> <old-binary>
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path


def recall(binary: str, root: str, query: str, expect: list[str], limit: int) -> float:
    out = subprocess.run(
        [binary, "search", query, "--root", root, "--limit", str(limit)],
        capture_output=True, text=True,
    )
    lines = out.stdout.splitlines()
    hits = sum(1 for e in expect if any(e in l for l in lines))
    return hits / len(expect)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--queries", required=True)
    ap.add_argument("--new-eg", required=True)
    ap.add_argument("--new-vihap", required=True)
    ap.add_argument("--old-eg", required=True)
    ap.add_argument("--old-vihap", required=True)
    ap.add_argument("--limit", type=int, default=50)
    ap.add_argument("new_binary")
    ap.add_argument("old_binary")
    args = ap.parse_args()

    rows = [json.loads(l) for l in Path(args.queries).read_text().splitlines() if l.strip()]
    new_roots = {"eg": args.new_eg, "aw": args.new_eg, "vihap": args.new_vihap}
    old_roots = {"eg": args.old_eg, "aw": args.old_eg, "vihap": args.old_vihap}

    print("| query | repo | old recall | new recall | improved |")
    print("|-------|------|-----------|-----------|----------|")
    improved = 0
    for row in rows:
        o = recall(args.old_binary, old_roots[row["repo"]], row["query"], row["expect"], args.limit)
        n = recall(args.new_binary, new_roots[row["repo"]], row["query"], row["expect"], args.limit)
        better = n > o
        improved += better
        print(f"| {row['query']} | {row['repo']} | {o:.2f} | {n:.2f} | {'YES' if better else 'NO'} |")
    print(f"\nimproved: {improved}/{len(rows)} (gate: >= 4/6)")
    return 0 if improved >= 4 else 1


if __name__ == "__main__":
    sys.exit(main())
