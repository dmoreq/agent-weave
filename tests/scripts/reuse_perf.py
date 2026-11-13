#!/usr/bin/env python3
"""Pinned reuse latency harness.

Phase 1 uses the Rust integration bench for its merge gates. This harness is
the reproducible external measurement: one pinned binary, a prepared index,
`--no-journal`, fixed `--limit`, optional warmup, and an explicit statistic.
It prints JSON with the binary version, sample count, and latency.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path


def percentile(samples: list[float], fraction: float) -> float:
    ordered = sorted(samples)
    index = max(0, min(len(ordered) - 1, int(len(ordered) * fraction + 0.999999) - 1))
    return ordered[index]


def binary_version(binary: Path) -> str:
    out = subprocess.run(
        [str(binary), "--version"],
        check=False,
        capture_output=True,
        text=True,
    )
    if out.returncode != 0:
        raise RuntimeError(f"version probe failed: {out.stderr.strip()}")
    return out.stdout.strip()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", default="target/release/eg", type=Path)
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--runs", default=50, type=int)
    parser.add_argument("--warmup", default=10, type=int)
    parser.add_argument("--limit", default=10, type=int)
    parser.add_argument("--stat", default="p95", choices=["p50", "p95"])
    args = parser.parse_args()

    binary = args.bin.resolve()
    root = args.root.resolve()
    if not binary.is_file():
        print(f"reuse_perf: binary not found: {binary}", file=sys.stderr)
        return 1
    if not (root / ".engram" / "index.db").is_file():
        print(f"reuse_perf: no index under {root}; run eg build", file=sys.stderr)
        return 1
    if args.runs < 1 or args.warmup < 0:
        print("reuse_perf: runs must be >= 1 and warmup >= 0", file=sys.stderr)
        return 2

    command = [
        str(binary),
        "--root",
        str(root),
        "--no-journal",
        "reuse",
        "--limit",
        str(args.limit),
    ]
    for _ in range(args.warmup):
        subprocess.run(command, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)

    samples_ms: list[float] = []
    for _ in range(args.runs):
        start = time.perf_counter_ns()
        subprocess.run(command, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        samples_ms.append((time.perf_counter_ns() - start) / 1_000_000.0)

    result = {
        "engram_version": binary_version(binary),
        "runs": args.runs,
        "warmup": args.warmup,
        "limit": args.limit,
        "statistic": args.stat,
        "value_ms": round(percentile(samples_ms, 0.95 if args.stat == "p95" else 0.50), 3),
        "min_ms": round(min(samples_ms), 3),
        "max_ms": round(max(samples_ms), 3),
    }
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
