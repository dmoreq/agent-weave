#!/usr/bin/env python3
"""Agent-utility A/B runner for the agy and zcode CLIs.

Reuses the frozen prompts/expected sets from the Grok/OpenCode battery.
Cells:
  path    P1-P3 x 2 arms x 2 hosts x 2 runs  (24)
  search  B1-B2 x 2 arms x 2 hosts x 1 run   ( 8)
  clones  C1-C2 x 2 arms x 2 hosts x 1 run   ( 8)
  context F1-F3 x 2 arms x 2 hosts x 1 run   (12)
Total 52 cells, run sequentially.

Usage: python3 run_hosts_2026-11-02.py [--max NEW_CELLS] [--cell HOST ARM TASK RUN]
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path("/Users/quy.doan/Workspace/kg/docs/baselines/utility-run-2026-11-02")
KG = "/Users/quy.doan/Workspace/kg/target/release/kg"
AGY = str(Path.home() / ".local/bin/agy")
ZCODE = "/opt/homebrew/bin/zcode"
ANSWERS = Path("/tmp/utility-answers-2026-11-02-hosts")
CORPORA = {
    "P1": Path("/tmp/utility-corpus-2026-11-02/kg"),
    "P2": Path("/tmp/utility-corpus-2026-11-02/kg"),
    "P3": Path("/tmp/utility-corpus-2026-11-02/vihap"),
    "B1": Path("/tmp/utility-corpus-2026-11-02/vihap"),
    "B2": Path("/tmp/utility-corpus-2026-11-02/vihap"),
    "C1": Path("/tmp/utility-corpus-2026-11-02/clones"),
    "C2": Path("/tmp/utility-corpus-2026-11-02/clones"),
    "F1": Path("/tmp/prosev-new"),
    "F2": Path("/tmp/prosev-new"),
    "F3": Path("/tmp/prosev-new"),
}
STAGE = {"P": "path", "B": "search", "C": "clones", "F": "context"}
TREATMENT_ARM = {"P": "kgp", "B": "kgs", "C": "kgcl", "F": "kgx"}
FEATURE_TOOL = {"path": "path", "search": "search", "clones": "clones", "context": "context"}
STATUS = ROOT / "status-hosts-2026-11-02.txt"
MANIFEST = ROOT / "manifest-hosts-2026-11-02.txt"
METRICS = ROOT / "metrics-hosts-2026-11-02.jsonl"
KILL_SECS = 600


def runids() -> list[tuple[str, str, str, int]]:
    cells = []
    for host in ("agy", "zcode"):
        for task in ("P1", "P2", "P3"):
            for run in (1, 2):
                cells.append((host, "kgc", task, run))
                cells.append((host, "kgp", task, run))
        for task in ("B1", "B2", "C1", "C2"):
            arm = TREATMENT_ARM[task[0]]
            for run in (1,):
                cells.append((host, "kgc", task, run))
                cells.append((host, arm, task, run))
        for task in ("F1", "F2", "F3"):
            for run in (1,):
                cells.append((host, "kgc", task, run))
                cells.append((host, "kgx", task, run))
    return cells


def kmg(root: Path) -> None:
    env = dict(os.environ)
    env.pop("KG_ARM", None)
    env.pop("KG_SESSION", None)
    subprocess.run([KG, "build", "--root", str(root)], env=env,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=120)


def journal(repo: Path, runid: str) -> dict:
    out = {"kg_calls": 0, "kg_tools": {}}
    log = ROOT / "logs" / f"{runid}.usage.log"
    if not log.exists():
        return out
    for line in log.read_text().splitlines():
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            continue
        # The agy/zcode shells do not inherit KG_ARM/KG_SESSION. The log is
        # cleared before each cell and moved after it, so every line belongs
        # to this cell; per-cell isolation replaces arm attribution.
        out["kg_calls"] += 1
        tool = rec.get("tool", "?")
        out["kg_tools"][tool] = out["kg_tools"].get(tool, 0) + 1
    return out


KG_SUBCOMMANDS = ("path", "callers", "callees", "context", "impact", "dead",
                  "unresolved", "symbols", "search", "clones", "stats",
                  "hotspots", "report", "usage", "build", "doctor")
KG_RE = re.compile(
    r"(?:target/release/)?\bkg\s+(?:--[a-z-]+\s+\S+\s+)*"
    r"(" + "|".join(KG_SUBCOMMANDS) + r")\b"
)


def transcript_commands(host: str, path: Path) -> list[str]:
    cmds: list[str] = []
    if not path.exists():
        return cmds
    for line in path.read_text(errors="replace").splitlines():
        try:
            d = json.loads(line)
        except json.JSONDecodeError:
            continue
        if host == "agy":
            if d.get("event") == "step_update":
                u = d.get("step_update") or {}
                if u.get("tool_name") == "run_command" and u.get("state") == "DONE":
                    cmd = (u.get("tool_info") or {}).get("parameters", {}).get("CommandLine")
                    if isinstance(cmd, str):
                        cmds.append(cmd)
        elif d.get("type") == "assistant":
            for part in (d.get("message") or {}).get("content") or []:
                if part.get("type") == "tool_use" and part.get("name") == "Bash":
                    cmd = (part.get("input") or {}).get("command")
                    if isinstance(cmd, str):
                        cmds.append(cmd)
    return cmds


def kg_from_transcript(host: str, path: Path) -> dict:
    out = {"kg_calls": 0, "kg_tools": {}, "direct_sqlite": 0}
    for cmd in transcript_commands(host, path):
        if "sqlite3" in cmd or ".kg/index.db" in cmd:
            out["direct_sqlite"] += 1
        for m in KG_RE.finditer(cmd):
            out["kg_calls"] += 1
            tool = m.group(1)
            out["kg_tools"][tool] = out["kg_tools"].get(tool, 0) + 1
    return out


def parse_agy(path: Path) -> dict:
    out = {"tools": 0, "model_calls": 0, "tok_in": 0, "tok_out": 0,
           "cache_read": 0, "model": None, "session": None, "event_error": None}
    if not path.exists():
        return out
    for line in path.read_text(errors="replace").splitlines():
        try:
            d = json.loads(line)
        except json.JSONDecodeError:
            continue
        ev = d.get("event")
        if ev == "init":
            out["model"] = (d.get("init") or {}).get("model")
            out["session"] = d.get("conversation_id")
        elif ev == "step_update":
            u = d.get("step_update") or {}
            if u.get("step_type") == "tool" and u.get("state") == "DONE":
                out["tools"] += 1
            if u.get("usage"):
                out["model_calls"] += 1
        elif ev == "result":
            r = d.get("result") or {}
            out["session"] = r.get("conversation_id") or out["session"]
            use = r.get("usage") or {}
            out["tok_in"] = use.get("input_tokens", 0)
            out["tok_out"] = use.get("output_tokens", 0)
            out["cache_read"] = use.get("cache_read_tokens", 0)
            if r.get("status") != "SUCCESS":
                out["event_error"] = r.get("status")
    return out


def parse_zcode(path: Path) -> dict:
    out = {"tools": 0, "model_calls": 0, "tok_in": 0, "tok_out": 0,
           "cache_read": 0, "model": None, "session": None, "event_error": None}
    if not path.exists():
        return out
    for line in path.read_text(errors="replace").splitlines():
        try:
            d = json.loads(line)
        except json.JSONDecodeError:
            continue
        t = d.get("type")
        if t == "system" and d.get("subtype") == "init":
            out["model"] = d.get("model")
            out["session"] = d.get("session_id")
        elif t == "assistant":
            for part in (d.get("message") or {}).get("content") or []:
                if part.get("type") == "tool_use":
                    out["tools"] += 1
        elif t == "result":
            out["session"] = d.get("session_id") or out["session"]
            use = d.get("usage") or {}
            out["tok_in"] = use.get("input_tokens", 0)
            out["tok_out"] = use.get("output_tokens", 0)
            out["cache_read"] = use.get("cache_read_input_tokens", 0)
            out["model_calls"] = d.get("num_turns", 0)
            if d.get("is_error"):
                out["event_error"] = d.get("result")
    return out


def run_cell(host: str, arm: str, task: str, run: int) -> None:
    stage = STAGE[task[0]]
    runid = f"{host}-{arm}-r{run}-{task}"
    if f"[done] {runid} " in STATUS.read_text() if STATUS.exists() else False:
        print(f"[skip] {runid}", flush=True)
        return
    repo = CORPORA[task]
    kmg(repo)
    for f in repo.joinpath(".kg").glob("usage.log*"):
        f.unlink(missing_ok=True)
    for f in repo.joinpath(".kg").glob("usage-results.jsonl*"):
        f.unlink(missing_ok=True)
    ANSWERS.mkdir(parents=True, exist_ok=True)
    answer = ANSWERS / f"{runid}.json"
    answer.unlink(missing_ok=True)
    prompt = (ROOT / "prompts" / f"{arm}-{task}.txt").read_text().replace(
        "{ANSWER_PATH}", str(answer)
    )
    prompt += (
        "\nAdditional isolation rules for this run (2026-11-02):\n"
        "- Do not read or list any path outside the repository, except the exact\n"
        "  answer file path above when writing it.\n"
        "- Do not read other answer files, previous-run artifacts, or\n"
        "  .kg/index.db directly. Use only the kg CLI and repository files.\n"
    )
    log = ROOT / "logs" / f"{runid}.jsonl"
    err = ROOT / "logs" / f"{runid}.err"
    log.parent.mkdir(parents=True, exist_ok=True)
    if host == "agy":
        cmd = [AGY, "--output-format", "stream-json",
               "--dangerously-skip-permissions", "--model", "gemini-3.8-flash-high",
               "--print-timeout", f"{KILL_SECS}s", f"-p={prompt}"]
    else:
        cmd = [ZCODE, "--permission-mode", "yolo", "--output-format", "stream-json",
               "--verbose", "-p", prompt]
    env = dict(os.environ)
    env["KG_SESSION"] = runid
    env["KG_ARM"] = runid
    env["PATH"] = "/Users/quy.doan/Workspace/kg/target/release:" + env.get("PATH", "")
    with STATUS.open("a") as st:
        st.write(f"[start] {runid} host={host} repo={repo} {time.strftime('%H:%M:%S')}\n")
    started = time.time()
    rc = -1
    with log.open("wb") as out, err.open("wb") as eout:
        proc = subprocess.Popen(cmd, cwd=repo, env=env, stdout=out, stderr=eout,
                                start_new_session=True)
        try:
            rc = proc.wait(timeout=KILL_SECS)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            rc = -9
    duration = round(time.time() - started, 1)
    if (repo / ".kg/usage.log").exists():
        shutil.move(str(repo / ".kg/usage.log"), ROOT / "logs" / f"{runid}.usage.log")
    if (repo / ".kg/usage-results.jsonl").exists():
        shutil.move(str(repo / ".kg/usage-results.jsonl"),
                    ROOT / "logs" / f"{runid}.usage-results.jsonl")
    if answer.exists():
        shutil.copy(answer, ROOT / "answers" / answer.name)
    parsed = parse_agy(log) if host == "agy" else parse_zcode(log)
    jr = journal(repo, runid)
    tr = kg_from_transcript(host, log)
    kg_tools = dict(jr["kg_tools"])
    for k, v in tr["kg_tools"].items():
        kg_tools[k] = max(kg_tools.get(k, 0), v)
    rec = {"runid": runid, "host": host, "arm": arm, "task": task, "stage": stage,
           "exit": rc, "duration_s": duration, **parsed,
           "journal_calls": jr["kg_calls"], "transcript_kg_calls": tr["kg_calls"],
           "kg_calls": max(jr["kg_calls"], tr["kg_calls"]), "kg_tools": kg_tools,
           "feature_calls": kg_tools.get(FEATURE_TOOL[stage], 0),
           "direct_sqlite": tr["direct_sqlite"], "answered": answer.exists()}
    rec["violations"] = []
    if arm == "kgc" and rec["feature_calls"] > 0:
        rec["violations"].append("control arm called the feature")
    if rec["tools"] == 0:
        rec["violations"].append("zero tool calls")
    if rec["direct_sqlite"] > 0:
        rec["violations"].append("direct sqlite/index.db query")
    if rec["tools"] > 25:
        rec["violations"].append(f"tool budget exceeded ({rec['tools']} > 25)")
    with METRICS.open("a") as mf:
        mf.write(json.dumps(rec) + "\n")
    with MANIFEST.open("a") as mf:
        mf.write(f"{runid}|{host}|{arm}|{task}|{parsed.get('session') or ''}\n")
    with STATUS.open("a") as st:
        st.write(f"[done] {runid} exit={rc} {time.strftime('%H:%M:%S')} "
                 f"tools={rec['tools']} kg={rec['kg_calls']} dur={duration}s\n")
    print(f"[done] {runid} exit={rc} tools={rec['tools']} kg={rec['kg_calls']} "
          f"dur={duration}s", flush=True)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--max", type=int, default=999)
    ap.add_argument("--cell", nargs=4, metavar=("HOST", "ARM", "TASK", "RUN"))
    args = ap.parse_args()
    if args.cell:
        host, arm, task, run = args.cell
        run_cell(host, arm, task, int(run))
        return 0
    done = 0
    for host, arm, task, run in runids():
        if done >= args.max:
            break
        runid = f"{host}-{arm}-r{run}-{task}"
        if STATUS.exists() and f"[done] {runid} " in STATUS.read_text():
            continue
        done += 1
        print(f"=== cell {done}/{args.max}: {runid} ===", flush=True)
        try:
            run_cell(host, arm, task, run)
        except Exception as exc:  # one bad cell must not stop the battery
            with STATUS.open("a") as st:
                st.write(f"[error] {runid} {exc!r}\n")
            print(f"[error] {runid} {exc!r}", flush=True)
    print(f"=== runner done: {done} new cells ===", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
