#!/usr/bin/env python3
"""Interactive Telemetry, Anti-Reinvention & Memory Dashboard Generator for `aw` (`agent-weave`).

Self-contained, zero-dependency script reading `.aw/usage.log`, `.aw/index.db`,
and `.aw/memory.db` (local and global stores). Produces a responsive, single-file
HTML dashboard following Zen Minimalism (Shibumi Utilitarian) with dual light/dark
themes, real-time SSE updates, a dedicated Anti-Reinvention Inspector (D1-D4 duplicate
code and side-by-side source inspection), and an Agent Memory Inspector.
"""

from __future__ import annotations

import argparse
import html
import http.server
import json
import math
import os
from pathlib import Path
import re
import sqlite3
import subprocess
import sys
import time
from typing import Any, Dict, List, Optional, Tuple

# In-memory TTL cache for reuse candidates to avoid running aw reuse on every HTTP hit
_REUSE_CACHE: Dict[str, Tuple[float, List[Dict[str, Any]], Dict[str, Any], List[Dict[str, Any]]]] = {}


def resolve_global_aw_dir() -> Path:
    """Resolve global ~/.aw directory, honoring AW_GLOBAL_DIR."""
    val = os.environ.get("AW_GLOBAL_DIR")
    if val and val.strip():
        return Path(val.strip())
    home = os.environ.get("HOME") or os.environ.get("USERPROFILE") or "."
    return Path(home) / ".aw"


def parse_usage_log_from_dir(aw_dir: Path) -> List[Dict[str, Any]]:
    """Load journal entries from usage.log.1 and usage.log in aw_dir."""
    entries: List[Dict[str, Any]] = []
    for filename in ["usage.log.1", "usage.log"]:
        log_file = aw_dir / filename
        if not log_file.is_file():
            continue
        try:
            with open(log_file, "r", encoding="utf-8", errors="replace") as f:
                for line in f:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        entries.append(json.loads(line))
                    except json.JSONDecodeError:
                        continue
        except OSError:
            continue
    return entries


def parse_usage_log(root: Path) -> List[Dict[str, Any]]:
    """Load journal entries from root/.aw/."""
    return parse_usage_log_from_dir(root / ".aw")


def inspect_index_db(root: Optional[Path]) -> Dict[str, Any]:
    """Inspect .aw/index.db for indexed file and symbol counts."""
    stats: Dict[str, Any] = {
        "exists": False,
        "size_bytes": 0,
        "files": 0,
        "symbols": 0,
        "edges": 0,
        "languages": {},
    }
    if not root:
        return stats

    db_path = root / ".aw" / "index.db"
    if not db_path.is_file():
        return stats

    stats["exists"] = True
    try:
        stats["size_bytes"] = db_path.stat().st_size
        conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
        cur = conn.cursor()
        cur.execute("SELECT COUNT(*) FROM files")
        stats["files"] = cur.fetchone()[0]

        cur.execute("SELECT COUNT(*) FROM symbols")
        stats["symbols"] = cur.fetchone()[0]

        cur.execute("SELECT name FROM sqlite_master WHERE type='table' AND name='edges'")
        if cur.fetchone():
            cur.execute("SELECT COUNT(*) FROM edges")
            stats["edges"] = cur.fetchone()[0]

        cur.execute("SELECT path FROM files")
        for (fpath,) in cur.fetchall():
            ext = Path(fpath).suffix
            if ext:
                stats["languages"][ext] = stats["languages"].get(ext, 0) + 1
        conn.close()
    except (sqlite3.Error, OSError):
        pass

    return stats


def extract_code_snippet(file_path: Path, line_start: int, line_end: int, max_lines: int = 30) -> str:
    """Read formatted lines from source file with 1-based line numbers."""
    if not file_path.is_file():
        return ""
    try:
        content = file_path.read_text(encoding="utf-8", errors="replace")
        lines = content.splitlines()
        start_0 = max(0, line_start - 1)
        end_0 = min(len(lines), max(line_start, line_end))
        sub = lines[start_0:end_0]
        trimmed = sub[:max_lines]
        formatted = [f"{line_start + idx:4d} | {line}" for idx, line in enumerate(trimmed)]
        if len(sub) > max_lines:
            formatted.append(f" ... ({len(sub) - max_lines} more lines)")
        return "\n".join(formatted)
    except OSError:
        return ""


def lookup_symbol_in_db(cur: sqlite3.Cursor, sym: str) -> Optional[Tuple[str, str, str, int, int, str]]:
    """Lookup (name, qualname, file_path, line_start, line_end, signature) from symbols table."""
    try:
        cur.execute(
            "SELECT name, qualname, file_path, line_start, line_end, signature FROM symbols WHERE qualname = ?1 LIMIT 1",
            (sym,),
        )
        row = cur.fetchone()
        if not row:
            cur.execute(
                "SELECT name, qualname, file_path, line_start, line_end, signature FROM symbols WHERE qualname LIKE ?1 LIMIT 1",
                ("%" + sym,),
            )
            row = cur.fetchone()
        if not row:
            short = sym.split(".")[-1]
            cur.execute(
                "SELECT name, qualname, file_path, line_start, line_end, signature FROM symbols WHERE name = ?1 LIMIT 1",
                (short,),
            )
            row = cur.fetchone()
        return row
    except sqlite3.Error:
        return None


def collect_all_reuse_candidates(
    aw_dir: Path,
    entries: List[Dict[str, Any]],
    single_root: Optional[Path] = None,
    max_age_secs: float = 15.0,
) -> Tuple[List[Dict[str, Any]], Dict[str, Any], List[Dict[str, Any]]]:
    """Extract D1-D4 duplicate candidates, read side-by-side code snippets, and correlate pre-write searches."""
    cache_key = str(single_root.resolve()) if single_root else str(aw_dir.resolve())
    now = time.time()
    if cache_key in _REUSE_CACHE:
        cached_ts, c_list, s_dict, p_list = _REUSE_CACHE[cache_key]
        if now - cached_ts < max_age_secs:
            return c_list, s_dict, p_list

    # Discover repositories
    repos: Dict[str, Path] = {}
    if single_root:
        if (single_root / ".aw" / "index.db").is_file():
            repos[single_root.name] = single_root
    else:
        for e in entries:
            r = e.get("repo")
            r_name = e.get("repo_name")
            if r and r_name and r_name not in repos:
                rp = Path(r)
                if (rp / ".aw" / "index.db").is_file():
                    repos[r_name] = rp
        cwd = Path.cwd().resolve()
        if cwd.name not in repos and (cwd / ".aw" / "index.db").is_file():
            repos[cwd.name] = cwd

    candidates: List[Dict[str, Any]] = []

    for r_name, r_root in sorted(repos.items()):
        db_path = r_root / ".aw" / "index.db"
        if not db_path.is_file():
            continue

        # Run aw reuse to extract calibrated candidates
        try:
            cmd = ["aw", "--no-journal", "--root", str(r_root), "reuse", "--limit", "25"]
            res = subprocess.run(cmd, capture_output=True, text=True, timeout=3.5)
            output = res.stdout
        except (subprocess.SubprocessError, OSError):
            output = ""

        conn: Optional[sqlite3.Connection] = None
        try:
            conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
            cur = conn.cursor()
        except (sqlite3.Error, OSError):
            cur = None

        lines = output.strip().splitlines()
        i = 0
        while i < len(lines):
            line = lines[i].strip()
            if line.startswith("symbol '"):
                m = re.match(
                    r"symbol '([^']+)' (shares (\d+)% (callee footprint|interface token overlap) with existing|bypasses canonical client|is a pass-through wrapper around existing) '([^']+)' \(\[callers=(\d+)\] in ([^:]+):(\d+)\)",
                    line,
                )
                shared_line = ""
                if i + 1 < len(lines) and lines[i + 1].startswith("  "):
                    shared_line = lines[i + 1].strip()
                    i += 1

                if m:
                    sym = m.group(1)
                    canonical = m.group(5)
                    callers = int(m.group(6))
                    c_file = m.group(7)
                    c_line = int(m.group(8))
                    pct = int(m.group(3)) if m.group(3) else 100

                    if "callee footprint" in line:
                        det = "D1"
                        det_title = "D1 Callee Footprint Overlap"
                        det_summary = f"Workflow '{sym}' calls identical downstream functions as canonical '{canonical}' ([callers={callers}])."
                    elif "interface token" in line:
                        det = "D2"
                        det_title = "D2 Interface Token Overlap"
                        det_summary = f"Signature and parameters of '{sym}' duplicate canonical '{canonical}' ([callers={callers}])."
                    elif "bypasses" in line:
                        det = "D3"
                        det_title = "D3 Raw Client Bypass"
                        det_summary = f"Symbol '{sym}' calls raw unmanaged external clients directly instead of canonical '{canonical}' ([callers={callers}])."
                    else:
                        det = "D4"
                        det_title = "D4 Redundant Pass-Through Wrapper"
                        det_summary = f"Single-caller wrapper '{sym}' adds zero logic around canonical '{canonical}' ([callers={callers}])."

                    # Symbol lookup in database
                    candidate_info = {"file": "", "line_start": 0, "line_end": 0, "signature": "", "loc": 0}
                    canonical_info = {
                        "file": c_file,
                        "line_start": c_line,
                        "line_end": c_line,
                        "signature": "",
                        "loc": 0,
                    }

                    if cur:
                        row_sym = lookup_symbol_in_db(cur, sym)
                        if row_sym:
                            candidate_info["file"] = row_sym[2]
                            candidate_info["line_start"] = row_sym[3]
                            candidate_info["line_end"] = row_sym[4]
                            candidate_info["signature"] = row_sym[5] or ""
                            candidate_info["loc"] = max(1, row_sym[4] - row_sym[3] + 1)

                        row_can = lookup_symbol_in_db(cur, canonical)
                        if row_can:
                            canonical_info["file"] = row_can[2]
                            canonical_info["line_start"] = row_can[3]
                            canonical_info["line_end"] = row_can[4]
                            canonical_info["signature"] = row_can[5] or ""
                            canonical_info["loc"] = max(1, row_can[4] - row_can[3] + 1)

                    # Read real source code snippets
                    candidate_snippet = ""
                    canonical_snippet = ""
                    if candidate_info["file"] and candidate_info["line_start"] > 0:
                        candidate_snippet = extract_code_snippet(
                            r_root / candidate_info["file"],
                            candidate_info["line_start"],
                            candidate_info["line_end"],
                        )
                    if canonical_info["file"] and canonical_info["line_start"] > 0:
                        end_line = canonical_info["line_end"] if canonical_info["line_end"] >= canonical_info["line_start"] else canonical_info["line_start"] + 15
                        canonical_snippet = extract_code_snippet(
                            r_root / canonical_info["file"],
                            canonical_info["line_start"],
                            end_line,
                        )

                    # Narrative and guidance
                    loc_saving = candidate_info["loc"] if candidate_info["loc"] > 0 else 10
                    prevention_narrative = (
                        f"Consolidating '{sym}' into authoritative '{canonical}' ([callers={callers}]) "
                        f"deletes ~{loc_saving} duplicate lines and eliminates drift between twin implementations."
                    )

                    candidates.append({
                        "id": f"{r_name}:{sym}->{canonical}",
                        "repo": r_name,
                        "repo_path": str(r_root),
                        "detector": det,
                        "detector_title": det_title,
                        "detector_summary": det_summary,
                        "symbol": sym,
                        "canonical": canonical,
                        "canonical_callers": callers,
                        "canonical_file": canonical_info["file"],
                        "canonical_line": canonical_info["line_start"],
                        "overlap_pct": pct,
                        "shared_evidence": shared_line,
                        "candidate_info": candidate_info,
                        "canonical_info": canonical_info,
                        "candidate_snippet": candidate_snippet,
                        "canonical_snippet": canonical_snippet,
                        "loc_saving": loc_saving,
                        "prevention_narrative": prevention_narrative,
                    })
            i += 1

        if conn:
            conn.close()

    # Extract Pre-Write Search Telemetry
    prewrite_searches: List[Dict[str, Any]] = []
    for e in reversed(entries):
        tool = e.get("tool")
        if tool in ("search", "reuse"):
            args = e.get("args") or {}
            terms = args.get("terms", "") or args.get("paths", "")
            prewrite_searches.append({
                "ts": e.get("ts", 0),
                "repo_name": e.get("repo_name") or "workspace",
                "tool": tool,
                "terms": terms,
                "shown": e.get("shown", e.get("result_count", 0)),
                "exit_code": e.get("exit_code", 0),
                "latency_us": e.get("latency_us", 0),
            })
            if len(prewrite_searches) >= 40:
                break

    # Calculate summary metrics
    total_candidates = len(candidates)
    preventable_loc = sum(c["loc_saving"] for c in candidates)
    prewrite_count = len([e for e in entries if e.get("tool") in ("search", "reuse")])
    avg_callers = (
        round(sum(c["canonical_callers"] for c in candidates) / total_candidates, 1)
        if total_candidates > 0
        else 0.0
    )

    summary_metrics = {
        "total_candidates": total_candidates,
        "preventable_loc": preventable_loc,
        "prewrite_count": prewrite_count,
        "avg_callers": avg_callers,
        "detector_counts": {
            "D1": sum(1 for c in candidates if c["detector"] == "D1"),
            "D2": sum(1 for c in candidates if c["detector"] == "D2"),
            "D3": sum(1 for c in candidates if c["detector"] == "D3"),
            "D4": sum(1 for c in candidates if c["detector"] == "D4"),
        },
    }

    _REUSE_CACHE[cache_key] = (now, candidates, summary_metrics, prewrite_searches)
    return candidates, summary_metrics, prewrite_searches


