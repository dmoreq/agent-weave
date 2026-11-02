#!/usr/bin/env python3
"""Stage-1 utility A/B scorer (kg path, Amendment 13).

Compares each cell's answer JSON against the frozen expected set:
  expected/P*.json  {"exists": bool, "hops": int}
Correctness = exact exists match AND exact hops match AND route well-formed:
  - every entry matches "<repo-relative-path>:<line>"
  - the file exists under the task repo and the line is within bounds
  - len(route) == hops when exists, == 0 otherwise
Usage: python3 path_check.py [runid ...]   (default: all cells in status.txt)
Writes a markdown table to stdout.
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path("/Users/quy.doan/Workspace/kg/docs/baselines/utility-run-2026-11-02")
EXP = Path("/Users/quy.doan/Workspace/kg/docs/baselines/expected")
REPOS = {
    "P1": Path("/tmp/utility-corpus-2026-11-02/kg"),
    "P2": Path("/tmp/utility-corpus-2026-11-02/kg"),
    "P3": Path("/tmp/utility-corpus-2026-11-02/vihap"),
}
PTR = re.compile(r"^(.+):(\d+)$")


def line_count(p: Path) -> int:
    try:
        return len(p.read_text(errors="replace").splitlines())
    except OSError:
        return 0


def score(runid: str) -> dict:
    parts = runid.split("-")
    arm = parts[1]
    task = parts[3]
    exp = json.loads((EXP / f"{task}.json").read_text())
    ans_path = ROOT / "answers" / f"{runid}.json"
    out = {"runid": runid, "arm": arm, "task": task, "exists": None,
           "hops": None, "ok": False, "why": ""}
    if not ans_path.exists():
        out["why"] = "no answer file"
        return out
    try:
        ans = json.loads(ans_path.read_text())
    except json.JSONDecodeError as e:
        out["why"] = f"invalid JSON: {e}"
        return out
    out["exists"] = ans.get("exists")
    out["hops"] = ans.get("hops")
    route = ans.get("route") or []
    repo = REPOS[task]
    if out["exists"] != exp["exists"]:
        out["why"] = f"exists {out['exists']} != {exp['exists']}"
        return out
    if out["hops"] != exp["hops"]:
        out["why"] = f"hops {out['hops']} != {exp['hops']}"
        return out
    if not isinstance(route, list):
        out["why"] = "route not a list"
        return out
    if len(route) != (exp["hops"] if exp["exists"] else 0):
        out["why"] = f"route length {len(route)} != hops"
        return out
    for item in route:
        m = PTR.match(str(item))
        if not m:
            out["why"] = f"bad pointer {item!r}"
            return out
        rel, line = m.group(1), int(m.group(2))
        f = repo / rel
        if not f.is_file():
            out["why"] = f"missing file {rel}"
            return out
        if line < 1 or line > line_count(f):
            out["why"] = f"line {line} out of bounds in {rel}"
            return out
    out["ok"] = True
    return out


def main() -> int:
    runids = sys.argv[1:]
    if not runids:
        seen = []
        status = ROOT / "status.txt"
        if status.exists():
            for line in status.read_text().splitlines():
                m = re.match(r"\[start\] (\S+) ", line)
                if m:
                    seen.append(m.group(1))
        runids = seen
    print("| runid | exists | hops | ok | why |")
    print("|-------|--------|------|----|-----|")
    ok_n = 0
    for r in runids:
        s = score(r)
        ok_n += s["ok"]
        print(f"| {s['runid']} | {s['exists']} | {s['hops']} | "
              f"{'OK' if s['ok'] else 'FAIL'} | {s['why']} |")
    print(f"\ncorrect cells: {ok_n}/{len(runids)}")
    return 0 if ok_n == len(runids) else 1


if __name__ == "__main__":
    sys.exit(main())
