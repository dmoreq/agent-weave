#!/usr/bin/env python3
"""G14 — symbol-search recall gate (aw Amendment 12b).

Reads docs/baselines/search-gt.jsonl (frozen BEFORE implementation) and runs
`aw search <query> --limit 10` against the ground-truth repos. A query line
carries: repo ("aw" -> --root <aw-repo>, "vihap" -> --root <vihap-repo>),
query (space-separated NL/identifier terms), expect (qualname suffixes that
must appear in the top-10 results).

Scoring: per-query recall = matched expects / total expects; reported as the
mean over queries plus the pooled symbol-level rate with a Wilson 95%
interval. Gate: mean recall >= 0.70. Usage:
    python3 tests/scripts/search_recall.py --queries <gt.jsonl> <aw-binary> \
        --aw-root <path> --vihap-root <path>
"""
from __future__ import annotations

import argparse
import json
import math
import subprocess
import sys
from pathlib import Path


def wilson(p: float, n: int, z: float = 1.959963985) -> tuple[float, float]:
    if n == 0:
        return (0.0, 0.0)
    denom = 1 + z * z / n
    center = p + z * z / (2 * n)
    spread = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n))
    return ((center - spread) / denom, (center + spread) / denom)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--queries", required=True)
    ap.add_argument("--aw-root", required=True)
    ap.add_argument("--vihap-root", required=True)
    ap.add_argument("binary")
    ap.add_argument("--limit", type=int, default=10)
    args = ap.parse_args()

    rows = [json.loads(l) for l in Path(args.queries).read_text().splitlines() if l.strip()]
    roots = {"aw": args.aw_root, "vihap": args.vihap_root}

    print("| query | hits | expects | recall |")
    print("|-------|------|---------|--------|")
    per_query = []
    hits_total = 0
    expects_total = 0
    for row in rows:
        root = roots[row["repo"]]
        out = subprocess.run(
            [args.binary, "search", row["query"], "--limit", str(args.limit), "--root", root],
            capture_output=True,
            text=True,
        )
        stdout = out.stdout
        got = 0
        for expect in row["expect"]:
            # qualname suffix match against the pointer text; qualnames in
            # search output appear after `path:line ` — match on any line.
            tail = expect.split(".", 1)[-1] if "." in expect else expect
            needle = expect.split(".")[-1] if expect.split(".")[-1] == tail else tail
            if any(expect in l or f" {needle}" in l for l in stdout.splitlines()):
                got += 1
        n = len(row["expect"])
        per_query.append((row["query"], got, n))
        hits_total += got
        expects_total += n
        print(f"| {row['query']} | {got} | {n} | {got / n:.2f} |")

    mean = sum(g / n for _, g, n in per_query) / len(per_query)
    pooled = hits_total / expects_total
    lo, hi = wilson(pooled, expects_total)
    print(f"\nmean per-query recall: {mean:.3f}")
    print(f"pooled symbol recall: {hits_total}/{expects_total} = {pooled:.3f} "
          f"(Wilson 95% [{lo:.3f}, {hi:.3f}])")
    verdict = "PASS" if mean >= 0.70 else "FAIL"
    print(f"G14: {verdict} (bar: mean recall >= 0.70)")
    return 0 if mean >= 0.70 else 1


if __name__ == "__main__":
    sys.exit(main())
