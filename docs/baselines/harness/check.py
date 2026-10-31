#!/usr/bin/env python3
"""D0 correctness checker.

Compares per-task answer JSON against frozen expected JSON.

Shapes (both sides, docs/baselines/{expected,transcripts}/<TASK>.answer.json and
expected/<TASK>.json):
  pointers:  list of "path:line" strings (or bare "path")  -> precision/recall
  ordering:  optional list of stage strings                -> positional match %
Match rule: pointer exact string match after whitespace strip; a "path:line"
pointer never matches a bare "path" expectation (line precision is the contract).

Usage: check.py [TASK ...]   (default: all tasks with both files present)
"""
import json
import os
import sys

BASE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")


def load(path):
    with open(path) as f:
        return json.load(f)


def set_scores(answer, expected):
    a = {p.strip() for p in answer}
    e = {p.strip() for p in expected}
    hit = a & e
    precision = len(hit) / len(a) if a else 0.0
    recall = len(hit) / len(e) if e else 0.0
    return precision, recall, sorted(a - e), sorted(e - a)


def order_score(answer, expected):
    n = max(len(answer), len(expected))
    if n == 0:
        return 1.0
    good = sum(1 for i in range(min(len(answer), len(expected)))
               if answer[i].strip() == expected[i].strip())
    return good / n


def main():
    tasks = sys.argv[1:]
    if not tasks:
        tasks = sorted({f[:2] for f in os.listdir(os.path.join(BASE, "expected"))
                        if f.endswith(".json")})
    print("| task | precision | recall | ordering |")
    print("|------|-----------|--------|----------|")
    for t in tasks:
        exp = load(os.path.join(BASE, "expected", f"{t}.json"))
        ans = load(os.path.join(BASE, "transcripts", f"{t}.answer.json"))
        p, r, fp, fn = set_scores(ans.get("pointers", []), exp.get("pointers", []))
        o = order_score(ans.get("ordering", []), exp.get("ordering", []))
        print(f"| {t} | {p:.2%} | {r:.2%} | {o:.0%} |")
        if fp:
            print(f"  {t} false positives: {', '.join(fp)}")
        if fn:
            print(f"  {t} misses: {', '.join(fn)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
