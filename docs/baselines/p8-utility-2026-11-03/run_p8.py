#!/usr/bin/env python3
"""Four-host agent-utility A/B runner for the P8 surfaces.

Cells: 5 tasks x 2 arms x 4 hosts x 2 runs = 80.
Usage:
  python3 -I run_p8.py                # run all pending cells
  python3 -I run_p8.py --smoke        # one task pair per host (8 cells)
  python3 -I run_p8.py --max N
  python3 -I run_p8.py --cell HOST ARM TASK RUN
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import signal
import sqlite3
import statistics
import subprocess
import sys
import time
import uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent
KG = Path("/Users/quy.doan/Workspace/kg/target/release/kg")
GROK = Path.home() / ".grok/bin/grok"
OPENCODE = Path.home() / ".opencode/bin/opencode"
AGY = Path.home() / ".local/bin/agy"
ZCODE = Path("/opt/homebrew/bin/zcode")
ANSWERS = Path("/tmp/p8-utility-answers-2026-11-03")
CORPORA = {
    "I1": Path("/tmp/p8-utility-corpus-2026-11-03/kg"),
    "T1": Path("/tmp/p8-utility-corpus-2026-11-03/vihap"),
    "H1": Path("/tmp/p8-utility-corpus-2026-11-03/vihap"),
    "D1": Path("/tmp/p8-utility-corpus-2026-11-03/vihap"),
    "A1": Path("/tmp/p8-utility-corpus-2026-11-03/finguard"),
}
FEATURE_TOOL = {
    "I1": "imports",
    "T1": "tests",
    "H1": "implementers",
    "D1": "impact",
    "A1": "api",
}
TASKS = ("I1", "T1", "H1", "D1", "A1")
ARMS = ("kgc", "kgp8")
HOST_TAGS = {"grok": "grok", "opencode": "oc", "agy": "agy", "zcode": "zcode"}
KG_SUBCOMMANDS = (
    "build", "callers", "callees", "path", "context", "implementers",
    "imports", "tests", "impact", "dead", "unresolved", "symbols",
    "complexity", "api", "resolve", "files", "search", "clones", "stats",
    "hotspots", "report", "doctor", "usage", "mcp",
)
KG_RE = re.compile(
    r"(?:target/release/)?\bkg\s+(?:--[a-z-]+\s+\S+\s+)*("
    + "|".join(KG_SUBCOMMANDS) + r")\b"
)
STATUS = HERE / "status.txt"
MANIFEST = HERE / "manifest.txt"
METRICS = HERE / "metrics-p8.jsonl"
KILL_SECS = 600
OC_DB = Path.home() / ".local/share/opencode/opencode.db"


def runids(smoke: bool = False) -> list[tuple[str, str, str, int]]:
    cells: list[tuple[str, str, str, int]] = []
    for host in ("grok", "opencode", "agy", "zcode"):
        tasks = ("I1",) if smoke else TASKS
        runs = (0,) if smoke else (1, 2)
        for task in tasks:
            for run in runs:
                # ABBA: r1 control/treatment, r2 treatment/control.
                if run == 1:
                    cells.extend([(host, "kgc", task, run),
                                  (host, "kgp8", task, run)])
                else:
                    cells.extend([(host, "kgp8", task, run),
                                  (host, "kgc", task, run)])
    return cells


def kg_build(repo: Path) -> None:
    env = dict(os.environ)
    env.pop("KG_ARM", None)
    env.pop("KG_SESSION", None)
    subprocess.run(
        [str(KG), "build", "--force", "--root", str(repo)],
        env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        timeout=300, check=False,
    )


def journal(runid: str) -> dict:
    out = {"kg_calls": 0, "kg_tools": {}, "latencies_us": []}
    log = HERE / "logs" / f"{runid}.usage.log"
    if not log.exists():
        return out
    for line in log.read_text(errors="replace").splitlines():
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            continue
        # Per-cell file isolation replaces env attribution: the journal was
        # wiped before the cell and moved after it.
        out["kg_calls"] += 1
        tool = rec.get("tool", "?")
        out["kg_tools"][tool] = out["kg_tools"].get(tool, 0) + 1
        latency = rec.get("latency_us")
        if isinstance(latency, (int, float)):
            out["latencies_us"].append(float(latency))
    return out


def latency_stats(latencies: list[float]) -> tuple[float | None, float | None]:
    if not latencies:
        return None, None
    ordered = sorted(latencies)

    def pct(p: float) -> float:
        idx = min(len(ordered) - 1, max(0, round(p * (len(ordered) - 1))))
        return ordered[idx]

    return statistics.median(ordered), pct(0.95)


def transcript_commands(host: str, path: Path, sid: str | None = None) -> list[str]:
    cmds: list[str] = []
    if not path.exists():
        return cmds
    if host in {"agy", "zcode"}:
        for line in path.read_text(errors="replace").splitlines():
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                continue
            if host == "agy" and rec.get("event") == "step_update":
                step = rec.get("step_update") or {}
                if (step.get("tool_name") == "run_command"
                        and step.get("state") == "DONE"):
                    cmd = ((step.get("tool_info") or {}).get("parameters") or {}).get(
                        "CommandLine")
                    if isinstance(cmd, str):
                        cmds.append(cmd)
            elif host == "zcode" and rec.get("type") == "assistant":
                for part in (rec.get("message") or {}).get("content") or []:
                    if (part.get("type") == "tool_use"
                            and part.get("name") == "Bash"):
                        cmd = (part.get("input") or {}).get("command")
                        if isinstance(cmd, str):
                            cmds.append(cmd)
        return cmds
    # grok/opencode: scan recognized JSON command fields in the captured output
    # and persisted histories without treating prose mentions as commands.
    sources = [path.read_text(errors="replace")]
    if host == "grok" and sid:
        for sid_path in Path.home().glob(f".grok/sessions/*/{sid}/chat_history.jsonl"):
            sources.append(sid_path.read_text(errors="replace"))
    for source in sources:
        for match in re.finditer(r'"command"\s*:\s*"((?:[^"\\]|\\.)*)"', source):
            try:
                cmds.append(json.loads(f'"{match.group(1)}"'))
            except json.JSONDecodeError:
                cmds.append(match.group(1))
    return cmds


def kg_from_transcript(host: str, path: Path, sid: str | None = None) -> dict:
    out = {"kg_calls": 0, "kg_tools": {}, "direct_sqlite": 0}
    for cmd in transcript_commands(host, path, sid):
        if "sqlite3 " in cmd or ".kg/index.db" in cmd:
            out["direct_sqlite"] += 1
        for match in KG_RE.finditer(cmd):
            out["kg_calls"] += 1
            tool = match.group(1)
            out["kg_tools"][tool] = out["kg_tools"].get(tool, 0) + 1
    return out


def parse_agy(path: Path) -> dict:
    out = {"tools": 0, "model_calls": 0, "tok_in": 0, "tok_out": 0,
           "cache_read": 0, "model": None, "session": None, "event_error": None}
    if not path.exists():
        return out
    for line in path.read_text(errors="replace").splitlines():
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            continue
        event = rec.get("event")
        if event == "init":
            out["model"] = (rec.get("init") or {}).get("model")
            out["session"] = rec.get("conversation_id")
        elif event == "step_update":
            step = rec.get("step_update") or {}
            if step.get("step_type") == "tool" and step.get("state") == "DONE":
                out["tools"] += 1
            if step.get("usage"):
                out["model_calls"] += 1
        elif event == "result":
            result = rec.get("result") or {}
            out["session"] = result.get("conversation_id") or out["session"]
            usage = result.get("usage") or {}
            out["tok_in"] = usage.get("input_tokens", 0)
            out["tok_out"] = usage.get("output_tokens", 0)
            out["cache_read"] = usage.get("cache_read_tokens", 0)
            if result.get("status") != "SUCCESS":
                out["event_error"] = result.get("status")
    return out


def parse_zcode(path: Path) -> dict:
    out = {"tools": 0, "model_calls": 0, "tok_in": 0, "tok_out": 0,
           "cache_read": 0, "model": None, "session": None, "event_error": None}
    if not path.exists():
        return out
    for line in path.read_text(errors="replace").splitlines():
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            continue
        kind = rec.get("type")
        if kind == "system" and rec.get("subtype") == "init":
            out["model"] = rec.get("model")
            out["session"] = rec.get("session_id")
        elif kind == "assistant":
            for part in (rec.get("message") or {}).get("content") or []:
                if part.get("type") == "tool_use":
                    out["tools"] += 1
        elif kind == "result":
            out["session"] = rec.get("session_id") or out["session"]
            usage = rec.get("usage") or {}
            out["tok_in"] = usage.get("input_tokens", 0)
            out["tok_out"] = usage.get("output_tokens", 0)
            out["cache_read"] = usage.get("cache_read_input_tokens", 0)
            out["model_calls"] = rec.get("num_turns", 0)
            if rec.get("is_error"):
                out["event_error"] = rec.get("result")
    return out


def grok_parse(sid: str) -> dict:
    out = {"tools": 0, "model_calls": 0, "tok_in": 0, "tok_out": 0,
           "cache_read": 0, "model": None, "session": sid, "event_error": None}
    for path in Path.home().glob(f".grok/sessions/*/{sid}"):
        usage_path = path / "usage.json"
        if usage_path.exists():
            usage = (json.loads(usage_path.read_text()).get("session") or {})
            out["tok_in"] = usage.get("inputTokens", 0)
            out["tok_out"] = usage.get("outputTokens", 0)
            out["model"] = usage.get("primaryModelId")
            out["model_calls"] = usage.get("modelCalls", 0)
        history = path / "chat_history.jsonl"
        if history.exists():
            for line in history.read_text(errors="replace").splitlines():
                try:
                    rec = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if rec.get("type") == "tool_result":
                    out["tools"] += 1
    return out


def opencode_parse(sid: str) -> dict:
    out = {"tools": 0, "model_calls": 0, "tok_in": 0, "tok_out": 0,
           "cache_read": 0, "model": None, "session": sid, "event_error": None}
    if not OC_DB.exists():
        return out
    conn = sqlite3.connect(f"file:{OC_DB}?mode=ro", uri=True)
    try:
        rows = conn.execute(
            "SELECT data FROM session_message WHERE session_id = ? "
            "AND type = 'assistant'", (sid,),
        ).fetchall()
    finally:
        conn.close()
    for (data,) in rows:
        try:
            rec = json.loads(data)
        except json.JSONDecodeError:
            continue
        out["model_calls"] += 1
        tokens = rec.get("tokens") or {}
        out["tok_in"] += tokens.get("input", 0)
        out["tok_out"] += tokens.get("output", 0)
        model = rec.get("model") or {}
        out["model"] = out["model"] or model.get("id")
        for part in rec.get("content") or []:
            if part.get("type") == "tool":
                out["tools"] += 1
    return out


def run_cell(host: str, arm: str, task: str, run: int) -> None:
    hosttag = HOST_TAGS[host]
    runid = f"{hosttag}-{arm}-r{run}-{task}"
    if STATUS.exists() and f"[done] {runid} " in STATUS.read_text():
        print(f"[skip] {runid}", flush=True)
        return
    repo = CORPORA[task]
    kg_build(repo)
    for pattern in ("usage.log*", "usage-results.jsonl*"):
        for path in (repo / ".kg").glob(pattern):
            path.unlink(missing_ok=True)
    ANSWERS.mkdir(parents=True, exist_ok=True)
    answer = ANSWERS / f"{runid}.json"
    answer.unlink(missing_ok=True)
    prompt_path = HERE / "logs" / f"{runid}.prompt.txt"
    prompt_path.write_text(
        (HERE / "prompts" / f"{arm}-{task}.txt").read_text().replace(
            "{ANSWER_PATH}", str(answer)
        )
        + "\nAdditional isolation rules for this run (2026-11-03):\n"
          "- Do not read or list any path outside the repository, except the\n"
          "  exact answer file path above when writing it.\n"
          "- Do not read other answer files, previous-run artifacts, or\n"
          "  .kg/index.db directly. Use only the kg CLI and repository files.\n"
          "- Use at most 25 tool calls. Stop immediately after writing the answer.\n",
        encoding="utf-8",
    )
    log = HERE / "logs" / f"{runid}.jsonl"
    err = HERE / "logs" / f"{runid}.err"
    if host == "grok":
        sid = str(uuid.uuid4()).lower()
        cmd = [str(GROK), "--always-approve", "--max-turns", "40",
               "--session-id", sid, "--prompt-file", str(prompt_path)]
    elif host == "opencode":
        sid = f"ses_p8_{runid.replace('-', '_')}_{uuid.uuid4().hex[:8]}"
        cmd = [str(OPENCODE), "run", "--standalone", "--auto",
               "-m", "openai/gpt-6-sol", "--format", "json",
               "--session", sid, prompt_path.read_text()]
    elif host == "agy":
        sid = None
        cmd = [str(AGY), "--output-format", "stream-json",
               "--dangerously-skip-permissions",
               "--model", "gemini-3.8-flash-high",
               "--print-timeout", f"{KILL_SECS}s",
               f"-p={prompt_path.read_text()}"]
    else:
        sid = None
        cmd = [str(ZCODE), "--permission-mode", "yolo",
               "--output-format", "stream-json", "--verbose", "-p",
               prompt_path.read_text()]
    manifest_row = f"{runid}|{host}|{arm}|{task}|{sid or ''}"
    with STATUS.open("a") as status:
        status.write(f"[start] {runid} host={host} repo={repo} "
                     f"{time.strftime('%H:%M:%S')}\n")
    env = dict(os.environ)
    env["KG_SESSION"] = runid
    env["KG_ARM"] = runid
    env["PATH"] = str(KG.parent) + os.pathsep + env.get("PATH", "")
    started = time.time()
    rc = -1
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("wb") as out, err.open("wb") as err_out:
        proc = subprocess.Popen(cmd, cwd=repo, env=env, stdout=out,
                                stderr=err_out, start_new_session=True)
        try:
            rc = proc.wait(timeout=KILL_SECS)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            rc = -9
    duration = round(time.time() - started, 1)
    for name in ("usage.log", "usage-results.jsonl"):
        source = repo / ".kg" / name
        if source.exists():
            shutil.move(str(source), str(HERE / "logs" / f"{runid}.{name}"))
    if answer.exists():
        shutil.copy(answer, HERE / "answers" / answer.name)

    if host == "grok":
        parsed = grok_parse(sid)
    elif host == "opencode":
        parsed = opencode_parse(sid)
    elif host == "agy":
        parsed = parse_agy(log)
    else:
        parsed = parse_zcode(log)
    jr = journal(runid)
    tr = kg_from_transcript(host, log, sid)
    kg_tools = dict(jr["kg_tools"])
    for tool, count in tr["kg_tools"].items():
        kg_tools[tool] = max(kg_tools.get(tool, 0), count)
    p50, p95 = latency_stats(jr["latencies_us"])
    feature = FEATURE_TOOL[task]
    rec = {
        "runid": runid, "host": host, "arm": arm, "task": task, "run": run,
        "exit": rc, "duration_s": duration, **parsed,
        "journal_calls": jr["kg_calls"],
        "transcript_kg_calls": tr["kg_calls"],
        "kg_calls": max(jr["kg_calls"], tr["kg_calls"]),
        "kg_tools": kg_tools,
        "feature_calls": kg_tools.get(feature, 0),
        "kg_latency_p50_us": p50,
        "kg_latency_p95_us": p95,
        "direct_sqlite": tr["direct_sqlite"],
        "answered": answer.exists(),
        "violations": [],
    }
    if arm == "kgc" and rec["feature_calls"] > 0:
        rec["violations"].append(f"control arm called {feature}")
    if arm == "kgp8" and rec["feature_calls"] == 0:
        rec["violations"].append(f"treatment did not call {feature}")
    if rec["tools"] == 0:
        rec["violations"].append("zero tool calls")
    if rec["direct_sqlite"] > 0:
        rec["violations"].append("direct sqlite/index.db query")
    if rec["tools"] > 25:
        rec["violations"].append(f"tool budget exceeded ({rec['tools']} > 25)")
    with METRICS.open("a") as metrics:
        metrics.write(json.dumps(rec) + "\n")
    with MANIFEST.open("a") as manifest:
        manifest.write(manifest_row + "\n")
    with STATUS.open("a") as status:
        status.write(f"[done] {runid} exit={rc} tools={rec['tools']} "
                     f"kg={rec['kg_calls']} dur={duration}s "
                     f"{time.strftime('%H:%M:%S')}\n")
    print(f"[done] {runid} exit={rc} tools={rec['tools']} "
          f"kg={rec['kg_calls']} dur={duration}s", flush=True)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--max", type=int, default=999)
    ap.add_argument("--smoke", action="store_true")
    ap.add_argument("--cell", nargs=4, metavar=("HOST", "ARM", "TASK", "RUN"))
    args = ap.parse_args()
    if args.cell:
        host, arm, task, run = args.cell
        if host not in HOST_TAGS or arm not in ARMS or task not in TASKS:
            ap.error("invalid cell")
        run_cell(host, arm, task, int(run))
        return 0
    done = 0
    for host, arm, task, run in runids(args.smoke):
        if done >= args.max:
            break
        hosttag = HOST_TAGS[host]
        runid = f"{hosttag}-{arm}-r{run}-{task}"
        if STATUS.exists() and f"[done] {runid} " in STATUS.read_text():
            continue
        done += 1
        print(f"=== cell {done}/{args.max}: {runid} ===", flush=True)
        try:
            run_cell(host, arm, task, run)
        except Exception as exc:
            with STATUS.open("a") as status:
                status.write(f"[error] {runid} {exc!r}\n")
            print(f"[error] {runid} {exc!r}", flush=True)
    print(f"=== runner done: {done} new cells ===", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
