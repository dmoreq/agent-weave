#!/usr/bin/env python3
"""D0 harness: byte/call counter for baseline transcripts.

Reads docs/baselines/transcripts/<TASK>.jsonl (JSONL records written by harness/g)
and prints per-task and total tool-call and byte counts. Both directions:
in_bytes = agent tool-call input (command string), out_bytes = tool-result bytes
returned to the agent. Exits 0 always; prints a markdown table.
"""
import glob
import json
import os
import sys

TR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "transcripts")


def load_task(path):
    calls = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line:
                calls.append(json.loads(line))
    return calls


def table(paths, label):
    print(f"\n### {label}\n")
    print("| task | tool calls | in bytes | out bytes | total bytes |")
    print("|------|-----------|----------|-----------|-------------|")
    grand = [0, 0, 0, 0]
    for path in paths:
        calls = load_task(path)
        task = os.path.basename(path)[:2]
        n = len(calls)
        ib = sum(c["in_bytes"] for c in calls)
        ob = sum(c["out_bytes"] + c["err_bytes"] for c in calls)
        print(f"| {task} | {n} | {ib} | {ob} | {ib + ob} |")
        grand[0] += n
        grand[1] += ib
        grand[2] += ob
        grand[3] += ib + ob
    print(f"| TOTAL | {grand[0]} | {grand[1]} | {grand[2]} | {grand[3]} |")
    return grand


def main():
    baseline = sorted(glob.glob(os.path.join(TR, "T[1-5].jsonl")))
    kgarm = sorted(glob.glob(os.path.join(TR, "kgarm", "T[1-5].jsonl")))
    # Optional positional arm dir (relative to transcripts/), e.g. kgoutlinearm
    # for the G13 outline-first gate (Amendment 12).
    arm_arg = sys.argv[1] if len(sys.argv) > 1 else None
    extra = sorted(glob.glob(os.path.join(TR, arm_arg, "T[1-5].jsonl"))) if arm_arg else []
    if not baseline and not kgarm and not extra:
        print("no transcripts found", file=sys.stderr)
        return 1
    if baseline:
        table(baseline, "baseline arm (grep)")
    if kgarm:
        table(kgarm, "treatment arm (kg)")
    if extra:
        table(extra, f"arm ({arm_arg})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
