#!/usr/bin/env python3
"""Stage-1 utility A/B audit: host accounting + kg journal attestation.

Reads docs/baselines/utility-run-2026-11-02/{manifest,status}.txt and emits
audit.json: per-cell host tokens/model calls, tool counts, kg call counts,
feature-call counts, and the contamination verdict.

Sources per host (precedent: grok-a-b-2026-11-01.md):
  grok:     ~/.grok/sessions/*/<sid>/usage.json (tokens, model, modelCalls)
            + chat_history.jsonl (tool_result count, output bytes)
  opencode: ~/.local/share/opencode/opencode.db session_message rows:
            assistant data.tokens {input, output}, assistant messages =
            model calls, content entries with type == "tool" = tool calls
            (output bytes not exposed -> null, recorded honestly)
  both:     <repo>/.kg/usage.log + usage-results.jsonl filtered by arm==runid
            (kg calls, tool breakdown, feature-call contamination)
Usage: python3 audit.py            # writes audit.json next to this script
"""
from __future__ import annotations

import json
import os
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parent
KG_ROOT = Path("/tmp/utility-corpus-2026-11-02/kg")
VIHAP_ROOT = Path("/tmp/utility-corpus-2026-11-02/vihap")
REPOS = {"P1": KG_ROOT, "P2": KG_ROOT, "P3": VIHAP_ROOT, "S1": KG_ROOT, "S2": KG_ROOT, "S3": VIHAP_ROOT}
FEATURE_TOOL = {"P1": "path", "P2": "path", "P3": "path",  # Stage 1
                 "S1": "search", "S2": "search", "S3": "search"}  # Stage 2
OC_DB = Path.home() / ".local/share/opencode/opencode.db"


def journal_calls(repo: Path, runid: str) -> dict:
    calls = {"kg_calls": 0, "kg_tool_breakdown": {}, "violations": []}
    log = ROOT / "logs" / f"{runid}.usage.log"
    if not log.exists():
        return calls
    for line in log.read_text().splitlines():
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            continue
        if rec.get("arm") != runid:
            continue
        calls["kg_calls"] += 1
        tool = rec.get("tool", "?")
        calls["kg_tool_breakdown"][tool] = calls["kg_tool_breakdown"].get(tool, 0) + 1
    return calls


def grok_accounting(sid: str) -> dict:
    out = {"tok_in": None, "tok_out": None, "model": None,
           "model_calls": None, "tools": None, "out_bytes": None}
    for d in Path.home().glob(f".grok/sessions/*/{sid}"):
        usage = d / "usage.json"
        if usage.exists():
            u = json.loads(usage.read_text()).get("session", {})
            out["tok_in"] = u.get("inputTokens")
            out["tok_out"] = u.get("outputTokens")
            out["model"] = u.get("primaryModelId")
            out["model_calls"] = u.get("modelCalls")
        hist = d / "chat_history.jsonl"
        if hist.exists():
            tools = 0
            nbytes = 0
            for line in hist.read_text().splitlines():
                try:
                    rec = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if rec.get("type") == "tool_result":
                    tools += 1
                    c = rec.get("content")
                    if isinstance(c, str):
                        nbytes += len(c)
            out["tools"] = tools
            out["out_bytes"] = nbytes
    return out


def opencode_accounting(sid: str) -> dict:
    out = {"tok_in": None, "tok_out": None, "model": None,
           "model_calls": None, "tools": None, "out_bytes": None}
    if not OC_DB.exists():
        return out
    conn = sqlite3.connect(f"file:{OC_DB}?mode=ro", uri=True)
    try:
        rows = conn.execute(
            "SELECT data FROM session_message WHERE session_id = ? AND type = 'assistant'",
            (sid,),
        ).fetchall()
    finally:
        conn.close()
    tok_in = tok_out = tools = 0
    model = None
    calls = 0
    for (data,) in rows:
        try:
            d = json.loads(data)
        except json.JSONDecodeError:
            continue
        calls += 1
        t = d.get("tokens") or {}
        tok_in += t.get("input", 0)
        tok_out += t.get("output", 0)
        m = d.get("model") or {}
        model = model or m.get("id")
        for part in d.get("content") or []:
            if part.get("type") == "tool":
                tools += 1
    out.update(tok_in=tok_in or None, tok_out=tok_out or None,
               model=model, model_calls=calls or None, tools=tools)
    return out


def main() -> int:
    stage = os.environ.get("STAGE", "1")
    status_name = "status.txt" if stage == "1" else f"status{stage}.txt"
    manifest_name = "manifest.txt" if stage == "1" else f"manifest{stage}.txt"
    out_name = "audit.json" if stage == "1" else f"audit{stage}.json"
    manifest = {}
    for line in (ROOT / manifest_name).read_text().splitlines():
        parts = line.split("|")
        if len(parts) >= 5:
            runid, host, arm, task, sid = parts[:5]
            manifest[runid] = {"host": host, "arm": arm, "task": task, "sid": sid}
    status = {}
    for line in (ROOT / status_name).read_text().splitlines():
        if line.startswith("[done] "):
            parts = line.split()
            status[parts[1]] = parts[2].replace("exit=", "")
    audit = []
    for runid, meta in sorted(manifest.items()):
        repo = REPOS[meta["task"]]
        rec = {"runid": runid, **meta, "exit": status.get(runid)}
        rec.update(journal_calls(repo, runid))
        host = grok_accounting(meta["sid"]) if meta["host"] == "grok" \
            else opencode_accounting(meta["sid"])
        rec.update(host)
        rec["feature_calls"] = rec["kg_tool_breakdown"].get(FEATURE_TOOL[meta["task"]], 0)
        if meta["arm"] == "kgc" and rec["feature_calls"] > 0:
            rec["violations"].append(
                f"control arm called {FEATURE_TOOL[meta['task']]} {rec['feature_calls']}x")
        rec["answered"] = (ROOT / "answers" / f"{runid}.json").exists()
        audit.append(rec)
    (ROOT / out_name).write_text(json.dumps(audit, indent=1) + "\n")
    print(f"{out_name}: {len(audit)} cells")
    for r in audit:
        print(f"{r['runid']:14s} host={r['host']:8s} exit={r['exit']} "
              f"kg_calls={r['kg_calls']} feature={r['feature_calls']} "
              f"tools={r['tools']} tok_in={r['tok_in']} answered={r['answered']} "
              f"violations={len(r['violations'])}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
