#!/usr/bin/env python3
"""Aggregate the agy/zcode A/B metrics and stage correctness into a markdown report."""
from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path("/Users/quy.doan/Workspace/kg/docs/baselines/utility-run-2026-11-02")
METRICS = ROOT / "metrics-hosts-2026-11-02.jsonl"
SCORERS = {
    "path": ROOT / "path_check.py",
    "search": ROOT / "search_check.py",
    "clones": ROOT / "clones_check.py",
    "context": ROOT / "context_check.py",
}


def score_stage(stage: str, runids: list[str]) -> dict[str, bool]:
    env = dict(os.environ)
    if stage == "context":
        env["KG_EXPECTED_DIR"] = str(
            Path("/Users/quy.doan/Workspace/kg/docs/baselines/expected-hosts-2026-11-02"))
    out = subprocess.run([sys.executable, "-I", str(SCORERS[stage]), *runids],
                         capture_output=True, text=True, env=env)
    verdicts: dict[str, bool] = {}
    for line in out.stdout.splitlines():
        if not line.startswith("|"):
            continue
        fields = [f.strip() for f in line.strip().strip("|").split("|")]
        if len(fields) < 2 or not fields[0].startswith(("agy-", "zcode-")):
            continue
        verdicts[fields[0]] = "OK" in fields[1:]
    if out.returncode not in (0, 1):
        print(out.stderr, file=sys.stderr)
    return verdicts


def mean(xs: list[float]) -> float:
    return sum(xs) / len(xs) if xs else 0.0


def pct(new: float, old: float) -> str:
    if old == 0:
        return "n/a"
    return f"{(new - old) / old * 100:+.0f}%"


def main() -> int:
    rows = [json.loads(l) for l in METRICS.read_text().splitlines() if l.strip()]
    by_stage: dict[str, list[dict]] = defaultdict(list)
    for r in rows:
        by_stage[r["stage"]].append(r)
    verdicts: dict[str, bool] = {}
    for stage, srows in by_stage.items():
        verdicts.update(score_stage(stage, [r["runid"] for r in srows]))

    print("# agy / zcode agent-utility battery (2026-11-02)\n")
    print("| host | stage | arm | n | correct | tools | kg calls | feature calls | "
          "input tok | cache-read tok | out tok | seconds | violations |")
    print("|---|---|---|---|---|---|---|---|---|---|---|---|")
    agg: dict[tuple[str, str, str], list[dict]] = defaultdict(list)
    for r in rows:
        agg[(r["host"], r["stage"], r["arm"])].append(r)
    order = {"path": 0, "search": 1, "clones": 2, "context": 3}
    for host in ("agy", "zcode"):
        for stage in sorted({r["stage"] for r in rows}, key=lambda s: order[s]):
            for arm in ("kgc", "kgp", "kgs", "kgcl", "kgx"):
                cells = [r for r in rows if r["host"] == host and r["stage"] == stage
                         and r["arm"] == arm]
                if not cells:
                    continue
                ok = sum(1 for r in cells if verdicts.get(r["runid"]))
                viol = sum(1 for r in cells if r["violations"])
                print(f"| {host} | {stage} | {arm} | {len(cells)} | {ok}/{len(cells)} | "
                      f"{mean([r['tools'] for r in cells]):.1f} | "
                      f"{mean([r['kg_calls'] for r in cells]):.1f} | "
                      f"{mean([r['feature_calls'] for r in cells]):.1f} | "
                      f"{mean([r['tok_in'] for r in cells]):.0f} | "
                      f"{mean([r['cache_read'] for r in cells]):.0f} | "
                      f"{mean([r['tok_out'] for r in cells]):.0f} | "
                      f"{mean([r['duration_s'] for r in cells]):.0f} | {viol} |")
    print("\n## Control vs treatment deltas\n")
    print("| host | stage | metric | control | treatment | delta |")
    print("|---|---|---|---|---|---|")
    for host in ("agy", "zcode"):
        for stage in sorted({r["stage"] for r in rows}, key=lambda s: order[s]):
            ctl = [r for r in rows if r["host"] == host and r["stage"] == stage
                   and r["arm"] == "kgc"]
            trt = [r for r in rows if r["host"] == host and r["stage"] == stage
                   and r["arm"] != "kgc"]
            if not ctl or not trt:
                continue
            for metric in ("tools", "kg_calls", "tok_in", "cache_read", "tok_out", "duration_s"):
                a, b = mean([r[metric] for r in ctl]), mean([r[metric] for r in trt])
                print(f"| {host} | {stage} | {metric} | {a:.1f} | {b:.1f} | {pct(b, a)} |")
    print("\n## Violations\n")
    for r in rows:
        if r["violations"]:
            print(f"- `{r['runid']}`: {', '.join(r['violations'])}")
    bad = [r for r in rows if not verdicts.get(r["runid"])]
    print("\n## Correctness failures\n")
    for r in bad:
        print(f"- `{r['runid']}` ({r['stage']}, {r['arm']})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
