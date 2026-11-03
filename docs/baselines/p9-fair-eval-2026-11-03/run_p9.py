#!/usr/bin/env python3
"""Agent-utility A/B runner for the P9 surfaces.

Cells: 5 tasks x 2 arms x 4 hosts x 2 runs = 80.
Usage:
  python3 -I run_p9.py                # run all pending cells
  python3 -I run_p9.py --smoke        # one I1 control/treatment pair on claude
  python3 -I run_p9.py --max N
  python3 -I run_p9.py --cell HOST ARM TASK RUN
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import signal
import statistics
import subprocess
import sys
import time
import uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
AW = ROOT / "target/release/aw"
GROK = Path.home() / ".grok/bin/grok"
AGY = Path.home() / ".local/bin/agy"
ZCODE = Path("/opt/homebrew/bin/zcode")
CLAUDE = Path.home() / ".local/bin/claude"
CLAUDE_MODEL = "deepseek-v4-pro"
CLAUDE_BASE_URL = "https://api.deepseek.com/anthropic"
ANSWERS = Path("/tmp/p9-utility-answers-2026-11-03")
CORPORA = {
    "I1": Path("/tmp/p9-corpus-2026-11-03/kg"),
    "T1": Path("/tmp/p9-corpus-2026-11-03/vihap"),
    "H1": Path("/tmp/p9-corpus-2026-11-03/vihap"),
    "D1": Path("/tmp/p9-corpus-2026-11-03/vihap"),
    "A2": Path("/tmp/p9-corpus-2026-11-03/finguard"),
}
FEATURE_TOOL = {
    "I1": "imports",
    "T1": "callers",
    "H1": "implementers",
    "D1": "impact",
    "A2": "symbols",
}
TASKS = ("I1", "T1", "H1", "D1", "A2")
ARMS = ("kgc", "kgp9")
DEFAULT_HOSTS = ("grok", "agy", "zcode", "claude")
SMOKE_HOST = "claude"
HOST_TAGS = {"grok": "grok", "agy": "agy", "zcode": "zcode",
             "claude": "claude"}
AW_SUBCOMMANDS = (
    "build", "callers", "callees", "path", "context", "implementers",
    "imports", "tests", "impact", "dead", "unresolved", "symbols",
    "complexity", "resolve", "files", "search", "clones", "stats",
    "hotspots", "report", "doctor", "usage", "mcp",
)
AW_RE = re.compile(
    r"(?:target/release/)?\baw\s+(?:--[a-z-]+\s+\S+\s+)*("
    + "|".join(AW_SUBCOMMANDS) + r")\b"
)
STATUS = HERE / "status-p9.txt"
MANIFEST = HERE / "manifest-p9.txt"
METRICS = HERE / "metrics-p9.jsonl"
LOGS = HERE / "logs-p9"
ANSWERS_COPY = HERE / "answers-p9"
KILL_SECS = 300


def configure_paths(suffix: str) -> None:
    """Redirect artifacts so a new run never overwrites prior evidence."""
    global STATUS, MANIFEST, METRICS, LOGS, ANSWERS, ANSWERS_COPY
    STATUS = HERE / f"status-p9{suffix}.txt"
    MANIFEST = HERE / f"manifest-p9{suffix}.txt"
    METRICS = HERE / f"metrics-p9{suffix}.jsonl"
    LOGS = HERE / f"logs-p9{suffix}"
    ANSWERS = Path(f"/tmp/p9-utility-answers-2026-11-03{suffix}")
    ANSWERS_COPY = HERE / f"answers-p9{suffix}"


def runids(smoke: bool = False,
           hosts: tuple[str, ...] = DEFAULT_HOSTS) -> list[tuple[str, str, str, int]]:
    if smoke:
        return [(SMOKE_HOST, "kgp9", "I1", 1),
                (SMOKE_HOST, "kgc", "I1", 1)]
    cells: list[tuple[str, str, str, int]] = []
    for host in hosts:
        for task in TASKS:
            for run in (1, 2):
                # ABBA: r1 control/treatment, r2 treatment/control.
                if run == 1:
                    cells.extend([(host, "kgc", task, run),
                                  (host, "kgp9", task, run)])
                else:
                    cells.extend([(host, "kgp9", task, run),
                                  (host, "kgc", task, run)])
    return cells


def check_index(repo: Path) -> None:
    index = repo / ".aw" / "index.db"
    if not index.is_file():
        raise RuntimeError(f"missing pre-built index: {index}")


def journal(runid: str) -> dict:
    out = {"aw_calls": 0, "aw_tools": {}, "latencies_us": []}
    log = LOGS / f"{runid}.usage.log"
    if not log.exists():
        return out
    for line in log.read_text(errors="replace").splitlines():
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            continue
        # Per-cell file isolation replaces env attribution: the journal was
        # wiped before the cell and moved after it.
        out["aw_calls"] += 1
        tool = rec.get("tool", "?")
        out["aw_tools"][tool] = out["aw_tools"].get(tool, 0) + 1
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
    if host in {"agy", "zcode", "claude"}:
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
            elif host in {"zcode", "claude"} and rec.get("type") == "assistant":
                for part in (rec.get("message") or {}).get("content") or []:
                    if (part.get("type") == "tool_use"
                            and part.get("name") == "Bash"):
                        cmd = (part.get("input") or {}).get("command")
                        if isinstance(cmd, str):
                            cmds.append(cmd)
        return cmds
    # grok: scan recognized JSON command fields in the captured output
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


def aw_from_transcript(host: str, path: Path, sid: str | None = None) -> dict:
    out = {"aw_calls": 0, "aw_tools": {}, "direct_sqlite": 0}
    for cmd in transcript_commands(host, path, sid):
        if "sqlite3 " in cmd or ".aw/index.db" in cmd:
            out["direct_sqlite"] += 1
        for match in AW_RE.finditer(cmd):
            out["aw_calls"] += 1
            tool = match.group(1)
            out["aw_tools"][tool] = out["aw_tools"].get(tool, 0) + 1
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


def run_cell(host: str, arm: str, task: str, run: int, aw: Path = AW) -> None:
    hosttag = HOST_TAGS[host]
    runid = f"{hosttag}-{arm}-r{run}-{task}"
    if STATUS.exists() and f"[done] {runid} " in STATUS.read_text():
        print(f"[skip] {runid}", flush=True)
        return
    repo = CORPORA[task]
    check_index(repo)
    for pattern in ("usage.log*", "usage-results.jsonl*"):
        for path in (repo / ".aw").glob(pattern):
            path.unlink(missing_ok=True)
    ANSWERS.mkdir(parents=True, exist_ok=True)
    LOGS.mkdir(parents=True, exist_ok=True)
    answer = ANSWERS / f"{runid}.json"
    answer.unlink(missing_ok=True)
    prompt_path = LOGS / f"{runid}.prompt.txt"
    prompt_path.write_text(
        (HERE / "prompts" / f"{arm}-{task}.txt").read_text().replace(
            "{ANSWER_PATH}", str(answer)
        )
        + "\nAdditional isolation rules for this run (2026-11-03):\n"
          "- Do not read or list any path outside the repository, except the\n"
          "  exact answer file path above when writing it.\n"
          "- Do not read other answer files, previous-run artifacts, or\n"
          "  .aw/index.db directly. Use only the aw CLI and repository files.\n"
          "- Use at most 25 tool calls. Stop immediately after writing the answer.\n",
        encoding="utf-8",
    )
    log = LOGS / f"{runid}.jsonl"
    err = LOGS / f"{runid}.err"
    if host == "grok":
        sid = str(uuid.uuid4()).lower()
        cmd = [str(GROK), "--always-approve", "--max-turns", "40",
               "--session-id", sid, "--prompt-file", str(prompt_path)]
    elif host == "agy":
        sid = None
        cmd = [str(AGY), "--output-format", "stream-json",
               "--dangerously-skip-permissions",
               "--model", "gemini-3.8-flash-high",
               "--print-timeout", f"{KILL_SECS}s",
               f"-p={prompt_path.read_text()}"]
    elif host == "zcode":
        sid = None
        cmd = [str(ZCODE), "--permission-mode", "yolo",
               "--output-format", "stream-json", "--verbose", "-p",
               prompt_path.read_text()]
    else:
        sid = None
        cmd = [str(CLAUDE), "-p", prompt_path.read_text(),
               "--output-format", "stream-json", "--verbose",
               "--dangerously-skip-permissions",
               "--model", CLAUDE_MODEL,
               "--add-dir", str(ANSWERS)]
    manifest_row = f"{runid}|{host}|{arm}|{task}|{sid or ''}"
    with STATUS.open("a") as status:
        status.write(f"[start] {runid} host={host} repo={repo} "
                     f"{time.strftime('%H:%M:%S')}\n")
    env = dict(os.environ)
    if host == "claude":
        key = env.get("DEEPSEEK_API_KEY", "").strip()
        if not key:
            raise RuntimeError("DEEPSEEK_API_KEY is not set")
        env.pop("ANTHROPIC_API_KEY", None)
        env.pop("CLAUDE_CODE_OAUTH_TOKEN", None)
        env.update({
            "ANTHROPIC_BASE_URL": CLAUDE_BASE_URL,
            "ANTHROPIC_AUTH_TOKEN": key,
            "ANTHROPIC_MODEL": CLAUDE_MODEL,
            "ANTHROPIC_SMALL_FAST_MODEL": CLAUDE_MODEL,
            "ANTHROPIC_DEFAULT_HAIKU_MODEL": CLAUDE_MODEL,
            "ANTHROPIC_DEFAULT_SONNET_MODEL": CLAUDE_MODEL,
            "ANTHROPIC_DEFAULT_OPUS_MODEL": CLAUDE_MODEL,
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
            "DISABLE_TELEMETRY": "1",
            "DISABLE_AUTOUPDATER": "1",
        })
    env["AW_SESSION"] = runid
    env["AW_ARM"] = runid
    env["PATH"] = str(aw.parent) + os.pathsep + env.get("PATH", "")
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
        source = repo / ".aw" / name
        if source.exists():
            shutil.move(str(source), str(LOGS / f"{runid}.{name}"))
    if answer.exists():
        ANSWERS_COPY.mkdir(parents=True, exist_ok=True)
        shutil.copy(answer, ANSWERS_COPY / answer.name)

    if host == "grok":
        parsed = grok_parse(sid)
    elif host == "agy":
        parsed = parse_agy(log)
    else:
        parsed = parse_zcode(log)
    jr = journal(runid)
    tr = aw_from_transcript(host, log, sid)
    aw_tools = dict(jr["aw_tools"])
    for tool, count in tr["aw_tools"].items():
        aw_tools[tool] = max(aw_tools.get(tool, 0), count)
    p50, p95 = latency_stats(jr["latencies_us"])
    feature = FEATURE_TOOL[task]
    rec = {
        "runid": runid, "host": host, "arm": arm, "task": task, "run": run,
        "exit": rc, "duration_s": duration, **parsed,
        "journal_calls": jr["aw_calls"],
        "transcript_aw_calls": tr["aw_calls"],
        "aw_calls": max(jr["aw_calls"], tr["aw_calls"]),
        "aw_tools": aw_tools,
        "feature_calls": aw_tools.get(feature, 0),
        "aw_latency_p50_us": p50,
        "aw_latency_p95_us": p95,
        "direct_sqlite": tr["direct_sqlite"],
        "answered": answer.exists(),
        "violations": [],
    }
    if arm == "kgc" and rec["feature_calls"] > 0:
        rec["violations"].append(f"control arm called {feature}")
    if arm == "kgp9" and rec["feature_calls"] == 0:
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
                     f"aw={rec['aw_calls']} dur={duration}s "
                     f"{time.strftime('%H:%M:%S')}\n")
    print(f"[done] {runid} exit={rc} tools={rec['tools']} "
          f"aw={rec['aw_calls']} dur={duration}s", flush=True)


def parse_hosts(value: str) -> tuple[str, ...]:
    hosts = tuple(host.strip() for host in value.split(",") if host.strip())
    unknown = [host for host in hosts if host not in HOST_TAGS]
    if unknown:
        raise argparse.ArgumentTypeError(f"unknown hosts: {', '.join(unknown)}")
    return hosts


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--max", type=int, default=999)
    ap.add_argument("--smoke", action="store_true")
    ap.add_argument("--cell", nargs=4, metavar=("HOST", "ARM", "TASK", "RUN"))
    ap.add_argument("--hosts", type=parse_hosts, default=DEFAULT_HOSTS)
    ap.add_argument("--aw", type=Path, default=AW)
    ap.add_argument("--suffix", default="")
    args = ap.parse_args()
    if not args.aw.is_file():
        ap.error(f"aw binary not found: {args.aw}")
    requested = set(args.hosts)
    if args.cell:
        requested.add(args.cell[0])
    if "claude" in requested and not CLAUDE.is_file():
        ap.error(f"claude binary not found: {CLAUDE}")
    configure_paths(args.suffix)
    if args.cell:
        host, arm, task, run = args.cell
        if host not in HOST_TAGS or arm not in ARMS or task not in TASKS:
            ap.error("invalid cell")
        run_cell(host, arm, task, int(run), args.aw)
        return 0
    done = 0
    for host, arm, task, run in runids(args.smoke, args.hosts):
        if done >= args.max:
            break
        hosttag = HOST_TAGS[host]
        runid = f"{hosttag}-{arm}-r{run}-{task}"
        if STATUS.exists() and f"[done] {runid} " in STATUS.read_text():
            continue
        done += 1
        print(f"=== cell {done}/{args.max}: {runid} ===", flush=True)
        try:
            run_cell(host, arm, task, run, args.aw)
        except Exception as exc:
            with STATUS.open("a") as status:
                status.write(f"[error] {runid} {exc!r}\n")
            print(f"[error] {runid} {exc!r}", flush=True)
    print(f"=== runner done: {done} new cells ===", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
