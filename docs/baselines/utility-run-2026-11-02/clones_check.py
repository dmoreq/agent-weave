#!/usr/bin/env python3
"""Stage-3 utility scorer (kg clones).

Expected C1: {"pairs": [[spanA, spanB], ...]} — every seeded pair.
Expected C2: {"exists": false, "pairs": []} — near.py is not a clone.
Correctness C1: recall 4/4 AND spurious <= 20% of reported pairs (G16 bar).
Correctness C2: exists != true, pairs empty, answer text reads as a negative.
Usage: python3 clones_check.py <runid ...>; default = cells in status3.txt.
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
EXP = Path("/Users/quy.doan/Workspace/kg/docs/baselines/expected")


def norm_pair(s: str):
    toks = str(s).split()
    return tuple(sorted(toks)) if len(toks) == 2 else None


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
    report = [p for p in (norm_pair(x) for x in (ans.get("pairs") or [])) if p]
    if task == "C1":
        want = {tuple(sorted(p)) for p in exp["pairs"]}
        hit = want & set(report)
        if len(hit) != len(want):
            out["why"] = f"recall {len(hit)}/{len(want)}"
            return out
        spurious = len(report) - len(hit)
        if spurious > max(1, len(report) // 5):
            out["why"] = f"spurious {spurious}/{len(report)}"
            return out
        out["ok"] = True
        return out
    # C2
    if ans.get("exists") is True or report:
        out["why"] = "claimed a duplicate pair"
        return out
    import re as _re
    text = str(ans.get("answer", "")).lower()
    if not _re.search(r"\bno\b|\bnot\b", text):
        out["why"] = "negative answer not stated in answer text"
        return out
    out["ok"] = True
    return out


def main() -> int:
    runids = sys.argv[1:]
    if not runids:
        st = ROOT / "status3.txt"
        seen = []
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