def collect_all_memories(
    aw_dir: Path,
    entries: List[Dict[str, Any]],
    single_root: Optional[Path] = None,
) -> List[Dict[str, Any]]:
    """Collect memory notes, enrich with recall frequency from telemetry, and verify code references."""
    stores: Dict[str, Tuple[Path, Path]] = {}

    if single_root:
        p = single_root / ".aw" / "memory.db"
        if p.is_file():
            stores[single_root.name] = (single_root, p)
    else:
        # Global store
        g = aw_dir / "memory.db"
        if g.is_file():
            stores["global"] = (aw_dir, g)

        # Repositories recorded in telemetry
        for e in entries:
            r = e.get("repo")
            r_name = e.get("repo_name")
            if r and r_name and r_name not in stores:
                p = Path(r) / ".aw" / "memory.db"
                if p.is_file():
                    stores[r_name] = (Path(r), p)

        # Fallback to current working directory
        cwd = Path.cwd().resolve()
        if cwd.name not in stores:
            p = cwd / ".aw" / "memory.db"
            if p.is_file():
                stores[cwd.name] = (cwd, p)

    # Pre-aggregate recall activity from usage.log for memory notes
    mem_recalls: Dict[str, Dict[str, Any]] = {}
    for e in entries:
        tool = e.get("tool")
        if tool in ("mem_query", "mem_get"):
            ts = e.get("ts", 0)
            args = e.get("args") or {}
            key_arg = str(args.get("key", ""))
            terms_arg = str(args.get("terms", "")).lower()
            all_arg = str(args.get("all", "false")).lower() == "true"
            kind_arg = str(args.get("kind", "")).lower()

            # Record generic queries
            if key_arg:
                info = mem_recalls.setdefault(key_arg, {"recalls": 0, "last_ts": 0})
                info["recalls"] += 1
                if ts > info["last_ts"]:
                    info["last_ts"] = ts

    all_notes: List[Dict[str, Any]] = []
    for store_name, (repo_root, db_path) in sorted(stores.items()):
        index_db_path = repo_root / ".aw" / "index.db"
        index_conn: Optional[sqlite3.Connection] = None
        if index_db_path.is_file():
            try:
                index_conn = sqlite3.connect(f"file:{index_db_path}?mode=ro", uri=True)
            except (sqlite3.Error, OSError):
                index_conn = None

        try:
            conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
            cur = conn.cursor()
            cur.execute("""
                SELECT id, key, rev, kind, scope, session, body, tags, refs,
                       content_hash, source, created_at, expires_at, superseded_by, superseded_at
                FROM memories
                ORDER BY key ASC, rev DESC
            """)
            for row in cur.fetchall():
                key = str(row[1])
                rev = int(row[2])
                kind = str(row[3])
                scope = str(row[4])
                session = str(row[5])
                body = str(row[6])
                tags = [t for t in str(row[7]).split() if t]
                refs_raw = [r for r in str(row[8]).split() if r]
                content_hash = hex(int(row[9]))
                source = str(row[10])
                created_at = int(row[11])
                expires_at = int(row[12])
                superseded_by = row[13]
                superseded_at = int(row[14])
                is_current = superseded_by is None

                # Compute Prevention Classification & Narrative
                if kind == "gotcha":
                    prev_category = "REGRESSION GUARD"
                    prev_title = "Regression & Bug Guard"
                    prev_detail = f"Prevents repeating root-cause trap in '{key}'. Blocks known fragility before changes are committed."
                elif kind == "decision":
                    prev_category = "ARCHITECTURE INVARIANT"
                    prev_title = "Architecture & Policy Guard"
                    prev_detail = f"Enforces repository architectural constraint '{key}'. Prevents forbidden abstractions or boundary violations."
                elif kind == "fact":
                    prev_category = "EXPLORATION SHORTCUT"
                    prev_title = "Verified Fact Shortcut"
                    prev_detail = f"Avoids costly multi-file grep scans by caching verified system truth for '{key}'."
                else:
                    prev_category = "HANDOFF CONTEXT"
                    prev_title = "Cross-Session Handoff Guard"
                    prev_detail = f"Preserves active state across conversation turns for '{key}'."

                # Correlate agent recall frequency
                recall_info = mem_recalls.get(key, {"recalls": 0, "last_ts": 0})
                recalls = recall_info["recalls"]
                last_recalled_ts = recall_info["last_ts"]

                # If no direct key recall logged, match terms from queries
                if recalls == 0:
                    for e in entries:
                        if e.get("tool") in ("mem_query", "mem_get"):
                            args = e.get("args") or {}
                            terms = str(args.get("terms", "")).lower()
                            if terms and (terms in key.lower() or terms in body.lower()):
                                recalls += 1
                                e_ts = e.get("ts", 0)
                                if e_ts > last_recalled_ts:
                                    last_recalled_ts = e_ts

                # Resolve code references against index.db
                resolved_refs: List[Dict[str, Any]] = []
                for r in refs_raw:
                    ref_info = {"ref": r, "verified": False, "type": "path", "location": ""}
                    if index_conn:
                        try:
                            icur = index_conn.cursor()
                            # Try files table
                            icur.execute("SELECT path, size FROM files WHERE path = ?1 OR path LIKE ?2 LIMIT 1", (r, f"%{r}%"))
                            frow = icur.fetchone()
                            if frow:
                                ref_info["verified"] = True
                                ref_info["type"] = "file"
                                ref_info["location"] = f"{frow[0]} ({frow[1]} bytes)"
                            else:
                                # Try symbols table
                                icur.execute("SELECT name, qualname, file_path, line_start, line_end FROM symbols WHERE name = ?1 OR qualname = ?1 LIMIT 1", (r,))
                                srow = icur.fetchone()
                                if srow:
                                    ref_info["verified"] = True
                                    ref_info["type"] = "symbol"
                                    ref_info["location"] = f"{srow[2]}:{srow[3]}-{srow[4]}"
                        except sqlite3.Error:
                            pass
                    resolved_refs.append(ref_info)

                all_notes.append({
                    "id": row[0],
                    "store": store_name,
                    "store_path": str(repo_root),
                    "key": key,
                    "rev": rev,
                    "kind": kind,
                    "scope": scope,
                    "session": session,
                    "body": body,
                    "tags": tags,
                    "refs": refs_raw,
                    "resolved_refs": resolved_refs,
                    "content_hash": content_hash,
                    "source": source,
                    "created_at": created_at,
                    "expires_at": expires_at,
                    "is_current": is_current,
                    "superseded_by": superseded_by,
                    "superseded_at": superseded_at,
                    "prev_category": prev_category,
                    "prev_title": prev_title,
                    "prev_detail": prev_detail,
                    "recall_count": recalls,
                    "last_recalled_ts": last_recalled_ts,
                })
            conn.close()
        except (sqlite3.Error, OSError):
            pass

        if index_conn:
            index_conn.close()

    return all_notes


def calculate_metrics(entries: List[Dict[str, Any]]) -> Dict[str, Any]:
    """Calculate aggregated telemetry metrics, per-repo breakdown, and errors."""
    total_calls = len(entries)
    if total_calls == 0:
        return {
            "total_calls": 0,
            "error_count": 0,
            "warning_count": 0,
            "stale_reads": 0,
            "reliability_pct": 100.0,
            "p50_ms": 0.0,
            "p95_ms": 0.0,
            "avg_ms": 0.0,
            "max_ms": 0.0,
            "saved_tokens": 0,
            "saved_dollars": 0.0,
            "reuse_hits": 0,
            "tool_aggs": {},
            "repo_aggs": {},
            "recent_queries": [],
            "recent_errors": [],
        }

    errors = 0
    warnings = 0
    stale_reads = 0
    reuse_hits = 0
    saved_tokens = 0
    latencies: List[float] = []
    tool_aggs: Dict[str, Dict[str, Any]] = {}
    repo_aggs: Dict[str, Dict[str, Any]] = {}
    all_errors: List[Dict[str, Any]] = []

    for e in entries:
        tool = str(e.get("tool", "unknown"))
        exit_code = int(e.get("exit_code", 0))
        shown = int(e.get("shown", e.get("result_count", 0)))
        result_count = int(e.get("result_count", 0))
        stale = bool(e.get("stale", False))
        lat_us = e.get("latency_us")
        ts = e.get("ts")
        repo_name = str(e.get("repo_name") or "workspace")
        is_worktree = bool(e.get("worktree", False))

        if repo_name.startswith(("aw-it-", "aw-mcp-", "usage-quality", "journal-", "global-usage-")):
            continue

        if tool not in tool_aggs:
            tool_aggs[tool] = {
                "calls": 0,
                "errors": 0,
                "warnings": 0,
                "shown": 0,
                "latencies": [],
            }

        tool_aggs[tool]["calls"] += 1
        tool_aggs[tool]["shown"] += shown

        if repo_name not in repo_aggs:
            repo_aggs[repo_name] = {
                "calls": 0,
                "errors": 0,
                "warnings": 0,
                "worktree": is_worktree,
                "last_ts": ts,
            }

        repo_aggs[repo_name]["calls"] += 1
        if ts and (repo_aggs[repo_name]["last_ts"] is None or ts > repo_aggs[repo_name]["last_ts"]):
            repo_aggs[repo_name]["last_ts"] = ts

        if exit_code >= 3:
            errors += 1
            tool_aggs[tool]["errors"] += 1
            repo_aggs[repo_name]["errors"] += 1
            all_errors.append(e)
        elif exit_code in (1, 2):
            warnings += 1
            tool_aggs[tool]["warnings"] += 1
            repo_aggs[repo_name]["warnings"] += 1

        if stale:
            stale_reads += 1

        if lat_us is not None and lat_us >= 0:
            ms = lat_us / 1000.0
            latencies.append(ms)
            tool_aggs[tool]["latencies"].append(ms)

        if exit_code == 0:
            if tool in ("context", "callers", "callees", "path", "impact", "imports"):
                saved_tokens += shown * 350 + 100
            elif tool == "search":
                saved_tokens += shown * 300 + 100
            elif tool == "reuse":
                reuse_hits += max(result_count, shown)
                saved_tokens += result_count * 800 + 1200
            elif tool == "symbols":
                saved_tokens += shown * 150
            elif tool == "build":
                saved_tokens += 500

    latencies.sort()
    n = len(latencies)
    p50 = latencies[int(math.ceil(0.50 * n)) - 1] if n else 0.0
    p95 = latencies[int(math.ceil(0.95 * n)) - 1] if n else 0.0
    avg = sum(latencies) / n if n else 0.0
    max_lat = latencies[-1] if n else 0.0

    for t, data in tool_aggs.items():
        t_lats = sorted(data["latencies"])
        tn = len(t_lats)
        data["p50"] = t_lats[int(math.ceil(0.50 * tn)) - 1] if tn else 0.0

    for r_name, r_data in repo_aggs.items():
        calls = r_data["calls"]
        errs = r_data["errors"]
        r_data["reliability_pct"] = 100.0 * (calls - errs) / calls if calls else 100.0

    reliability = 100.0 * (total_calls - errors) / total_calls if total_calls else 100.0
    saved_dollars = saved_tokens * 0.000003

    recent = list(reversed(entries[-50:]))
    recent_errors = list(reversed(all_errors[-25:]))

    return {
        "total_calls": total_calls,
        "error_count": errors,
        "warning_count": warnings,
        "stale_reads": stale_reads,
        "reliability_pct": reliability,
        "p50_ms": p50,
        "p95_ms": p95,
        "avg_ms": avg,
        "max_ms": max_lat,
        "saved_tokens": saved_tokens,
        "saved_dollars": saved_dollars,
        "reuse_hits": reuse_hits,
        "tool_aggs": tool_aggs,
        "repo_aggs": repo_aggs,
        "recent_queries": recent,
        "recent_errors": recent_errors,
    }


