#!/usr/bin/env python3
"""G17 — call-path correctness gate (eg Amendment 13).

Reads docs/baselines/path-gt.jsonl (frozen 2026-11-02; every reachable
pair's hop edges were verified EXTRACTED in the index at freeze, and every
no_path pair's negative was verified structurally — zero out-edges or zero
cross-references) and runs `eg path <from> <to>` against the ground-truth
repos. A pair line carries: repo ("eg" | "vihap"), from, to, and either
expect_hops (int) or expect ("no_path").

Scoring: a reachable pair agrees when the exit code is 0 and the chain line
reports exactly expect_hops; a no_path pair agrees when the exit code is 1
and stderr states "no call path". Gate: 100% agreement. Mismatch routing is
a gate-day analysis rule, not script logic: a missing or moved hop edge is
resolution-attributable (route to the G2 retune), a wrong route or byte
shape is a path-feature kill.

Usage:
    python3 tests/scripts/path_recall.py --pairs docs/baselines/path-gt.jsonl \
        <eg-binary> --eg-root <path> --vihap-root <path>
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

HOPS = re.compile(r"\((\d+) hops\)")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pairs", required=True)
    ap.add_argument("--eg-root", required=True)
    ap.add_argument("--vihap-root", required=True)
    ap.add_argument("binary")
    args = ap.parse_args()

    rows = [json.loads(l) for l in Path(args.pairs).read_text().splitlines() if l.strip()]
    roots = {"eg": args.eg_root, "aw": args.eg_root, "vihap": args.vihap_root}

    print("| repo | from -> to | expect | got | ok |")
    print("|------|------------|--------|-----|----|")
    n_ok = 0
    for row in rows:
        root = roots[row["repo"]]
        out = subprocess.run(
            [args.binary, "path", row["from"], row["to"], "--root", root],
            capture_output=True,
            text=True,
        )
        expect = row.get("expect_hops", row.get("expect"))
        if out.returncode == 0:
            m = None
            for line in out.stdout.splitlines():
                m = HOPS.search(line)
                if m:
                    break
            got = int(m.group(1)) if m else "error: no chain line"
        elif out.returncode == 1 and "no call path" in out.stderr:
            got = "no_path"
        else:
            got = f"error: exit {out.returncode}: {out.stderr.strip()[:80]}"
        ok = got == expect
        n_ok += ok
        print(f"| {row['repo']} | {row['from']} -> {row['to']} | {expect} | {got} | {'OK' if ok else 'FAIL'} |")

    print(f"\nagreement: {n_ok}/{len(rows)} (gate: 100%)")
    return 0 if n_ok == len(rows) else 1


if __name__ == "__main__":
    sys.exit(main())
