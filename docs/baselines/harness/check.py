#!/usr/bin/env python3
"""D0 correctness checker.

Compares per-task answer JSON against frozen expected JSON.

Shapes (docs/baselines/expected/<TASK>.json frozen truth; docs/baselines/
transcripts/<TASK>.answer.json for either arm):
  pointers          list of "path:line" strings          -> precision/recall
  ordering          optional list of stage keys          -> positional match;
                       both sides normalized to the token before " (" so prose
                       annotations cannot gate the comparison
  grouping          optional {dir: count} map (T1)      -> per-dir exact match
  scoring           optional "recall_only": expected set is a verified minimal
                       core, extras are NOT false positives; precision is
                       reported but not gating (T4/T5)
  candidate_hits    optional [pointers]: AMBIGUOUS results whose candidate
                       list contains the true callee -> count as recall hits,
                       excluded from the precision denominator (tasks.md rule)
  dynamic_excluded  optional [pointers]: dynamic dispatch (e.g. getattr) ->
                       excluded from the precision denominator per spec
Precision convention: an answer with an EMPTY precision denominator (all
candidate hits / all dynamic) reports precision 1.0 — recall still gates the
answer, so a thin answer cannot hide behind this.

Usage: check.py [TASK ...]   (default: all tasks with both files present)
"""
import json
import os
import sys

BASE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")


def load(path):
    with open(path) as f:
        return json.load(f)


def order_key(stage):
    return stage.split(" (", 1)[0].strip()


def main():
    exp_dir = os.path.join(BASE, "expected")
    tr_dir = os.path.join(BASE, "transcripts")
    tasks = sys.argv[1:]
    if not tasks:
        tasks = sorted(f[:2] for f in os.listdir(exp_dir) if f.endswith(".json"))
    print("| task | precision | recall | ordering | grouping |")
    print("|------|-----------|--------|----------|----------|")
    for t in tasks:
        exp = load(os.path.join(exp_dir, f"{t}.json"))
        ans = load(os.path.join(tr_dir, f"{t}.answer.json"))
        recall_only = exp.get("scoring") == "recall_only"
        a = {p.strip() for p in ans.get("pointers", [])}
        e = {p.strip() for p in exp.get("pointers", [])}
        cand = {p.strip() for p in ans.get("candidate_hits", [])}
        dyn = {p.strip() for p in ans.get("dynamic_excluded", [])}
        hit = (a | (cand & e)) & e
        prec_denom = len(a - dyn - cand)
        precision = (len(hit & a)) / prec_denom if prec_denom else 1.0
        recall = len(hit) / len(e) if e else 0.0
        if recall_only:
            p_txt = f"{precision:.0%} (recall-only mode: not gating)"
        else:
            p_txt = f"{precision:.2%}"
        miss = sorted(e - hit)
        extra = sorted(a - e)
        o_ans = [order_key(s) for s in ans.get("ordering", [])]
        o_exp = [order_key(s) for s in exp.get("ordering", [])]
        n = max(len(o_ans), len(o_exp))
        order = (sum(1 for i in range(min(len(o_ans), len(o_exp)))
                     if o_ans[i] == o_exp[i]) / n) if n else 1.0
        grp = ""
        if "grouping" in exp:
            ag = ans.get("grouping", {})
            eg = exp.get("grouping", {})
            ok = sum(1 for d in eg if ag.get(d) == eg[d])
            grp = f"{ok}/{len(eg)} dirs"
        print(f"| {t} | {p_txt} | {recall:.2%} | {order:.0%} | {grp or '-'} |")
        if miss:
            print(f"  {t} misses: {', '.join(miss)}")
        if extra and not recall_only:
            print(f"  {t} false positives: {', '.join(extra)}")
        elif extra:
            print(f"  {t} extras (accepted, recall-only): {', '.join(extra[:8])}"
                  + (" ..." if len(extra) > 8 else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
