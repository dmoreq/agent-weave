#!/usr/bin/env python3
"""Aggregate P9 utility metrics, scores, and the frozen decision rule."""
from __future__ import annotations

import argparse
import json
import statistics
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
METRICS = HERE / "metrics-p9.jsonl"
TASKS = ("I1", "T1", "H1", "D1", "A2")
HOSTS = ("grok", "agy", "zcode", "claude")
VALID_HOSTS = ("grok", "agy", "zcode", "claude")


def mean(values: list[float]) -> float:
    return statistics.fmean(values) if values else 0.0


def pct(new: float, old: float) -> str:
    if old == 0:
        return "n/a"
    return f"{(new - old) / old * 100:+.0f}%"


def scores(answers: Path) -> dict[str, bool]:
    out = subprocess.run(
        [sys.executable, "-I", str(HERE / "score_p9.py"),
         "--answers", str(answers)],
        capture_output=True, text=True, check=False,
    )
    verdicts: dict[str, bool] = {}
    for line in out.stdout.splitlines():
        if not line.startswith("| "):
            continue
        fields = [field.strip() for field in line.strip().strip("|").split("|")]
        if len(fields) >= 3 and fields[0] not in {"runid", ""}:
            verdicts[fields[0]] = fields[2] == "OK"
    if out.returncode not in (0, 1):
        print(out.stderr, file=sys.stderr)
    return verdicts


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--metrics", type=Path, default=METRICS)
    ap.add_argument("--answers", type=Path, default=HERE / "answers-p9")
    ap.add_argument("--hosts", default=",".join(HOSTS))
    ap.add_argument("--valid-hosts", default=",".join(VALID_HOSTS))
    ap.add_argument("--no-rule", action="store_true")
    args = ap.parse_args()
    hosts = tuple(host.strip() for host in args.hosts.split(",") if host.strip())
    valid_hosts = tuple(host.strip() for host in args.valid_hosts.split(",")
                        if host.strip())
    if not args.metrics.exists():
        print("no metrics yet", file=sys.stderr)
        return 1
    rows = [json.loads(line) for line in args.metrics.read_text().splitlines()
            if line.strip()]
    verdicts = scores(args.answers)
    print("# P9 agent-utility battery (2026-11-03)\n")
    print("| host | task | arm | n | correct | tools | aw calls | feature calls | "
          "input tok | cache tok | output tok | seconds | aw p50 ms | aw p95 ms | "
          "violations |")
    print("|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    grouped: dict[tuple[str, str, str], list[dict]] = defaultdict(list)
    for row in rows:
        grouped[(row["host"], row["task"], row["arm"])].append(row)
    for host in hosts:
        for task in TASKS:
            for arm in ("kgc", "kgp9"):
                cells = grouped.get((host, task, arm), [])
                if not cells:
                    continue
                ok = sum(1 for cell in cells if verdicts.get(cell["runid"]))
                violations = sum(1 for cell in cells if cell["violations"])
                p50 = [cell["aw_latency_p50_us"] for cell in cells
                       if cell["aw_latency_p50_us"] is not None]
                p95 = [cell["aw_latency_p95_us"] for cell in cells
                       if cell["aw_latency_p95_us"] is not None]
                print(
                    f"| {host} | {task} | {arm} | {len(cells)} | {ok}/{len(cells)} | "
                    f"{mean([c['tools'] for c in cells]):.1f} | "
                    f"{mean([c['aw_calls'] for c in cells]):.1f} | "
                    f"{mean([c['feature_calls'] for c in cells]):.1f} | "
                    f"{mean([c['tok_in'] for c in cells]):.0f} | "
                    f"{mean([c['cache_read'] for c in cells]):.0f} | "
                    f"{mean([c['tok_out'] for c in cells]):.0f} | "
                    f"{mean([c['duration_s'] for c in cells]):.0f} | "
                    f"{mean(p50) / 1000:.2f} | {mean(p95) / 1000:.2f} | {violations} |"
                )

    print("\n## Control vs treatment deltas\n")
    print("| host | task | metric | control | treatment | delta |")
    print("|---|---|---|---:|---:|---:|")
    for host in valid_hosts:
        for task in TASKS:
            control = grouped.get((host, task, "kgc"), [])
            treatment = grouped.get((host, task, "kgp9"), [])
            if not control or not treatment:
                continue
            for metric in ("tools", "aw_calls", "tok_in", "tok_out",
                           "duration_s"):
                old = mean([row[metric] for row in control])
                new = mean([row[metric] for row in treatment])
                print(f"| {host} | {task} | {metric} | {old:.1f} | {new:.1f} | "
                      f"{pct(new, old)} |")

    if not args.no_rule:
        print("\n## Frozen decision rule (valid hosts: "
              f"{', '.join(valid_hosts)})\n")
        print("| task | host | control correct | treatment correct | tool delta | "
              "verdict |")
        print("|---|---|---:|---:|---:|---|")
        task_host_conditions: dict[str, list[bool]] = defaultdict(list)
        for task in TASKS:
            for host in valid_hosts:
                control = [row for row in grouped.get((host, task, "kgc"), [])
                           if not any("control arm called" in v
                                      for v in row["violations"])]
                treatment = grouped.get((host, task, "kgp9"), [])
                if not control or not treatment:
                    continue
                control_ok = sum(1 for row in control if verdicts.get(row["runid"]))
                treatment_ok = sum(1 for row in treatment
                                   if verdicts.get(row["runid"]))
                parity = treatment_ok >= control_ok
                old_tools = mean([row["tools"] for row in control])
                new_tools = mean([row["tools"] for row in treatment])
                baseline_ok = old_tools >= 3.0
                economy = baseline_ok and new_tools <= old_tools * 0.8
                correctness_win = (control_ok <= len(control) / 2
                                    and treatment_ok >= len(treatment) * 0.75)
                keep = parity and (economy or correctness_win)
                task_host_conditions[task].append(keep)
                label = "KEEP" if keep else (
                    "NO (compressed)" if not baseline_ok else "NO")
                print(f"| {task} | {host} | {control_ok}/{len(control)} | "
                      f"{treatment_ok}/{len(treatment)} | {pct(new_tools, old_tools)} | "
                      f"{label} |")

        print("\n## Task verdicts\n")
        print("| task | qualifying hosts | verdict |")
        print("|---|---:|---|")
        for task in TASKS:
            conditions = task_host_conditions.get(task, [])
            qualifying = sum(conditions)
            if len(conditions) < len(valid_hosts):
                verdict = "INCOMPLETE"
            elif qualifying >= len(valid_hosts):
                verdict = "KEEP"
            elif qualifying >= 1:
                verdict = "PARTIAL"
            else:
                verdict = "NO VALUE"
            print(f"| {task} | {qualifying}/{len(valid_hosts)} | {verdict} |")

    print("\n## Violations\n")
    for row in rows:
        if row["violations"]:
            print(f"- `{row['runid']}`: {', '.join(row['violations'])}")
    print("\n## Correctness failures\n")
    for row in rows:
        if not verdicts.get(row["runid"]):
            print(f"- `{row['runid']}` ({row['task']}, {row['arm']})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