def render_html_dashboard(
    metrics: Dict[str, Any],
    index_stats: Dict[str, Any],
    scope_name: str,
    memories: List[Dict[str, Any]],
    reuse_candidates: List[Dict[str, Any]],
    reuse_summary: Dict[str, Any],
    prewrite_searches: List[Dict[str, Any]],
    is_global: bool = False,
) -> str:
    """Render single-file interactive HTML dashboard with Zen Minimalism, Anti-Reinvention, and Memory."""
    p50_str = f"{metrics['p50_ms']:.1f}ms"
    p95_str = f"{metrics['p95_ms']:.1f}ms"
    tokens_str = f"{metrics['saved_tokens']:,}"
    dollars_str = f"${metrics['saved_dollars']:.2f}"
    gen_time = time.strftime("%Y-%m-%d %H:%M:%S UTC", time.gmtime())

    repo_aggs = metrics.get("repo_aggs", {})
    repo_count = len(repo_aggs)
    total_calls = metrics["total_calls"]
    total_errors = metrics["error_count"]

    # Repositories Filter Options
    repo_options = ['<option value="ALL">All Repositories</option>']
    for r_name in sorted(repo_aggs.keys()):
        repo_options.append(f'<option value="{html.escape(r_name)}">{html.escape(r_name)}</option>')
    repo_options_html = "\n".join(repo_options)

    # Repositories Table Rows with slim 3px tracks
    repo_rows = []
    sorted_repos = sorted(repo_aggs.items(), key=lambda kv: kv[1]["calls"], reverse=True)
    max_repo_calls = max((r[1]["calls"] for r in sorted_repos), default=1)
    for r_name, r_data in sorted_repos:
        calls = r_data["calls"]
        errs = r_data["errors"]
        rel = r_data["reliability_pct"]
        wt = r_data["worktree"]
        last_ts = r_data["last_ts"]
        last_str = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(last_ts)) if last_ts else "—"
        pct = (calls / max_repo_calls) * 100.0 if max_repo_calls else 0.0
        wt_badge = '<span class="badge badge-wt">worktree</span>' if wt else '<span class="badge badge-repo">repo</span>'
        rel_class = "status-ok" if errs == 0 else "status-err"
        repo_rows.append(
            f'<tr class="repo-row-item" data-repo="{html.escape(r_name)}" onclick="filterByRepo(\'{html.escape(r_name)}\')">'
            f'<td>'
            f'  <div class="repo-name-wrap"><strong>{html.escape(r_name)}</strong> {wt_badge}</div>'
            f'  <div class="row-track"><div class="row-track-fill" style="width: {pct:.1f}%;"></div></div>'
            f'</td>'
            f'<td class="text-right num-col">{calls}</td>'
            f'<td><span class="badge {rel_class}">{rel:.1f}% ({errs} err)</span></td>'
            f'<td class="text-muted num-col">{last_str}</td>'
            f'</tr>'
        )
    repos_tbody = "\n".join(repo_rows) if repo_rows else "<tr><td colspan='4' class='empty-cell'>No repositories recorded</td></tr>"

    # Tool Distribution Rows with slim 3px tracks
    tool_rows = []
    tool_aggs = metrics.get("tool_aggs", {})
    sorted_tools = sorted(tool_aggs.items(), key=lambda kv: kv[1]["calls"], reverse=True)
    max_tool_calls = max((t[1]["calls"] for t in sorted_tools), default=1)
    for t_name, t_data in sorted_tools:
        calls = t_data["calls"]
        pct = (calls / max_tool_calls) * 100.0 if max_tool_calls else 0.0
        p50_t = f"{t_data.get('p50', 0.0):.1f}ms"
        share_pct = (calls / total_calls * 100.0) if total_calls else 0.0
        share_str = "<0.1%" if 0.0 < share_pct < 0.1 else f"{share_pct:.1f}%"
        tool_rows.append(
            f'<tr>'
            f'<td>'
            f'  <div class="tool-name-wrap"><code>{html.escape(t_name)}</code> <span class="text-muted" style="font-size: 0.75rem;">({share_str})</span></div>'
            f'  <div class="row-track"><div class="row-track-fill" style="width: {pct:.1f}%;"></div></div>'
            f'</td>'
            f'<td class="text-right num-col">{calls}</td>'
            f'<td class="text-right num-col text-muted">{p50_t}</td>'
            f'</tr>'
        )
    tools_tbody = "\n".join(tool_rows) if tool_rows else "<tr><td colspan='3' class='empty-cell'>No tool calls recorded</td></tr>"

    # Recent queries rows
    recent_rows = []
    for idx, q in enumerate(metrics.get("recent_queries", [])):
        ts = q.get("ts", 0)
        time_str = time.strftime("%H:%M:%S", time.localtime(ts)) if ts else "—"
        tool = html.escape(str(q.get("tool", "—")))
        r_name = html.escape(str(q.get("repo_name") or "workspace"))
        shown = q.get("shown", q.get("result_count", 0))
        lat_us = q.get("latency_us", 0)
        lat_str = f"{lat_us / 1000.0:.1f}ms" if lat_us is not None else "—"
        exit_code = int(q.get("exit_code", 0))
        if exit_code == 0:
            status = "ok"
            status_class = "status-ok"
        elif exit_code == 1:
            status = "empty"
            status_class = "status-warn"
        elif exit_code == 2:
            status = "ambig"
            status_class = "status-ambig"
        else:
            status = f"err {exit_code}"
            status_class = "status-err"

        args_str = json.dumps(q.get("args", {}), ensure_ascii=False)
        args_short = args_str if len(args_str) <= 80 else args_str[:77] + "..."
        args_escaped = html.escape(args_short)
        args_full = html.escape(args_str)

        recent_rows.append(
            f'<tr class="query-row" data-repo="{r_name}" data-code="{exit_code}">'
            f'<td class="num-col text-muted">{time_str}</td>'
            f'<td><span class="repo-tag">{r_name}</span></td>'
            f'<td><code>{tool}</code></td>'
            f'<td class="text-right num-col">{shown}</td>'
            f'<td class="text-right num-col">{lat_str}</td>'
            f'<td><span class="badge {status_class}">{status}</span></td>'
            f'<td class="args-cell" title="{args_full}"><code>{args_escaped}</code></td>'
            f'</tr>'
        )
    recent_tbody = "\n".join(recent_rows) if recent_rows else "<tr><td colspan='7' class='empty-cell'>No recent queries</td></tr>"

    # Triage error rows
    error_rows = []
    for e in metrics.get("recent_errors", []):
        ts = e.get("ts", 0)
        time_str = time.strftime("%H:%M:%S", time.localtime(ts)) if ts else "—"
        tool = html.escape(str(e.get("tool", "—")))
        r_name = html.escape(str(e.get("repo_name") or "workspace"))
        exit_code = int(e.get("exit_code", 0))
        lat_us = e.get("latency_us", 0)
        lat_str = f"{lat_us / 1000.0:.1f}ms" if lat_us is not None else "—"
        args_str = html.escape(json.dumps(e.get("args", {}), ensure_ascii=False))
        error_rows.append(
            f'<tr>'
            f'<td class="num-col text-muted">{time_str}</td>'
            f'<td><span class="repo-tag">{r_name}</span></td>'
            f'<td><code>{tool}</code></td>'
            f'<td><span class="badge status-err">Exit {exit_code}</span></td>'
            f'<td class="text-right num-col">{lat_str}</td>'
            f'<td class="args-cell"><code>{args_str}</code></td>'
            f'</tr>'
        )
    errors_tbody = "\n".join(error_rows) if error_rows else "<tr><td colspan='6' class='empty-cell'>Zero critical errors recorded</td></tr>"

    # Memory Counters
    active_memory_count = sum(1 for m in memories if m.get("is_current", True))
    total_memories = len(memories)
    superseded_count = total_memories - active_memory_count
    decisions_count = sum(1 for m in memories if m.get("is_current", True) and m.get("kind") == "decision")
    gotchas_count = sum(1 for m in memories if m.get("is_current", True) and m.get("kind") == "gotcha")
    facts_count = sum(1 for m in memories if m.get("is_current", True) and m.get("kind") == "fact")
    tasks_count = sum(1 for m in memories if m.get("is_current", True) and m.get("kind") == "task")
    total_recalls = sum(m.get("recall_count", 0) for m in memories)
    verified_refs_count = sum(sum(1 for r in m.get("resolved_refs", []) if r.get("verified")) for m in memories)

    # Reuse Counters
    total_candidates = reuse_summary.get("total_candidates", len(reuse_candidates))
    preventable_loc = reuse_summary.get("preventable_loc", sum(c.get("loc_saving", 0) for c in reuse_candidates))
    prewrite_count = reuse_summary.get("prewrite_count", len(prewrite_searches))
    avg_callers = reuse_summary.get("avg_callers", 0.0)

    header_title = "aw global telemetry dashboard" if is_global else f"aw telemetry dashboard — {html.escape(scope_name)}"
    sub_title = (
        f"Monitored across {repo_count} active repositories/worktrees · Source: ~/.aw/usage.log"
        if is_global
        else f"Repository: {html.escape(scope_name)} · Index: {index_stats.get('files', 0)} files, {index_stats.get('symbols', 0)} symbols"
    )

    memories_json = json.dumps(memories, ensure_ascii=False)
    recent_queries_json = json.dumps(metrics.get("recent_queries", []), ensure_ascii=False)
    reuse_candidates_json = json.dumps(reuse_candidates, ensure_ascii=False)
    prewrite_searches_json = json.dumps(prewrite_searches, ensure_ascii=False)

    return f"""<!DOCTYPE html>
<html lang="en" data-theme="light">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{header_title}</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Fraunces:opsz,wght@9..144,400&family=JetBrains+Mono:wght@400;500;600&family=Plus+Jakarta+Sans:wght@400;500;600&display=swap" rel="stylesheet">
<style>
  :root {{
    --bg-canvas: #F7F4EE;
    --bg-surface: #FBF9F5;
    --bg-subtle: #EFECE4;
    --bg-elevated: #FFFFFF;
    --text-primary: #1E1C1A;
    --text-secondary: #4A4640;
    --text-muted: #666059;
    --border-subtle: rgba(30, 28, 26, 0.09);
    --border-strong: rgba(30, 28, 26, 0.18);
    --intent-primary: #9E583B;
    --intent-primary-soft: rgba(158, 88, 59, 0.10);
    --intent-success: #246A49;
    --intent-success-soft: rgba(36, 106, 73, 0.10);
    --intent-warning: #A86408;
    --intent-warning-soft: rgba(168, 100, 8, 0.10);
    --intent-danger: #B8322C;
    --intent-danger-soft: rgba(184, 50, 44, 0.10);
    --intent-info: #2E5A70;
    --intent-info-soft: rgba(46, 90, 112, 0.10);
    --font-display: 'Fraunces', Georgia, serif;
    --font-sans: 'Plus Jakarta Sans', -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    --font-mono: 'JetBrains Mono', ui-monospace, SFMono-Regular, monospace;
    --shadow-card: 0 1px 3px rgba(30, 28, 26, 0.04), 0 1px 2px rgba(30, 28, 26, 0.02);
  }}

  :root[data-theme="dark"] {{
    --bg-canvas: #121110;
    --bg-surface: #1A1816;
    --bg-subtle: #24211E;
    --bg-elevated: #2A2724;
    --text-primary: #F3EFEA;
    --text-secondary: #C4BDB3;
    --text-muted: #968E84;
    --border-subtle: rgba(243, 239, 234, 0.09);
    --border-strong: rgba(243, 239, 234, 0.18);
    --intent-primary: #D9825B;
    --intent-primary-soft: rgba(217, 130, 91, 0.15);
    --intent-success: #52B788;
    --intent-success-soft: rgba(82, 183, 136, 0.15);
    --intent-warning: #E9A84A;
    --intent-warning-soft: rgba(233, 168, 74, 0.15);
    --intent-danger: #F06A63;
    --intent-danger-soft: rgba(240, 106, 99, 0.15);
    --intent-info: #6BA3BE;
    --intent-info-soft: rgba(107, 163, 190, 0.15);
    --shadow-card: 0 1px 3px rgba(0, 0, 0, 0.25);
  }}

  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{
    background: var(--bg-canvas);
    color: var(--text-primary);
    font-family: var(--font-sans);
    line-height: 1.5;
    padding: 2rem 1.5rem;
    -webkit-font-smoothing: antialiased;
  }}

  .container {{ max-width: 1320px; margin: 0 auto; }}

  /* Top Navigation & Header */
  header {{
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding-bottom: 1.25rem;
    border-bottom: 1px solid var(--border-subtle);
    margin-bottom: 1.5rem;
    flex-wrap: wrap;
    gap: 1rem;
  }}
  .header-left {{ display: flex; align-items: baseline; gap: 0.75rem; flex-wrap: wrap; }}
  h1 {{
    font-family: var(--font-display);
    font-size: 1.65rem;
    font-weight: 400;
    letter-spacing: -0.02em;
    color: var(--text-primary);
  }}
  .header-meta {{ color: var(--text-muted); font-size: 0.85rem; }}
  .header-right {{ display: flex; align-items: center; gap: 0.75rem; }}

  .theme-btn {{
    background: var(--bg-surface);
    border: 1px solid var(--border-strong);
    color: var(--text-primary);
    padding: 0.35rem 0.75rem;
    border-radius: 6px;
    font-family: var(--font-mono);
    font-size: 0.75rem;
    cursor: pointer;
    transition: background 0.15s, border-color 0.15s;
  }}
  .theme-btn:hover {{ background: var(--bg-subtle); }}

  /* Navigation Tabs */
  .nav-tabs {{
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 1.75rem;
    border-bottom: 1px solid var(--border-subtle);
    padding-bottom: 0.5rem;
    flex-wrap: wrap;
  }}
  .tab-btn {{
    background: transparent;
    border: none;
    color: var(--text-muted);
    font-family: var(--font-sans);
    font-size: 0.92rem;
    font-weight: 500;
    padding: 0.5rem 0.9rem;
    border-radius: 6px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 0.45rem;
    transition: color 0.15s, background 0.15s;
  }}
  .tab-btn:hover {{ color: var(--text-primary); background: var(--bg-subtle); }}
  .tab-btn.active {{
    color: var(--intent-primary);
    background: var(--intent-primary-soft);
    font-weight: 600;
  }}
  .tab-num {{
    font-family: var(--font-display);
    font-size: 0.95rem;
    opacity: 0.75;
  }}
  .tab-badge {{
    font-family: var(--font-mono);
    font-size: 0.7rem;
    padding: 0.1rem 0.45rem;
    border-radius: 999px;
    background: var(--border-strong);
    color: var(--text-primary);
  }}

  /* KPI Strip (Unified Card with Hairline Dividers) */
  .kpi-strip {{
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: 12px;
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    box-shadow: var(--shadow-card);
    margin-bottom: 1.75rem;
    overflow: hidden;
  }}
  @media (max-width: 860px) {{
    .kpi-strip {{ grid-template-columns: repeat(2, 1fr); }}
  }}
  @media (max-width: 480px) {{
    .kpi-strip {{ grid-template-columns: 1fr; }}
  }}
  .kpi-tile {{
    padding: 1.25rem 1.4rem;
    border-right: 1px solid var(--border-subtle);
  }}
  .kpi-tile:last-child {{ border-right: none; }}
  .kpi-label {{
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
    margin-bottom: 0.35rem;
  }}
  .kpi-value {{
    font-family: var(--font-mono);
    font-size: 1.85rem;
    font-weight: 600;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
    line-height: 1.2;
  }}
  .kpi-sub {{
    font-size: 0.8rem;
    color: var(--text-muted);
    margin-top: 0.3rem;
  }}

  /* Section Containers & Cards */
  .panel-grid {{
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1.5rem;
    margin-bottom: 1.75rem;
  }}
  @media (max-width: 900px) {{ .panel-grid {{ grid-template-columns: 1fr; }} }}

  .card {{
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: 12px;
    padding: 1.25rem 1.4rem;
    box-shadow: var(--shadow-card);
    margin-bottom: 1.75rem;
  }}
  .card-title {{
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: 0.9rem;
    display: flex;
    justify-content: space-between;
    align-items: center;
  }}

  /* Tables & Progress Tracks */
  table {{
    width: 100%;
    border-collapse: collapse;
    font-size: 0.85rem;
  }}
  th, td {{
    padding: 0.65rem 0.75rem;
    border-bottom: 1px solid var(--border-subtle);
    text-align: left;
    vertical-align: middle;
  }}
  th {{
    font-size: 0.7rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
  }}
  tr.repo-row-item {{ cursor: pointer; }}
  tr:hover td {{ background: var(--bg-subtle); }}

  .row-track {{
    background: var(--border-subtle);
    height: 3px;
    border-radius: 2px;
    overflow: hidden;
    margin-top: 0.35rem;
    width: 100%;
  }}
  .row-track-fill {{
    background: var(--intent-primary);
    height: 100%;
  }}

  .num-col {{ font-family: var(--font-mono); font-variant-numeric: tabular-nums; }}
  .text-right {{ text-align: right; }}
  .text-muted {{ color: var(--text-muted); }}
  .text-success {{ color: var(--intent-success); }}

  /* Status Badges */
  .badge {{
    display: inline-block;
    padding: 0.15rem 0.45rem;
    border-radius: 4px;
    font-family: var(--font-mono);
    font-size: 0.72rem;
    font-weight: 500;
  }}
  .status-ok {{ background: var(--intent-success-soft); color: var(--intent-success); }}
  .status-warn {{ background: var(--intent-warning-soft); color: var(--intent-warning); }}
  .status-ambig {{ background: var(--intent-info-soft); color: var(--intent-info); }}
  .status-err {{ background: var(--intent-danger-soft); color: var(--intent-danger); }}
  .badge-repo {{ background: var(--bg-subtle); color: var(--text-secondary); }}
  .badge-wt {{ background: var(--intent-primary-soft); color: var(--intent-primary); }}

  .badge-d1 {{ background: var(--intent-danger-soft); color: var(--intent-danger); }}
  .badge-d2 {{ background: var(--intent-warning-soft); color: var(--intent-warning); }}
  .badge-d3 {{ background: var(--intent-primary-soft); color: var(--intent-primary); }}
  .badge-d4 {{ background: var(--intent-info-soft); color: var(--intent-info); }}

  .repo-tag {{
    font-family: var(--font-mono);
    font-size: 0.75rem;
    color: var(--text-secondary);
    background: var(--bg-subtle);
    padding: 0.1rem 0.35rem;
    border-radius: 3px;
  }}

  .args-cell {{
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-secondary);
  }}
  .args-cell code {{
    font-family: var(--font-mono);
    font-size: 0.75rem;
  }}

  /* Filter Controls */
  .filter-bar {{
    display: flex;
    gap: 0.75rem;
    align-items: center;
    margin-bottom: 1rem;
    flex-wrap: wrap;
  }}
  .filter-select, .search-input {{
    background: var(--bg-elevated);
    border: 1px solid var(--border-strong);
    color: var(--text-primary);
    padding: 0.4rem 0.7rem;
    border-radius: 6px;
    font-family: var(--font-sans);
    font-size: 0.82rem;
  }}
  .search-input {{ min-width: 260px; flex: 1; }}

  .pill-group {{
    display: inline-flex;
    gap: 0.35rem;
    align-items: center;
  }}
  .filter-pill {{
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    color: var(--text-muted);
    font-family: var(--font-mono);
    font-size: 0.75rem;
    padding: 0.25rem 0.6rem;
    border-radius: 999px;
    cursor: pointer;
    transition: all 0.15s;
  }}
  .filter-pill:hover {{ background: var(--bg-subtle); color: var(--text-primary); }}
  .filter-pill.active {{
    background: var(--intent-primary-soft);
    color: var(--intent-primary);
    border-color: var(--intent-primary);
    font-weight: 600;
  }}

  /* Master-Detail Split Pane Layout */
  .master-detail-split {{
    display: grid;
    grid-template-columns: 5fr 7fr;
    gap: 1.5rem;
    align-items: start;
    margin-bottom: 1.75rem;
  }}
  @media (max-width: 960px) {{
    .master-detail-split {{ grid-template-columns: 1fr; }}
  }}

  .list-pane {{
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    max-height: 820px;
    overflow-y: auto;
    padding-right: 0.35rem;
  }}

  .dossier-pane {{
    position: sticky;
    top: 1.5rem;
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: 12px;
    padding: 1.35rem;
    box-shadow: var(--shadow-card);
    max-height: 860px;
    overflow-y: auto;
  }}

  .select-item-card {{
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: 8px;
    padding: 0.9rem 1rem;
    cursor: pointer;
    transition: border-color 0.15s, background 0.15s;
  }}
  .select-item-card:hover {{ background: var(--bg-subtle); }}
  .select-item-card.selected {{
    border-color: var(--intent-primary);
    background: var(--intent-primary-soft);
  }}

  .item-card-head {{
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 0.35rem;
  }}
  .item-key {{
    font-family: var(--font-mono);
    font-size: 0.88rem;
    font-weight: 600;
    color: var(--text-primary);
    word-break: break-all;
  }}
  .item-snippet {{
    font-size: 0.82rem;
    color: var(--text-secondary);
    margin-top: 0.3rem;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }}
  .item-meta {{
    font-size: 0.75rem;
    color: var(--text-muted);
    margin-top: 0.45rem;
    display: flex;
    gap: 0.6rem;
    align-items: center;
    flex-wrap: wrap;
  }}

  /* Dossier Elements */
  .dossier-head {{
    border-bottom: 1px solid var(--border-subtle);
    padding-bottom: 0.9rem;
    margin-bottom: 1rem;
  }}
  .dossier-title {{
    font-family: var(--font-mono);
    font-size: 1.05rem;
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: 0.25rem;
    word-break: break-all;
  }}
  .prevention-banner {{
    background: var(--intent-primary-soft);
    border-left: 3px solid var(--intent-primary);
    padding: 0.75rem 1rem;
    border-radius: 0 6px 6px 0;
    margin-bottom: 1rem;
    font-size: 0.85rem;
    line-height: 1.45;
  }}
  .recall-banner {{
    background: var(--intent-success-soft);
    border-left: 3px solid var(--intent-success);
    padding: 0.65rem 0.9rem;
    border-radius: 0 6px 6px 0;
    margin-bottom: 1rem;
    font-size: 0.82rem;
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 0.5rem;
  }}
  .dossier-section-title {{
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
    margin-top: 1rem;
    margin-bottom: 0.45rem;
  }}
  .dossier-body {{
    font-family: var(--font-sans);
    font-size: 0.9rem;
    line-height: 1.6;
    color: var(--text-primary);
    white-space: pre-wrap;
    background: var(--bg-canvas);
    border: 1px solid var(--border-subtle);
    border-radius: 8px;
    padding: 0.9rem 1rem;
  }}

  /* Side-by-Side Source Code Panels */
  .side-by-side-grid {{
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1rem;
    margin-top: 0.5rem;
  }}
  @media (max-width: 860px) {{
    .side-by-side-grid {{ grid-template-columns: 1fr; }}
  }}
  .code-panel {{
    background: var(--bg-canvas);
    border: 1px solid var(--border-subtle);
    border-radius: 8px;
    padding: 0.75rem 0.9rem;
    font-family: var(--font-mono);
    font-size: 0.76rem;
    overflow-x: auto;
    max-height: 380px;
  }}
  .code-panel-head {{
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding-bottom: 0.4rem;
    margin-bottom: 0.5rem;
    border-bottom: 1px solid var(--border-subtle);
    font-size: 0.74rem;
    font-weight: 600;
  }}

  .chips-row {{ display: flex; gap: 0.4rem; flex-wrap: wrap; }}
  .chip {{
    font-family: var(--font-mono);
    font-size: 0.75rem;
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
  }}
  .chip-tag {{ background: var(--intent-primary-soft); color: var(--intent-primary); }}
  .chip-ref {{ background: var(--bg-subtle); color: var(--text-secondary); border: 1px solid var(--border-subtle); }}
  .chip-ref.verified {{ border-color: var(--intent-success); color: var(--intent-success); }}

  .empty-cell {{
    text-align: center;
    padding: 2.5rem 1rem;
    color: var(--text-muted);
    font-size: 0.85rem;
  }}

  .hidden {{ display: none !important; }}
  .live-dot {{
    display: inline-block;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--intent-success);
    margin-right: 0.4rem;
    animation: pulse 2s infinite;
  }}
  @keyframes pulse {{
    0% {{ opacity: 1; transform: scale(1); }}
    50% {{ opacity: 0.4; transform: scale(0.9); }}
    100% {{ opacity: 1; transform: scale(1); }}
  }}
</style>
</head>
<body>
<div class="container">
  <header>
    <div class="header-left">
      <h1>{header_title}</h1>
      <span class="header-meta">{sub_title}</span>
    </div>
    <div class="header-right">
      <span id="live-indicator" class="badge status-ok" style="display: none;"><span class="live-dot"></span>LIVE</span>
      <button class="theme-btn" id="theme-toggle-btn" onclick="toggleTheme()">Theme: Light</button>
    </div>
  </header>

  <!-- Primary Tab Bar (3 Dedicated Tabs) -->
  <nav class="nav-tabs" role="tablist">
    <button class="tab-btn active" id="tab-telemetry-btn" onclick="switchTab('telemetry')">
      <span class="tab-num">01</span> Telemetry &amp; Operations
    </button>
    <button class="tab-btn" id="tab-reuse-btn" onclick="switchTab('reuse')">
      <span class="tab-num">02</span> Anti-Reinvention Inspector
      <span class="tab-badge" id="nav-reuse-count">{total_candidates}</span>
    </button>
    <button class="tab-btn" id="tab-memory-btn" onclick="switchTab('memory')">
      <span class="tab-num">03</span> Agent Memory Inspector
      <span class="tab-badge" id="nav-mem-count">{active_memory_count}</span>
    </button>
  </nav>

  <!-- ==================== TAB 01: TELEMETRY & OPERATIONS ==================== -->
  <div id="tab-telemetry-content">
    <div class="kpi-strip">
      <div class="kpi-tile">
        <div class="kpi-label">Total Queries</div>
        <div id="metric-total-calls" class="kpi-value">{total_calls}</div>
        <div id="metric-reliability" class="kpi-sub">{metrics['reliability_pct']:.1f}% clean ({total_errors} errs)</div>
      </div>
      <div class="kpi-tile">
        <div class="kpi-label">Median Latency (p50)</div>
        <div id="metric-p50" class="kpi-value text-success">{p50_str}</div>
        <div class="kpi-sub">p95 = {p95_str} · avg = {metrics['avg_ms']:.1f}ms</div>
      </div>
      <div class="kpi-tile">
        <div class="kpi-label">Est. Tokens Saved</div>
        <div id="metric-tokens" class="kpi-value">~{tokens_str}</div>
        <div class="kpi-sub">~{dollars_str} avoided vs full reads</div>
      </div>
      <div class="kpi-tile">
        <div class="kpi-label">Anti-Reinvention &amp; Memory</div>
        <div class="kpi-value">{metrics['reuse_hits']} <span style="font-size: 0.9rem; color: var(--text-muted); font-weight: normal;">hits</span></div>
        <div class="kpi-sub">{active_memory_count} memories across stores</div>
      </div>
    </div>

    <div class="panel-grid">
      <!-- Repositories & Worktrees Breakdown -->
      <div class="card">
        <div class="card-title">
          <span>Active Repositories &amp; Worktrees</span>
          <span class="text-muted" style="font-size: 0.75rem; font-weight: normal;">Click row to filter</span>
        </div>
        <table>
          <thead>
            <tr>
              <th>Repository / Branch</th>
              <th class="text-right">Queries</th>
              <th>Status</th>
              <th class="text-right">Last Active</th>
            </tr>
          </thead>
          <tbody id="repos-tbody">
            {repos_tbody}
          </tbody>
        </table>
      </div>

      <!-- Tool Distribution -->
      <div class="card">
        <div class="card-title">
          <span>Tool Distribution &amp; Latency</span>
          <span class="text-muted" style="font-size: 0.75rem; font-weight: normal;">Volume &amp; p50</span>
        </div>
        <table>
          <thead>
            <tr>
              <th>Tool Command</th>
              <th class="text-right">Calls</th>
              <th class="text-right">p50 Latency</th>
            </tr>
          </thead>
          <tbody id="tools-tbody">
            {tools_tbody}
          </tbody>
        </table>
      </div>
    </div>

    <!-- Live Query Stream -->
    <div class="card">
      <div class="card-title">
        <span>Recent Query Activity</span>
        <span class="text-muted" style="font-size: 0.75rem; font-weight: normal;">Streaming via /events</span>
      </div>

      <div class="filter-bar">
        <label for="repo-filter" class="text-muted" style="font-size: 0.78rem;">Scope:</label>
        <select id="repo-filter" class="filter-select" onchange="applyTelemetryFilters()">
          {repo_options_html}
        </select>

        <label for="status-filter" class="text-muted" style="font-size: 0.78rem;">Status:</label>
        <select id="status-filter" class="filter-select" onchange="applyTelemetryFilters()">
          <option value="ALL">All Statuses</option>
          <option value="0">OK (Exit 0)</option>
          <option value="1">Empty (Exit 1)</option>
          <option value="2">Ambiguous (Exit 2)</option>
          <option value="ERR">Errors (Exit >= 3)</option>
        </select>

        <input type="text" id="query-search" class="search-input" placeholder="Search tool or argument..." oninput="applyTelemetryFilters()">
      </div>

      <table>
        <thead>
          <tr>
            <th>Time</th>
            <th>Repository</th>
            <th>Tool</th>
            <th class="text-right">Items</th>
            <th class="text-right">Latency</th>
            <th>Status</th>
            <th>Arguments</th>
          </tr>
        </thead>
        <tbody id="recent-tbody">
          {recent_tbody}
        </tbody>
      </table>
    </div>

    <!-- Triage Log -->
    <div class="card">
      <div class="card-title">
        <span>Critical Error &amp; Bug Triage</span>
        <span class="text-muted" style="font-size: 0.75rem; font-weight: normal;">Exit Code >= 3</span>
      </div>
      <table>
        <thead>
          <tr>
            <th>Time</th>
            <th>Repository</th>
            <th>Tool</th>
            <th>Exit Code</th>
            <th class="text-right">Latency</th>
            <th>Arguments</th>
          </tr>
        </thead>
        <tbody id="errors-tbody">
          {errors_tbody}
        </tbody>
      </table>
    </div>
  </div>

  <!-- ==================== TAB 02: ANTI-REINVENTION INSPECTOR ==================== -->
  <div id="tab-reuse-content" class="hidden">
    <!-- Prevention KPI Strip -->
    <div class="kpi-strip">
      <div class="kpi-tile">
        <div class="kpi-label">Duplicate Candidates Caught</div>
        <div class="kpi-value text-success" id="reuse-kpi-count">{total_candidates}</div>
        <div class="kpi-sub">D1–D4 detectors across code index</div>
      </div>
      <div class="kpi-tile">
        <div class="kpi-label">Preventable Excess LOC</div>
        <div class="kpi-value" id="reuse-kpi-loc">~{preventable_loc:,}</div>
        <div class="kpi-sub">Redundant lines saved via consolidation</div>
      </div>
      <div class="kpi-tile">
        <div class="kpi-label">Pre-Write Helper Searches</div>
        <div class="kpi-value">{prewrite_count}</div>
        <div class="kpi-sub">aw search queries finding [callers=N] helpers</div>
      </div>
      <div class="kpi-tile">
        <div class="kpi-label">Canonical Concentration</div>
        <div class="kpi-value">{avg_callers} <span style="font-size: 0.85rem; color: var(--text-muted);">callers</span></div>
        <div class="kpi-sub">Authoritative reuse targets</div>
      </div>
    </div>

    <!-- Filter Bar -->
    <div class="filter-bar">
      <label for="reuse-repo-filter" class="text-muted" style="font-size: 0.78rem;">Repo:</label>
      <select id="reuse-repo-filter" class="filter-select" onchange="renderReuseList()">
        <option value="ALL">All Repositories</option>
      </select>

      <span class="text-muted" style="font-size: 0.78rem;">Detector:</span>
      <div class="pill-group">
        <button class="filter-pill active" data-det="ALL" onclick="setReuseDetectorFilter('ALL')">All</button>
        <button class="filter-pill" data-det="D1" onclick="setReuseDetectorFilter('D1')">D1 Callee</button>
        <button class="filter-pill" data-det="D2" onclick="setReuseDetectorFilter('D2')">D2 Interface</button>
        <button class="filter-pill" data-det="D3" onclick="setReuseDetectorFilter('D3')">D3 Bypass</button>
        <button class="filter-pill" data-det="D4" onclick="setReuseDetectorFilter('D4')">D4 Wrapper</button>
      </div>

      <input type="text" id="reuse-search" class="search-input" placeholder="Search candidate, canonical, or evidence..." oninput="renderReuseList()">
    </div>

    <!-- Master-Detail Split Pane -->
    <div class="master-detail-split">
      <!-- Master List -->
      <div class="list-pane" id="reuse-cards-container">
        <!-- Rendered dynamically via JS -->
      </div>

      <!-- Detail Dossier with Side-by-Side Code Viewer -->
      <div class="dossier-pane" id="reuse-dossier-container">
        <!-- Rendered dynamically via JS -->
      </div>
    </div>

    <!-- Pre-Write Helper Discovery Feed -->
    <div class="card" style="margin-top: 1.5rem;">
      <div class="card-title">
        <span>Pre-Write Helper Discovery Stream (`aw search` &amp; `aw reuse`)</span>
        <span class="text-muted" style="font-size: 0.75rem; font-weight: normal;">Searches before writing code</span>
      </div>
      <table>
        <thead>
          <tr>
            <th>Time</th>
            <th>Repository</th>
            <th>Tool</th>
            <th>Query / Target</th>
            <th class="text-right">Hits Returned</th>
            <th class="text-right">Latency</th>
          </tr>
        </thead>
        <tbody id="prewrite-tbody">
          <!-- Rendered dynamically via JS -->
        </tbody>
      </table>
    </div>
  </div>

  <!-- ==================== TAB 03: AGENT MEMORY INSPECTOR ==================== -->
  <div id="tab-memory-content" class="hidden">
    <!-- Memory KPI Strip -->
    <div class="kpi-strip">
      <div class="kpi-tile">
        <div class="kpi-label">Active Memories</div>
        <div class="kpi-value text-success" id="mem-kpi-active">{active_memory_count}</div>
        <div class="kpi-sub">Across all connected stores</div>
      </div>
      <div class="kpi-tile">
        <div class="kpi-label">Agent Recalls</div>
        <div class="kpi-value text-success">{total_recalls} <span style="font-size: 0.85rem; color: var(--text-muted);">queries</span></div>
        <div class="kpi-sub">Correlated from mem_query / mem_get</div>
      </div>
      <div class="kpi-tile">
        <div class="kpi-label">Bugs &amp; Regressions Guarded</div>
        <div class="kpi-value">{gotchas_count + decisions_count} <span style="font-size: 0.85rem; color: var(--text-muted);">rules</span></div>
        <div class="kpi-sub">{gotchas_count} gotchas · {decisions_count} decisions</div>
      </div>
      <div class="kpi-tile">
        <div class="kpi-label">Code References Guarded</div>
        <div class="kpi-value">{verified_refs_count} <span style="font-size: 0.85rem; color: var(--text-muted);">symbols</span></div>
        <div class="kpi-sub">Verified in repository index</div>
      </div>
    </div>

    <!-- Memory Filter Bar -->
    <div class="filter-bar">
      <label for="mem-store-filter" class="text-muted" style="font-size: 0.78rem;">Store:</label>
      <select id="mem-store-filter" class="filter-select" onchange="renderMemoryList()">
        <option value="ALL">All Stores</option>
      </select>

      <label for="mem-kind-filter" class="text-muted" style="font-size: 0.78rem;">Kind:</label>
      <select id="mem-kind-filter" class="filter-select" onchange="renderMemoryList()">
        <option value="ALL">All Kinds</option>
        <option value="decision">Decision</option>
        <option value="gotcha">Gotcha</option>
        <option value="fact">Fact</option>
        <option value="task">Task</option>
      </select>

      <input type="text" id="mem-search" class="search-input" placeholder="Search key, prose body, #tag or ref..." oninput="renderMemoryList()">

      <label style="font-size: 0.78rem; display: flex; align-items: center; gap: 0.35rem; cursor: pointer;">
        <input type="checkbox" id="mem-show-superseded" onchange="renderMemoryList()">
        <span class="text-muted">Show superseded revs</span>
      </label>
    </div>

    <!-- Master-Detail Split Pane -->
    <div class="master-detail-split">
      <!-- Master List -->
      <div class="list-pane" id="memory-cards-container">
        <!-- Rendered dynamically via JS -->
      </div>

      <!-- Detail Dossier -->
      <div class="dossier-pane" id="memory-dossier-container">
        <!-- Rendered dynamically via JS -->
      </div>
    </div>
  </div>
</div>

<script>
// Data Seeded From Server
let allMemories = {memories_json};
let recentQueries = {recent_queries_json};
let reuseCandidates = {reuse_candidates_json};
let prewriteSearches = {prewrite_searches_json};
let totalCalls = {total_calls};
let totalErrors = {total_errors};

let selectedMemoryIndex = 0;
let selectedReuseIndex = 0;
let activeReuseDetector = 'ALL';

// Theme Toggle
function initTheme() {{
  const saved = localStorage.getItem('aw-theme') || 'light';
  document.documentElement.setAttribute('data-theme', saved);
  updateThemeButtonText(saved);
}}

function toggleTheme() {{
  const current = document.documentElement.getAttribute('data-theme') || 'light';
  const next = current === 'light' ? 'dark' : 'light';
  document.documentElement.setAttribute('data-theme', next);
  localStorage.setItem('aw-theme', next);
  updateThemeButtonText(next);
}}

function updateThemeButtonText(theme) {{
  const btn = document.getElementById('theme-toggle-btn');
  if (btn) btn.textContent = 'Theme: ' + (theme === 'dark' ? 'Dark' : 'Light');
}}

// Tab Switching
function switchTab(tab) {{
  const tabs = ['telemetry', 'reuse', 'memory'];
  tabs.forEach(t => {{
    const btn = document.getElementById('tab-' + t + '-btn');
    const content = document.getElementById('tab-' + t + '-content');
    if (t === tab) {{
      if (btn) btn.classList.add('active');
      if (content) content.classList.remove('hidden');
    }} else {{
      if (btn) btn.classList.remove('active');
      if (content) content.classList.add('hidden');
    }}
  }});

  if (tab === 'reuse') {{
    populateReuseRepoFilter();
    renderReuseList();
    renderPrewriteTable();
  }} else if (tab === 'memory') {{
    populateMemoryStoreFilter();
    renderMemoryList();
  }}
}}

// Telemetry Query Filter
function applyTelemetryFilters() {{
  const selectedRepo = document.getElementById('repo-filter').value;
  const selectedStatus = document.getElementById('status-filter').value;
  const search = document.getElementById('query-search').value.toLowerCase().trim();

  const queryRows = document.querySelectorAll('#recent-tbody tr.query-row');
  queryRows.forEach(row => {{
    const repo = row.getAttribute('data-repo');
    const code = row.getAttribute('data-code');
    const text = row.textContent.toLowerCase();

    let matchesRepo = (selectedRepo === 'ALL' || repo === selectedRepo);
    let matchesStatus = true;
    if (selectedStatus === '0') matchesStatus = (code === '0');
    else if (selectedStatus === '1') matchesStatus = (code === '1');
    else if (selectedStatus === '2') matchesStatus = (code === '2');
    else if (selectedStatus === 'ERR') matchesStatus = (parseInt(code, 10) >= 3);

    let matchesSearch = (!search || text.includes(search));
    row.style.display = (matchesRepo && matchesStatus && matchesSearch) ? '' : 'none';
  }});
}}

function filterByRepo(repoName) {{
  const select = document.getElementById('repo-filter');
  if (select) {{
    select.value = repoName;
    applyTelemetryFilters();
  }}
}}

// ==================== TAB 02: ANTI-REINVENTION LOGIC ====================
function populateReuseRepoFilter() {{
  const select = document.getElementById('reuse-repo-filter');
  if (!select) return;
  const repos = new Set();
  reuseCandidates.forEach(c => repos.add(c.repo));

  select.innerHTML = '<option value="ALL">All Repositories</option>';
  Array.from(repos).sort().forEach(r => {{
    const opt = document.createElement('option');
    opt.value = r;
    opt.textContent = r;
    select.appendChild(opt);
  }});
}}

function setReuseDetectorFilter(det) {{
  activeReuseDetector = det;
  document.querySelectorAll('.filter-pill[data-det]').forEach(btn => {{
    btn.classList.toggle('active', btn.getAttribute('data-det') === det);
  }});
  selectedReuseIndex = 0;
  renderReuseList();
}}

function getFilteredReuseCandidates() {{
  const repo = document.getElementById('reuse-repo-filter') ? document.getElementById('reuse-repo-filter').value : 'ALL';
  const search = document.getElementById('reuse-search') ? document.getElementById('reuse-search').value.toLowerCase().trim() : '';

  return reuseCandidates.filter(c => {{
    if (repo !== 'ALL' && c.repo !== repo) return false;
    if (activeReuseDetector !== 'ALL' && c.detector !== activeReuseDetector) return false;
    if (search) {{
      const matchSym = c.symbol.toLowerCase().includes(search);
      const matchCan = c.canonical.toLowerCase().includes(search);
      const matchEv = (c.shared_evidence || '').toLowerCase().includes(search);
      const matchFile = (c.canonical_file || '').toLowerCase().includes(search);
      if (!matchSym && !matchCan && !matchEv && !matchFile) return false;
    }}
    return true;
  }});
}}

function renderReuseList() {{
  const container = document.getElementById('reuse-cards-container');
  if (!container) return;
  const filtered = getFilteredReuseCandidates();

  if (filtered.length === 0) {{
    container.innerHTML = '<div class="empty-cell">Zero duplicate candidates match current criteria.<br><span style="font-size: 0.8rem; opacity: 0.7;">Codebase is free from D1–D4 duplication.</span></div>';
    renderReuseDossier(null);
    return;
  }}

  container.innerHTML = '';
  filtered.forEach((c, idx) => {{
    const isSelected = idx === selectedReuseIndex;
    const card = document.createElement('div');
    card.className = 'select-item-card' + (isSelected ? ' selected' : '');
    card.onclick = () => {{
      selectedReuseIndex = idx;
      renderReuseList();
    }};

    const detBadgeClass = 'badge badge-' + c.detector.toLowerCase();
    const repoBadge = '<span class="badge badge-repo">' + escapeHtml(c.repo) + '</span>';
    const locBadge = '<span class="badge status-warn">-' + c.loc_saving + ' LOC</span>';

    card.innerHTML =
      '<div class="item-card-head">' +
      '  <div style="display:flex; align-items:center; gap:0.4rem;">' +
      '    <span class="' + detBadgeClass + '">' + c.detector + '</span>' +
      '    <span class="badge" style="background:var(--bg-subtle);">' + c.overlap_pct + '% overlap</span>' +
      '    ' + locBadge +
      '  </div>' +
      '  ' + repoBadge +
      '</div>' +
      '<div class="item-key">' + escapeHtml(c.symbol) + '</div>' +
      '<div class="item-snippet">&rarr; canonical: <code>' + escapeHtml(c.canonical) + '</code> <span class="badge status-ok">[callers=' + c.canonical_callers + ']</span></div>' +
      '<div class="item-meta">' +
      '  <span>' + escapeHtml(c.canonical_file) + ':' + c.canonical_line + '</span>' +
      (c.shared_evidence ? '<span>&bull;</span><span>' + escapeHtml(c.shared_evidence) + '</span>' : '') +
      '</div>';

    container.appendChild(card);
  }});

  if (filtered[selectedReuseIndex]) {{
    renderReuseDossier(filtered[selectedReuseIndex]);
  }} else {{
    selectedReuseIndex = 0;
    renderReuseDossier(filtered[0] || null);
  }}
}}

function renderReuseDossier(c) {{
  const container = document.getElementById('reuse-dossier-container');
  if (!container) return;
  if (!c) {{
    container.innerHTML = '<div class="empty-cell" style="padding-top: 5rem;">Select a duplication candidate to inspect side-by-side source code.</div>';
    return;
  }}

  const detBadgeClass = 'badge badge-' + c.detector.toLowerCase();

  // Side-by-side code blocks
  const symSnippet = c.candidate_snippet || '<span class="text-muted">(Source snippet unavailable)</span>';
  const canSnippet = c.canonical_snippet || '<span class="text-muted">(Source snippet unavailable)</span>';

  container.innerHTML =
    '<div class="dossier-head">' +
    '  <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom: 0.5rem;">' +
    '    <div style="display:flex; gap:0.45rem; align-items:center;">' +
    '      <span class="' + detBadgeClass + '">' + c.detector + '</span>' +
    '      <span class="badge status-ok">' + c.overlap_pct + '% OVERLAP</span>' +
    '      <span class="badge badge-repo">' + escapeHtml(c.repo) + '</span>' +
    '    </div>' +
    '    <span class="badge status-warn">Save ~' + c.loc_saving + ' LOC</span>' +
    '  </div>' +
    '  <div class="dossier-title">' + escapeHtml(c.symbol) + '</div>' +
    '  <div class="text-muted" style="font-size:0.82rem;">Duplicates Authoritative Target: <strong>' + escapeHtml(c.canonical) + '</strong> <span class="badge status-ok">[callers=' + c.canonical_callers + ']</span></div>' +
    '</div>' +
    '<div class="prevention-banner">' +
    '  <strong>What This Prevents:</strong> ' + escapeHtml(c.prevention_narrative) +
    '</div>' +
    (c.shared_evidence ? '<div class="dossier-section-title">Shared Footprint &amp; Overlap Evidence</div><div class="chips-row"><span class="chip chip-tag">' + escapeHtml(c.shared_evidence) + '</span></div>' : '') +
    '<div class="dossier-section-title">Side-by-Side Source Code Comparison</div>' +
    '<div class="side-by-side-grid">' +
    '  <div class="code-panel">' +
    '    <div class="code-panel-head">' +
    '      <span class="badge status-warn">DUPLICATE CANDIDATE</span>' +
    '      <span>' + escapeHtml(c.candidate_info.file) + ':' + c.candidate_info.line_start + '-' + c.candidate_info.line_end + '</span>' +
    '    </div>' +
    '    <pre><code>' + escapeHtml(symSnippet) + '</code></pre>' +
    '  </div>' +
    '  <div class="code-panel">' +
    '    <div class="code-panel-head">' +
    '      <span class="badge status-ok">CANONICAL REUSE TARGET</span>' +
    '      <span>' + escapeHtml(c.canonical_info.file) + ':' + c.canonical_info.line_start + '-' + c.canonical_info.line_end + '</span>' +
    '    </div>' +
    '    <pre><code>' + escapeHtml(canSnippet) + '</code></pre>' +
    '  </div>' +
    '</div>';
}}

function renderPrewriteTable() {{
  const tbody = document.getElementById('prewrite-tbody');
  if (!tbody) return;
  if (!prewriteSearches || prewriteSearches.length === 0) {{
    tbody.innerHTML = '<tr><td colspan="6" class="empty-cell">No pre-write helper searches recorded yet</td></tr>';
    return;
  }}

  tbody.innerHTML = '';
  prewriteSearches.forEach(s => {{
    const tr = document.createElement('tr');
    const ts = s.ts ? new Date(s.ts * 1000).toTimeString().split(' ')[0] : '—';
    const latStr = s.latency_us ? (s.latency_us / 1000.0).toFixed(1) + 'ms' : '—';
    tr.innerHTML =
      '<td class="num-col text-muted">' + ts + '</td>' +
      '<td><span class="repo-tag">' + escapeHtml(s.repo_name) + '</span></td>' +
      '<td><code>' + escapeHtml(s.tool) + '</code></td>' +
      '<td class="args-cell"><code>' + escapeHtml(s.terms || '(all paths)') + '</code></td>' +
      '<td class="text-right num-col">' + s.shown + '</td>' +
      '<td class="text-right num-col">' + latStr + '</td>';
    tbody.appendChild(tr);
  }});
}}

// ==================== TAB 03: AGENT MEMORY LOGIC ====================
function populateMemoryStoreFilter() {{
  const select = document.getElementById('mem-store-filter');
  if (!select) return;
  const stores = new Set();
  allMemories.forEach(m => stores.add(m.store));

  select.innerHTML = '<option value="ALL">All Stores</option>';
  Array.from(stores).sort().forEach(s => {{
    const opt = document.createElement('option');
    opt.value = s;
    opt.textContent = s;
    select.appendChild(opt);
  }});
}}

function getFilteredMemories() {{
  const store = document.getElementById('mem-store-filter') ? document.getElementById('mem-store-filter').value : 'ALL';
  const kind = document.getElementById('mem-kind-filter') ? document.getElementById('mem-kind-filter').value : 'ALL';
  const search = document.getElementById('mem-search') ? document.getElementById('mem-search').value.toLowerCase().trim() : '';
  const showSuperseded = document.getElementById('mem-show-superseded') ? document.getElementById('mem-show-superseded').checked : false;

  return allMemories.filter(m => {{
    if (!showSuperseded && !m.is_current) return false;
    if (store !== 'ALL' && m.store !== store) return false;
    if (kind !== 'ALL' && m.kind !== kind) return false;
    if (search) {{
      const matchKey = m.key.toLowerCase().includes(search);
      const matchBody = m.body.toLowerCase().includes(search);
      const matchTags = (m.tags || []).some(t => t.toLowerCase().includes(search));
      const matchRefs = (m.refs || []).some(r => r.toLowerCase().includes(search));
      if (!matchKey && !matchBody && !matchTags && !matchRefs) return false;
    }}
    return true;
  }});
}}

function renderMemoryList() {{
  const container = document.getElementById('memory-cards-container');
  if (!container) return;
  const filtered = getFilteredMemories();

  if (filtered.length === 0) {{
    container.innerHTML = '<div class="empty-cell">No memory notes match the filter.<br><span style="font-size: 0.8rem; opacity: 0.7;">Agents write memories via <code>aw mem write --body "..." --kind gotcha &lt;key&gt;</code></span></div>';
    renderMemoryDossier(null);
    return;
  }}

  container.innerHTML = '';
  filtered.forEach((m, idx) => {{
    const isSelected = idx === selectedMemoryIndex;
    const card = document.createElement('div');
    card.className = 'select-item-card' + (isSelected ? ' selected' : '');
    card.onclick = () => {{
      selectedMemoryIndex = idx;
      renderMemoryList();
    }};

    const kindClass = 'badge status-' + (m.kind === 'decision' ? 'ambig' : m.kind === 'gotcha' ? 'warn' : m.kind === 'fact' ? 'ok' : 'err');
    const timeStr = m.created_at ? new Date(m.created_at * 1000).toLocaleDateString() : '—';
    const storeBadge = '<span class="badge badge-repo">' + escapeHtml(m.store) + '</span>';
    const revPill = '<span class="badge" style="background: var(--bg-subtle);">r' + m.rev + '</span>';
    const recallsBadge = m.recall_count > 0 ? '<span class="badge status-ok">' + m.recall_count + ' recalls</span>' : '';

    card.innerHTML =
      '<div class="item-card-head">' +
      '  <div style="display:flex; align-items:center; gap:0.4rem;">' +
      '    <span class="' + kindClass + '">' + m.kind.toUpperCase() + '</span>' +
      '    ' + revPill +
      '    ' + recallsBadge +
      '  </div>' +
      '  ' + storeBadge +
      '</div>' +
      '<div class="item-key">' + escapeHtml(m.key) + '</div>' +
      '<div class="item-snippet">' + escapeHtml(m.body) + '</div>' +
      '<div class="item-meta">' +
      '  <span>' + m.scope + '</span>' +
      '  <span>&bull;</span>' +
      '  <span>' + timeStr + '</span>' +
      (m.refs && m.refs.length ? '<span>&bull;</span><span>' + m.refs.length + ' refs</span>' : '') +
      '</div>';

    container.appendChild(card);
  }});

  if (filtered[selectedMemoryIndex]) {{
    renderMemoryDossier(filtered[selectedMemoryIndex]);
  }} else {{
    selectedMemoryIndex = 0;
    renderMemoryDossier(filtered[0] || null);
  }}
}}

function renderMemoryDossier(m) {{
  const container = document.getElementById('memory-dossier-container');
  if (!container) return;
  if (!m) {{
    container.innerHTML = '<div class="empty-cell" style="padding-top: 5rem;">Select a memory note from the list to view full dossier.</div>';
    return;
  }}

  const kindClass = 'badge status-' + (m.kind === 'decision' ? 'ambig' : m.kind === 'gotcha' ? 'warn' : m.kind === 'fact' ? 'ok' : 'err');
  const dateStr = m.created_at ? new Date(m.created_at * 1000).toLocaleString() : '—';
  const lastRecalledStr = m.last_recalled_ts ? new Date(m.last_recalled_ts * 1000).toLocaleString() : 'Never queried in current session';

  let tagsHtml = '';
  if (m.tags && m.tags.length) {{
    tagsHtml = '<div class="dossier-section-title">Tags</div><div class="chips-row">';
    m.tags.forEach(t => {{ tagsHtml += '<span class="chip chip-tag">#' + escapeHtml(t) + '</span>'; }});
    tagsHtml += '</div>';
  }}

  let refsHtml = '';
  if (m.resolved_refs && m.resolved_refs.length) {{
    refsHtml = '<div class="dossier-section-title">Guarded Code References (.aw/index.db verified)</div><div class="chips-row">';
    m.resolved_refs.forEach(r => {{
      const verClass = r.verified ? 'chip-ref verified' : 'chip-ref';
      const verBadge = r.verified ? ' [VERIFIED ' + r.type.toUpperCase() + ']' : '';
      const locStr = r.location ? ' &middot; ' + escapeHtml(r.location) : '';
      refsHtml += '<span class="' + verClass + '"><strong>' + escapeHtml(r.ref) + '</strong>' + verBadge + locStr + '</span>';
    }});
    refsHtml += '</div>';
  }} else if (m.refs && m.refs.length) {{
    refsHtml = '<div class="dossier-section-title">Referenced Symbols &amp; Paths</div><div class="chips-row">';
    m.refs.forEach(r => {{ refsHtml += '<span class="chip chip-ref">' + escapeHtml(r) + '</span>'; }});
    refsHtml += '</div>';
  }}

  // Find all revisions for the same key & store
  const allRevs = allMemories.filter(other => other.store === m.store && other.key === m.key);
  let revTimelineHtml = '';
  if (allRevs.length > 1) {{
    revTimelineHtml = '<div class="dossier-section-title">Revision History (' + allRevs.length + ' versions)</div><div class="rev-timeline">';
    allRevs.sort((a, b) => b.rev - a.rev).forEach(r => {{
      const isCur = r.rev === m.rev;
      const rDate = r.created_at ? new Date(r.created_at * 1000).toLocaleDateString() : '—';
      revTimelineHtml += '<div class="rev-step' + (isCur ? ' active' : '') + '">' +
        '<strong>r' + r.rev + '</strong> ' +
        '<span class="badge ' + (r.is_current ? 'status-ok' : 'status-warn') + '">' + (r.is_current ? 'CURRENT' : 'SUPERSEDED') + '</span>' +
        '<span class="text-muted" style="margin-left:auto;">' + rDate + '</span>' +
        '</div>';
    }});
    revTimelineHtml += '</div>';
  }}

  container.innerHTML =
    '<div class="dossier-head">' +
    '  <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom: 0.5rem;">' +
    '    <div style="display:flex; gap:0.45rem; align-items:center;">' +
    '      <span class="' + kindClass + '">' + m.kind.toUpperCase() + '</span>' +
    '      <span class="badge" style="background:var(--bg-subtle);">Rev ' + m.rev + '</span>' +
    '      <span class="badge badge-repo">' + escapeHtml(m.store) + '</span>' +
    '    </div>' +
    '    <span class="text-muted" style="font-size:0.78rem;">' + dateStr + '</span>' +
    '  </div>' +
    '  <div class="dossier-title">' + escapeHtml(m.key) + '</div>' +
    '  <div class="text-muted" style="font-size:0.8rem;">Scope: <strong>' + m.scope + '</strong>' +
       (m.scope === 'session' && m.expires_at ? ' (expires ' + new Date(m.expires_at * 1000).toLocaleString() + ')' : '') +
       ' &middot; Source: <code>' + m.source + '</code></div>' +
    '</div>' +
    '<div class="recall-banner">' +
    '  <div><strong>' + escapeHtml(m.prev_category || 'KNOWLEDGE GUARD') + ':</strong> ' + escapeHtml(m.prev_detail || '') + '</div>' +
    '  <div style="font-family:var(--font-mono); font-size:0.78rem;">Recalls: <strong>' + m.recall_count + '</strong> &middot; Last: ' + lastRecalledStr + '</div>' +
    '</div>' +
    '<div class="dossier-section-title">Memory Content</div>' +
    '<div class="dossier-body">' + escapeHtml(m.body) + '</div>' +
    refsHtml +
    tagsHtml +
    revTimelineHtml;
}}

// Live Event Stream (SSE)
if (window.location.protocol.startsWith('http')) {{
  const liveBadge = document.getElementById('live-indicator');
  if (liveBadge) liveBadge.style.display = 'inline-block';

  const source = new EventSource('/events');
  source.onmessage = function(e) {{
    try {{
      const q = JSON.parse(e.data);
      handleLiveQuery(q);

      // If event touched memory, refresh memory store
      if (q.tool && q.tool.startsWith('mem_')) {{
        fetch('/api/memories')
          .then(r => r.json())
          .then(data => {{
            allMemories = data;
            const navBadge = document.getElementById('nav-mem-count');
            if (navBadge) navBadge.textContent = allMemories.filter(m => m.is_current).length;
            renderMemoryList();
          }}).catch(() => {{}});
      }}

      // If event touched reuse or search, refresh reuse data
      if (q.tool && (q.tool === 'reuse' || q.tool === 'search')) {{
        fetch('/api/reuse')
          .then(r => r.json())
          .then(data => {{
            reuseCandidates = data.candidates || [];
            prewriteSearches = data.prewrite_searches || [];
            const navReuseBadge = document.getElementById('nav-reuse-count');
            if (navReuseBadge) navReuseBadge.textContent = reuseCandidates.length;
            renderReuseList();
            renderPrewriteTable();
          }}).catch(() => {{}});
      }}
    }} catch(err) {{}}
  }};
}}

function handleLiveQuery(q) {{
  totalCalls++;
  const exitCode = parseInt(q.exit_code || 0, 10);
  if (exitCode >= 3) totalErrors++;

  const totalCallsEl = document.getElementById('metric-total-calls');
  if (totalCallsEl) totalCallsEl.textContent = totalCalls;

  const relEl = document.getElementById('metric-reliability');
  if (relEl) {{
    const rel = totalCalls > 0 ? (100.0 * (totalCalls - totalErrors) / totalCalls).toFixed(1) : "100.0";
    relEl.textContent = rel + "% clean (" + totalErrors + " errs)";
  }}

  const recentTbody = document.getElementById('recent-tbody');
  if (recentTbody) {{
    const tr = document.createElement('tr');
    tr.className = 'query-row';
    const rName = q.repo_name || 'workspace';
    tr.setAttribute('data-repo', rName);
    tr.setAttribute('data-code', exitCode);

    const ts = q.ts ? new Date(q.ts * 1000).toTimeString().split(' ')[0] : '—';
    const latStr = q.latency_us ? (q.latency_us / 1000.0).toFixed(1) + 'ms' : '—';
    let statusClass = 'status-ok';
    let statusText = 'ok';
    if (exitCode === 1) {{
      statusClass = 'status-warn';
      statusText = 'empty';
    }} else if (exitCode === 2) {{
      statusClass = 'status-ambig';
      statusText = 'ambig';
    }} else if (exitCode >= 3) {{
      statusClass = 'status-err';
      statusText = 'err ' + exitCode;
    }}

    const argsRaw = JSON.stringify(q.args || {{}});
    const argsShort = argsRaw.length > 90 ? argsRaw.substring(0, 90) + '...' : argsRaw;

    tr.innerHTML =
      '<td class="num-col text-muted">' + ts + '</td>' +
      '<td><span class="repo-tag">' + escapeHtml(rName) + '</span></td>' +
      '<td><code>' + escapeHtml(q.tool || '—') + '</code></td>' +
      '<td class="text-right num-col">' + (q.shown || q.result_count || 0) + '</td>' +
      '<td class="text-right num-col">' + latStr + '</td>' +
      '<td><span class="badge ' + statusClass + '">' + statusText + '</span></td>' +
      '<td class="args-cell" title="' + escapeHtml(argsRaw) + '"><code>' + escapeHtml(argsShort) + '</code></td>';

    recentTbody.insertBefore(tr, recentTbody.firstChild);
    while (recentTbody.children.length > 60) {{
      recentTbody.removeChild(recentTbody.lastChild);
    }}
  }}
}}

function escapeHtml(str) {{
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}}

// Initialize On Load
window.addEventListener('DOMContentLoaded', () => {{
  initTheme();
  populateReuseRepoFilter();
  populateMemoryStoreFilter();
  renderReuseList();
  renderMemoryList();
  renderPrewriteTable();
}});
</script>
</body>
</html>
"""


