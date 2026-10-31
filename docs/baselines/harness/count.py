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


def main():
    files = sorted(glob.glob(os.path.join(TR, "T*.jsonl")))
    if not files:
        print("no transcripts found", file=sys.stderr)
        return 1
    print("| task | tool calls | in bytes | out bytes | total bytes |")
    print("|------|-----------|----------|-----------|-------------|")
    grand = [0, 0, 0, 0]
    for path in files:
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
    return 0


if __name__ == "__main__":
    sys.exit(main())
