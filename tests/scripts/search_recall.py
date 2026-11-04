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
import re
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


def count_matches(expected: list[str], stdout: str) -> int:
    """Match each expected suffix to at most one returned symbol identity."""
    symbols = [match.group(1) for line in stdout.splitlines()
               if (match := re.match(r"^\S+:\d+\s+(\S+)", line))]
    matched = 0
    for name in sorted(expected, key=len, reverse=True):
        for index, symbol in enumerate(symbols):
            if symbol == name or symbol.endswith("." + name):
                symbols.pop(index)
                matched += 1
                break
    return matched


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--queries", required=True)
    ap.add_argument("--aw-root", required=True)
    ap.add_argument("--vihap-root", required=True)
    ap.add_argument("binary")
    ap.add_argument("--limit", type=int, default=10)
    args = ap.parse_args()

    rows = [json.loads(l) for l in Path(args.queries).read_text().splitlines() if l.strip()]
    if not rows or any(not row.get("expect") for row in rows):
        ap.error("queries and expected symbol lists must not be empty")
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
        if out.returncode not in (0, 1):
            print(f"search failed for {row['query']!r}: exit {out.returncode}", file=sys.stderr)
            return 1
        got = count_matches(row["expect"], out.stdout)
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
