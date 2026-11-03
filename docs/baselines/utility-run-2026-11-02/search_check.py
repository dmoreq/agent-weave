#!/usr/bin/env python3
"""Stage-2 utility scorer (kg search).

Expected shape (expected/S*.json): {"exists": true, "hops": 0,
"route": ["<path>:<line>"]} — the single definition pointer.
Correctness = exists=true AND the expected pointer appears in the answer's
route verbatim AND hops == 0.
Usage: python3 search_check.py <runid ...>; default = cells in status2.txt.
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
EXP = Path("/Users/quy.doan/Workspace/kg/docs/baselines/expected")


def score(runid: str) -> dict:
    parts = runid.split("-")
    arm, task = parts[1], parts[3]
    exp = json.loads((EXP / f"{task}.json").read_text())
    out = {"runid": runid, "arm": arm, "task": task, "ok": False, "why": ""}
    f = ROOT / "answers" / f"{runid}.json"
    if not f.exists():
        out["why"] = "no answer file"
        return out
    try:
        ans = json.loads(f.read_text())
    except json.JSONDecodeError as e:
        out["why"] = f"invalid JSON: {e}"
        return out
    if ans.get("exists") is not True:
        out["why"] = "exists != true"
        return out
    if ans.get("hops") != 0:
        out["why"] = f"hops {ans.get('hops')!r} != 0"
        return out
    route = ans.get("route") or []
    if exp["route"][0] not in route:
        out["why"] = f"expected {exp['route'][0]} not in route {route}"
        return out
    qualname = exp.get("qualname")
    if qualname and qualname not in str(ans.get("answer", "")):
        out["why"] = f"qualname {qualname} missing from answer text"
        return out
    out["ok"] = True
    return out


def main() -> int:
    runids = sys.argv[1:]
    if not runids:
        seen = []
        st = ROOT / "status2.txt"
        if st.exists():
            for line in st.read_text().splitlines():
                m = re.match(r"\[start\] (\S+) ", line)
                if m:
                    seen.append(m.group(1))
        runids = seen
    print("| runid | ok | why |")
    print("|-------|----|-----|")
    ok = 0
    for r in runids:
        s = score(r)
        ok += s["ok"]
        print(f"| {s['runid']} | {'OK' if s['ok'] else 'FAIL'} | {s['why']} |")
    print(f"\ncorrect cells: {ok}/{len(runids)}")
    return 0 if ok == len(runids) else 1


if __name__ == "__main__":
    raise SystemExit(main())
