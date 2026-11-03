#!/usr/bin/env python3
"""Synthetic corpus generator for the G17 latency leg (aw Amendment 13).

Amendment 11 measured the performance pass on a 2,025-file / 7.9 MB
synthetic corpus whose generator was never committed; this script recreates
a comparable-scale corpus deterministically (no randomness, fixed layout):
FILES files under <out>/synth/, each a 40-function call chain where every
function calls exactly the next one and a per-file entry calls the head.
That gives the BFS a per-file 40-hop route (deep truncated negatives at the
default depth 10) plus at-cap 10-hop sub-routes.

Scale reference (Amendment 11): 2,025 files, ~7.9 MB source, ~4.6 MB index.
Usage:
    python3 tests/scripts/make_synth_corpus.py <out-dir> [--files 2025]
"""
from __future__ import annotations

import argparse
import sys
from pathlib import Path

FUNCS = 40


def file_body(i: int) -> str:
    lines = [f"def entry_{i:04}():", f"    return f{i:04}_00()"]
    for k in range(FUNCS):
        lines.append(f"def f{i:04}_{k:02}():")
        lines.append(f"    acc_{k:02} = {k} * 2 + {i} % 7")
        lines.append(f"    acc_{k:02} = acc_{k:02} * 3 - {k}")
        lines.append(f"    acc_{k:02} = acc_{k:02} // ({k} + 1)")
        if k < FUNCS - 1:
            lines.append(f"    return acc_{k:02} + f{i:04}_{k + 1:02}()")
        else:
            lines.append(f"    return acc_{k:02}")
    return "\n".join(lines) + "\n"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("out_dir")
    ap.add_argument("--files", type=int, default=2025)
    args = ap.parse_args()
    out = Path(args.out_dir) / "synth"
    out.mkdir(parents=True, exist_ok=True)
    total = 0
    mark = max(1, args.files // 10)
    for i in range(args.files):
        body = file_body(i)
        (out / f"file{i:05}.py").write_text(body)
        total += len(body)
        if (i + 1) % mark == 0:
            print(f"aw-synth: {i + 1}/{args.files} files, {total} bytes", flush=True)
    print(f"aw-synth: done — {args.files} files, {total} bytes under {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