def run_live_server(aw_dir: Path, port: int = 7890, open_browser: bool = True) -> None:
    """Serve interactive dashboard with live SSE streaming and JSON API endpoints."""
    log_file = aw_dir / "usage.log"

    class QuietThreadingHTTPServer(http.server.ThreadingHTTPServer):
        daemon_threads = True

    class SSEHandler(http.server.BaseHTTPRequestHandler):
        def do_HEAD(self):
            if self.path in ("/", "/index.html", "/dashboard.html"):
                self.send_response(200)
                self.send_header("Content-Type", "text/html; charset=utf-8")
                self.send_header("Connection", "close")
                self.end_headers()
            elif self.path == "/favicon.ico":
                self.send_response(204)
                self.send_header("Connection", "close")
                self.end_headers()
            elif self.path in ("/api/summary", "/api/metrics", "/api/memories", "/api/reuse"):
                self.send_response(200)
                self.send_header("Content-Type", "application/json; charset=utf-8")
                self.send_header("Connection", "close")
                self.end_headers()
            else:
                self.send_response(404)
                self.send_header("Connection", "close")
                self.end_headers()

        def do_GET(self):
            if self.path in ("/", "/index.html", "/dashboard.html"):
                self.close_connection = True
                entries = parse_usage_log_from_dir(aw_dir)
                metrics = calculate_metrics(entries)
                memories = collect_all_memories(aw_dir, entries)
                candidates, reuse_summary, prewrite_searches = collect_all_reuse_candidates(aw_dir, entries)
                content = render_html_dashboard(
                    metrics,
                    inspect_index_db(None),
                    "Global Multi-Repo",
                    memories,
                    candidates,
                    reuse_summary,
                    prewrite_searches,
                    is_global=True,
                )
                encoded = content.encode("utf-8")
                self.send_response(200)
                self.send_header("Content-Type", "text/html; charset=utf-8")
                self.send_header("Content-Length", str(len(encoded)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(encoded)
            elif self.path == "/favicon.ico":
                self.close_connection = True
                self.send_response(204)
                self.send_header("Connection", "close")
                self.end_headers()
            elif self.path in ("/api/summary", "/api/metrics"):
                self.close_connection = True
                entries = parse_usage_log_from_dir(aw_dir)
                metrics = calculate_metrics(entries)
                memories = collect_all_memories(aw_dir, entries)
                candidates, reuse_summary, prewrite_searches = collect_all_reuse_candidates(aw_dir, entries)
                data = {
                    "total_calls": metrics.get("total_calls", 0),
                    "error_count": metrics.get("error_count", 0),
                    "warning_count": metrics.get("warning_count", 0),
                    "reliability_pct": metrics.get("reliability_pct", 100.0),
                    "p50_ms": metrics.get("p50_ms", 0.0),
                    "p95_ms": metrics.get("p95_ms", 0.0),
                    "saved_tokens": metrics.get("saved_tokens", 0),
                    "saved_dollars": metrics.get("saved_dollars", 0.0),
                    "repo_count": len(metrics.get("repo_aggs", {})),
                    "active_memories": sum(1 for m in memories if m.get("is_current", True)),
                    "total_memories": len(memories),
                    "duplicate_candidates": len(candidates),
                    "preventable_loc": reuse_summary.get("preventable_loc", 0),
                    "prewrite_searches": len(prewrite_searches),
                    "tool_counts": {t: d.get("calls", 0) for t, d in metrics.get("tool_aggs", {}).items()},
                }
                encoded = json.dumps(data, indent=2).encode("utf-8")
                self.send_response(200)
                self.send_header("Content-Type", "application/json; charset=utf-8")
                self.send_header("Content-Length", str(len(encoded)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(encoded)
            elif self.path == "/api/memories":
                self.close_connection = True
                entries = parse_usage_log_from_dir(aw_dir)
                memories = collect_all_memories(aw_dir, entries)
                encoded = json.dumps(memories, indent=2, ensure_ascii=False).encode("utf-8")
                self.send_response(200)
                self.send_header("Content-Type", "application/json; charset=utf-8")
                self.send_header("Content-Length", str(len(encoded)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(encoded)
            elif self.path == "/api/reuse":
                self.close_connection = True
                entries = parse_usage_log_from_dir(aw_dir)
                candidates, reuse_summary, prewrite_searches = collect_all_reuse_candidates(aw_dir, entries)
                payload = {
                    "candidates": candidates,
                    "summary": reuse_summary,
                    "prewrite_searches": prewrite_searches,
                }
                encoded = json.dumps(payload, indent=2, ensure_ascii=False).encode("utf-8")
                self.send_response(200)
                self.send_header("Content-Type", "application/json; charset=utf-8")
                self.send_header("Content-Length", str(len(encoded)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(encoded)
            elif self.path == "/events":
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Cache-Control", "no-cache")
                self.send_header("Connection", "keep-alive")
                self.send_header("Access-Control-Allow-Origin", "*")
                self.end_headers()

                last_pos = 0
                if log_file.is_file():
                    try:
                        last_pos = log_file.stat().st_size
                    except OSError:
                        pass

                last_keepalive = time.time()
                while True:
                    try:
                        now = time.time()
                        if log_file.is_file():
                            current_size = log_file.stat().st_size
                            if current_size < last_pos:
                                last_pos = 0
                            if current_size > last_pos:
                                with open(log_file, "r", encoding="utf-8", errors="replace") as f:
                                    f.seek(last_pos)
                                    for line in f:
                                        line = line.strip()
                                        if line:
                                            self.wfile.write(f"data: {line}\n\n".encode("utf-8"))
                                            self.wfile.flush()
                                            last_keepalive = now
                                    last_pos = f.tell()

                        if now - last_keepalive >= 15.0:
                            self.wfile.write(b": keepalive\n\n")
                            self.wfile.flush()
                            last_keepalive = now

                        time.sleep(0.5)
                    except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                        break
                    except OSError as e:
                        if e.errno in (32, 54, 104):
                            break
                        break
                    except Exception:
                        break
            else:
                self.close_connection = True
                self.send_response(404)
                self.send_header("Connection", "close")
                self.end_headers()

        def log_message(self, format, *args):
            pass

    bind_host = "0.0.0.0"
    try:
        server = QuietThreadingHTTPServer((bind_host, port), SSEHandler)
    except OSError:
        bind_host = "127.0.0.1"
        server = QuietThreadingHTTPServer((bind_host, port), SSEHandler)

    url = f"http://localhost:{port}/"
    print(f"aw: live dashboard running at {url}")
    print("Press Ctrl+C to stop.")

    if open_browser:
        opener = "open" if sys.platform == "darwin" else "xdg-open"
        try:
            subprocess.run([opener, url], check=False)
        except OSError:
            pass

    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nShutting down live dashboard.")
        server.server_close()


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate an interactive HTML telemetry, anti-reinvention, and memory dashboard for aw."
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=None,
        help="Repository root directory containing .aw/ (default: auto-detect)",
    )
    parser.add_argument(
        "--global",
        dest="is_global",
        action="store_true",
        help="Read from global ~/.aw/usage.log instead of local repository",
    )
    parser.add_argument(
        "--serve",
        type=int,
        nargs="?",
        const=7890,
        default=None,
        metavar="PORT",
        help="Start live HTTP + SSE server on specified port (default: 7890)",
    )
    parser.add_argument(
        "--open",
        action="store_true",
        help="Open dashboard in default web browser automatically",
    )
    args = parser.parse_args()

    aw_dir = resolve_global_aw_dir() if args.is_global else (args.root / ".aw" if args.root else Path.cwd() / ".aw")

    if args.serve is not None:
        run_live_server(aw_dir, port=args.serve, open_browser=args.open)
        return

    entries = parse_usage_log_from_dir(aw_dir)
    metrics = calculate_metrics(entries)
    index_stats = inspect_index_db(args.root or Path.cwd())
    scope_name = "Global Multi-Repo" if args.is_global else (args.root.name if args.root else Path.cwd().name)
    memories = collect_all_memories(aw_dir, entries, single_root=args.root)
    candidates, reuse_summary, prewrite_searches = collect_all_reuse_candidates(aw_dir, entries, single_root=args.root)

    content = render_html_dashboard(
        metrics,
        index_stats,
        scope_name,
        memories,
        candidates,
        reuse_summary,
        prewrite_searches,
        is_global=args.is_global,
    )
    print(content)


if __name__ == "__main__":
    main()
