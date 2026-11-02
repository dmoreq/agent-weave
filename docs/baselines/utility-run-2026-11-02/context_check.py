#!/usr/bin/env python3
"""Stage-5 scorer (kg context): exact qualname + caller/callee set agreement."""
import json, os, re, sys
from pathlib import Path
ROOT = Path(__file__).resolve().parent
SHARED_EXP = Path("/Users/quy.doan/Workspace/kg/docs/baselines/expected")
EXP = Path(os.environ.get("KG_EXPECTED_DIR", SHARED_EXP))

def score(runid):
    parts = runid.split("-"); arm, task = parts[1], parts[3]
    exp_path = EXP / f"{task}.json"
    if not exp_path.exists():
        exp_path = SHARED_EXP / f"{task}.json"
    exp = json.loads(exp_path.read_text())
    f = ROOT / "answers" / f"{runid}.json"
    out = {"runid": runid, "ok": False, "why": ""}
    if not f.exists():
        out["why"] = "no answer file"; return out
    try: ans = json.loads(f.read_text())
    except json.JSONDecodeError as e:
        out["why"] = f"invalid JSON: {e}"; return out
    if ans.get("qualname") != exp["qualname"]:
        out["why"] = f"qualname {ans.get('qualname')} != {exp['qualname']}"; return out
    for key in ("callers", "callees"):
        got = set(ans.get(key) or [])
        want = set(exp[key])
        if got != want:
            out["why"] = f"{key} mismatch: missing={sorted(want-got)[:3]} extra={sorted(got-want)[:3]}"
            return out
    out["ok"] = True; return out

def main():
    runids = sys.argv[1:]
    if not runids:
        st = ROOT / "status5.txt"
        runids = sorted({m.group(1) for m in re.finditer(r"\[start\] (\S+) ", st.read_text())}) if st.exists() else []
    print("| runid | ok | why |"); print("|-------|----|-----|")
    ok = 0
    for r in runids:
        s = score(r); ok += s["ok"]
        print(f"| {s['runid']} | {'OK' if s['ok'] else 'FAIL'} | {s['why']} |")
    print(f"\ncorrect cells: {ok}/{len(runids)}")
    return 0 if ok == len(runids) else 1

if __name__ == "__main__": sys.exit(main())
