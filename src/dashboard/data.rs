//! Data aggregation engine for the native dashboard.
//! Queries `.aw/usage.log`, `.aw/index.db`, and `.aw/memory.db` with sub-millisecond latency.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::{json, Value};

use crate::db;
use crate::error::Result;

pub struct AggregatedData {
    pub summary: Value,
    pub reuse: Value,
    pub memories: Value,
}

pub fn read_usage_entries(log_path: &Path) -> Vec<Value> {
    let mut entries = Vec::new();
    let parent = log_path.parent().unwrap_or_else(|| Path::new("."));

    for filename in &["usage.log.1", "usage.log"] {
        let p = parent.join(filename);
        if let Ok(file) = File::open(&p) {
            let reader = BufReader::new(file);
            for line in reader.lines().map_while(std::result::Result::ok) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
                    entries.push(val);
                }
            }
        }
    }
    entries
}

pub fn compute_summary(entries: &[Value]) -> Value {
    let total_calls = entries.len();
    if total_calls == 0 {
        return json!({
            "total_calls": 0,
            "error_count": 0,
            "warning_count": 0,
            "reliability_pct": 100.0,
            "p50_ms": 0.0,
            "p95_ms": 0.0,
            "avg_ms": 0.0,
            "saved_tokens": 0,
            "saved_dollars": 0.0,
            "reuse_hits": 0,
            "tool_aggs": {},
            "repo_aggs": {},
            "recent_queries": [],
            "recent_errors": []
        });
    }

    let mut errors = 0usize;
    let mut warnings = 0usize;
    let mut reuse_hits = 0usize;
    let mut saved_tokens = 0usize;
    let mut latencies_ms: Vec<f64> = Vec::with_capacity(entries.len());
    let mut tool_aggs: HashMap<String, ToolAgg> = HashMap::new();
    let mut repo_aggs: HashMap<String, RepoAgg> = HashMap::new();
    let mut recent_errors: Vec<Value> = Vec::new();

    for e in entries {
        let tool = e.get("tool").and_then(Value::as_str).unwrap_or("unknown");
        let exit_code = e.get("exit_code").and_then(Value::as_i64).unwrap_or(0);
        let shown = e
            .get("shown")
            .or_else(|| e.get("result_count"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        let result_count = e.get("result_count").and_then(Value::as_u64).unwrap_or(0) as usize;
        let lat_us = e.get("latency_us").and_then(Value::as_f64);
        let ts = e.get("ts").and_then(Value::as_f64).unwrap_or(0.0);
        let repo_name = e
            .get("repo_name")
            .and_then(Value::as_str)
            .unwrap_or("workspace");
        let is_worktree = e.get("worktree").and_then(Value::as_bool).unwrap_or(false);

        let t_entry = tool_aggs.entry(tool.to_string()).or_default();
        t_entry.calls += 1;
        t_entry.shown += shown;

        let r_entry = repo_aggs.entry(repo_name.to_string()).or_default();
        r_entry.calls += 1;
        r_entry.worktree = is_worktree;
        if ts > r_entry.last_ts {
            r_entry.last_ts = ts;
        }

        if exit_code >= 3 {
            errors += 1;
            t_entry.errors += 1;
            r_entry.errors += 1;
            recent_errors.push(e.clone());
        } else if exit_code == 1 || exit_code == 2 {
            warnings += 1;
        }

        if let Some(us) = lat_us {
            if us >= 0.0 {
                let ms = us / 1000.0;
                latencies_ms.push(ms);
                t_entry.latencies.push(ms);
            }
        }

        if exit_code == 0 {
            match tool {
                "context" | "callers" | "callees" | "path" | "impact" | "imports" => {
                    saved_tokens += shown * 350 + 100;
                }
                "search" => {
                    saved_tokens += shown * 300 + 100;
                }
                "reuse" => {
                    reuse_hits += shown.max(result_count);
                    saved_tokens += result_count * 800 + 1200;
                }
                "symbols" => {
                    saved_tokens += shown * 150;
                }
                "build" => {
                    saved_tokens += 500;
                }
                _ => {}
            }
        }
    }

    latencies_ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = latencies_ms.len();
    let p50 = if n > 0 {
        latencies_ms[(0.50 * n as f64).ceil() as usize - 1]
    } else {
        0.0
    };
    let p95 = if n > 0 {
        latencies_ms[(0.95 * n as f64).ceil() as usize - 1]
    } else {
        0.0
    };
    let avg = if n > 0 {
        latencies_ms.iter().sum::<f64>() / n as f64
    } else {
        0.0
    };

    let mut tool_aggs_json = BTreeMap::new();
    for (k, mut v) in tool_aggs {
        v.latencies
            .sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let vn = v.latencies.len();
        let p50_t = if vn > 0 {
            v.latencies[(0.50 * vn as f64).ceil() as usize - 1]
        } else {
            0.0
        };
        tool_aggs_json.insert(
            k,
            json!({
                "calls": v.calls,
                "errors": v.errors,
                "shown": v.shown,
                "p50": p50_t,
            }),
        );
    }

    let mut repo_aggs_json = BTreeMap::new();
    for (k, v) in repo_aggs {
        let rel = if v.calls > 0 {
            100.0 * (v.calls - v.errors) as f64 / v.calls as f64
        } else {
            100.0
        };
        repo_aggs_json.insert(
            k,
            json!({
                "calls": v.calls,
                "errors": v.errors,
                "worktree": v.worktree,
                "last_ts": v.last_ts,
                "reliability_pct": rel,
            }),
        );
    }

    let reliability = 100.0 * (total_calls - errors) as f64 / total_calls as f64;
    let saved_dollars = saved_tokens as f64 * 0.000003;

    let recent_queries: Vec<Value> = entries.iter().rev().take(50).cloned().collect();
    recent_errors.reverse();
    recent_errors.truncate(25);

    json!({
        "total_calls": total_calls,
        "error_count": errors,
        "warning_count": warnings,
        "reliability_pct": reliability,
        "p50_ms": p50,
        "p95_ms": p95,
        "avg_ms": avg,
        "saved_tokens": saved_tokens,
        "saved_dollars": saved_dollars,
        "reuse_hits": reuse_hits,
        "tool_aggs": tool_aggs_json,
        "repo_aggs": repo_aggs_json,
        "recent_queries": recent_queries,
        "recent_errors": recent_errors,
    })
}

#[derive(Default)]
struct ToolAgg {
    calls: usize,
    errors: usize,
    shown: usize,
    latencies: Vec<f64>,
}

#[derive(Default)]
struct RepoAgg {
    calls: usize,
    errors: usize,
    worktree: bool,
    last_ts: f64,
}

pub fn read_snippet(file_path: &Path, line_start: i64, line_end: i64, max_lines: usize) -> String {
    if !file_path.is_file() {
        return String::new();
    }
    if let Ok(file) = File::open(file_path) {
        let reader = BufReader::new(file);
        let start = (line_start.max(1) - 1) as usize;
        let end = (line_end.max(line_start) as usize).max(start + 1);

        let lines: Vec<String> = reader.lines().map_while(std::result::Result::ok).collect();
        if start >= lines.len() {
            return String::new();
        }

        let slice_end = end.min(lines.len());
        let sub = &lines[start..slice_end];
        let mut out = Vec::new();
        for (idx, line) in sub.iter().take(max_lines).enumerate() {
            out.push(format!("{:4} | {}", line_start as usize + idx, line));
        }
        if sub.len() > max_lines {
            out.push(format!(" ... ({} more lines)", sub.len() - max_lines));
        }
        return out.join("\n");
    }
    String::new()
}

pub fn collect_reuse_candidates(
    root: Option<&Path>,
    entries: &[Value],
) -> Result<(Vec<Value>, Value, Vec<Value>)> {
    let mut repos: HashMap<String, PathBuf> = HashMap::new();
    if let Some(r) = root {
        let db_path = r.join(".aw").join("index.db");
        if db_path.is_file() {
            let name = r.file_name().and_then(|n| n.to_str()).unwrap_or("repo");
            repos.insert(name.to_string(), r.to_path_buf());
        }
    } else {
        for e in entries {
            if let (Some(r), Some(r_name)) = (
                e.get("repo").and_then(Value::as_str),
                e.get("repo_name").and_then(Value::as_str),
            ) {
                let p = PathBuf::from(r);
                if p.join(".aw").join("index.db").is_file() && !repos.contains_key(r_name) {
                    repos.insert(r_name.to_string(), p);
                }
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            let name = cwd
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("workspace");
            if cwd.join(".aw").join("index.db").is_file() && !repos.contains_key(name) {
                repos.insert(name.to_string(), cwd);
            }
        }
    }

    let mut candidates_json = Vec::new();

    for (r_name, r_root) in &repos {
        let db_path = r_root.join(".aw").join("index.db");
        if !db_path.is_file() {
            continue;
        }

        let conn =
            match Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            {
                Ok(c) => c,
                Err(_) => continue,
            };

        let (candidates, _) = match crate::query::reuse::analyze_corpus(&conn, None, None, 25) {
            Ok(res) => res,
            Err(_) => continue,
        };

        for c in candidates {
            let (det_title, det_summary) = match c.detector {
                "D1" => (
                    "D1 Callee Footprint Overlap",
                    format!("Workflow '{}' calls identical downstream functions as canonical '{}' ([callers={}]).", c.symbol, c.canonical, c.canonical_callers),
                ),
                "D2" => (
                    "D2 Interface Token Overlap",
                    format!("Signature and parameters of '{}' duplicate canonical '{}' ([callers={}]).", c.symbol, c.canonical, c.canonical_callers),
                ),
                "D3" => (
                    "D3 Raw Client Bypass",
                    format!("Symbol '{}' calls raw external clients directly instead of canonical '{}' ([callers={}]).", c.symbol, c.canonical, c.canonical_callers),
                ),
                _ => (
                    "D4 Redundant Pass-Through Wrapper",
                    format!("Single-caller wrapper '{}' adds zero logic around canonical '{}' ([callers={}]).", c.symbol, c.canonical, c.canonical_callers),
                ),
            };

            // Query candidate location and canonical location from symbols
            let mut cand_info = json!({
                "file": c.file,
                "line_start": c.line,
                "line_end": c.line + 10,
                "loc": 10
            });
            let mut can_info = json!({
                "file": c.canonical_file,
                "line_start": c.canonical_line,
                "line_end": c.canonical_line + 10,
                "loc": 10
            });

            if let Ok(mut stmt) = conn.prepare("SELECT file_path, line_start, line_end FROM symbols WHERE qualname = ?1 OR qualname LIKE ?2 LIMIT 1") {
                if let Ok(row) = stmt.query_row(rusqlite::params![c.symbol, format!("%{}", c.symbol)], |r| {
                    let fp: String = r.get(0)?;
                    let ls: i64 = r.get(1)?;
                    let le: i64 = r.get(2)?;
                    Ok((fp, ls, le))
                }) {
                    let loc = (row.2 - row.1 + 1).max(1);
                    cand_info = json!({ "file": row.0, "line_start": row.1, "line_end": row.2, "loc": loc });
                }
                if let Ok(row) = stmt.query_row(rusqlite::params![c.canonical, format!("%{}", c.canonical)], |r| {
                    let fp: String = r.get(0)?;
                    let ls: i64 = r.get(1)?;
                    let le: i64 = r.get(2)?;
                    Ok((fp, ls, le))
                }) {
                    let loc = (row.2 - row.1 + 1).max(1);
                    can_info = json!({ "file": row.0, "line_start": row.1, "line_end": row.2, "loc": loc });
                }
            }

            let cand_file = cand_info.get("file").and_then(Value::as_str).unwrap_or("");
            let cand_start = cand_info
                .get("line_start")
                .and_then(Value::as_i64)
                .unwrap_or(1);
            let cand_end = cand_info
                .get("line_end")
                .and_then(Value::as_i64)
                .unwrap_or(cand_start + 15);
            let cand_snippet = read_snippet(&r_root.join(cand_file), cand_start, cand_end, 30);

            let can_file = can_info.get("file").and_then(Value::as_str).unwrap_or("");
            let can_start = can_info
                .get("line_start")
                .and_then(Value::as_i64)
                .unwrap_or(1);
            let can_end = can_info
                .get("line_end")
                .and_then(Value::as_i64)
                .unwrap_or(can_start + 15);
            let can_snippet = read_snippet(&r_root.join(can_file), can_start, can_end, 30);

            let loc_saving = cand_info.get("loc").and_then(Value::as_i64).unwrap_or(10);
            let narrative = format!(
                "Consolidating '{}' into authoritative '{}' ([callers={}]) deletes ~{} duplicate lines and eliminates drift between twin implementations.",
                c.symbol, c.canonical, c.canonical_callers, loc_saving
            );

            let shared_evidence = if c.shared_items.is_empty() {
                String::new()
            } else {
                format!(
                    "{}: [{}]",
                    match c.detector {
                        "D1" => "shared callees",
                        "D2" => "shared tokens",
                        "D3" => "client bypass",
                        _ => "details",
                    },
                    c.shared_items.join(", ")
                )
            };

            candidates_json.push(json!({
                "id": format!("{}:{}->{}", r_name, c.symbol, c.canonical),
                "repo": r_name,
                "detector": c.detector,
                "detector_title": det_title,
                "detector_summary": det_summary,
                "symbol": c.symbol,
                "canonical": c.canonical,
                "canonical_callers": c.canonical_callers,
                "canonical_file": c.canonical_file,
                "canonical_line": c.canonical_line,
                "overlap_pct": c.overlap_pct,
                "shared_evidence": shared_evidence,
                "candidate_info": cand_info,
                "canonical_info": can_info,
                "candidate_snippet": cand_snippet,
                "canonical_snippet": can_snippet,
                "loc_saving": loc_saving,
                "prevention_narrative": narrative,
            }));
        }
    }

    // Prewrite search telemetry
    let mut prewrite_searches = Vec::new();
    for e in entries.iter().rev() {
        let tool = e.get("tool").and_then(Value::as_str).unwrap_or("");
        if tool == "search" || tool == "reuse" {
            let args = e.get("args").cloned().unwrap_or(Value::Null);
            let terms = args
                .get("terms")
                .or_else(|| args.get("paths"))
                .and_then(Value::as_str)
                .unwrap_or("");
            prewrite_searches.push(json!({
                "ts": e.get("ts").unwrap_or(&Value::Null),
                "repo_name": e.get("repo_name").unwrap_or(&Value::Null),
                "tool": tool,
                "terms": terms,
                "shown": e.get("shown").or_else(|| e.get("result_count")).unwrap_or(&json!(0)),
                "latency_us": e.get("latency_us").unwrap_or(&json!(0)),
            }));
            if prewrite_searches.len() >= 40 {
                break;
            }
        }
    }

    let total_cands = candidates_json.len();
    let prev_loc: i64 = candidates_json
        .iter()
        .map(|c| c.get("loc_saving").and_then(Value::as_i64).unwrap_or(0))
        .sum();
    let prewrite_count = entries
        .iter()
        .filter(|e| {
            matches!(
                e.get("tool").and_then(Value::as_str),
                Some("search") | Some("reuse")
            )
        })
        .count();
    let avg_callers = if total_cands > 0 {
        let callers_sum: i64 = candidates_json
            .iter()
            .map(|c| {
                c.get("canonical_callers")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
            })
            .sum();
        callers_sum as f64 / total_cands as f64
    } else {
        0.0
    };

    let summary = json!({
        "total_candidates": total_cands,
        "preventable_loc": prev_loc,
        "prewrite_count": prewrite_count,
        "avg_callers": (avg_callers * 10.0).round() / 10.0,
        "detector_counts": {
            "D1": candidates_json.iter().filter(|c| c.get("detector") == Some(&json!("D1"))).count(),
            "D2": candidates_json.iter().filter(|c| c.get("detector") == Some(&json!("D2"))).count(),
            "D3": candidates_json.iter().filter(|c| c.get("detector") == Some(&json!("D3"))).count(),
            "D4": candidates_json.iter().filter(|c| c.get("detector") == Some(&json!("D4"))).count(),
        }
    });

    Ok((candidates_json, summary, prewrite_searches))
}

pub fn collect_memories(
    root: Option<&Path>,
    global_only: bool,
    entries: &[Value],
) -> Result<Vec<Value>> {
    let mut stores: HashMap<String, (PathBuf, PathBuf)> = HashMap::new();

    if let Ok(g_dir) = db::global_aw_dir() {
        let g_db = g_dir.join("memory.db");
        if g_db.is_file() {
            stores.insert("global".to_string(), (g_dir, g_db));
        }
    }

    if !global_only {
        if let Some(r) = root {
            let l_db = r.join(".aw").join("memory.db");
            if l_db.is_file() {
                let name = r.file_name().and_then(|n| n.to_str()).unwrap_or("repo");
                stores.insert(name.to_string(), (r.to_path_buf(), l_db));
            }
        } else {
            for e in entries {
                if let (Some(r), Some(r_name)) = (
                    e.get("repo").and_then(Value::as_str),
                    e.get("repo_name").and_then(Value::as_str),
                ) {
                    let rp = PathBuf::from(r);
                    let l_db = rp.join(".aw").join("memory.db");
                    if l_db.is_file() && !stores.contains_key(r_name) {
                        stores.insert(r_name.to_string(), (rp, l_db));
                    }
                }
            }
            if let Ok(cwd) = std::env::current_dir() {
                let name = cwd
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("workspace");
                let l_db = cwd.join(".aw").join("memory.db");
                if l_db.is_file() && !stores.contains_key(name) {
                    stores.insert(name.to_string(), (cwd, l_db));
                }
            }
        }
    }

    // Correlate recall counts from usage entries
    let mut recalls_map: HashMap<String, (usize, f64)> = HashMap::new();
    for e in entries {
        let tool = e.get("tool").and_then(Value::as_str).unwrap_or("");
        if tool == "mem_get" || tool == "mem_query" {
            let ts = e.get("ts").and_then(Value::as_f64).unwrap_or(0.0);
            let args = e.get("args").cloned().unwrap_or(Value::Null);
            let key = args.get("key").and_then(Value::as_str).unwrap_or("");
            if !key.is_empty() {
                let rec = recalls_map.entry(key.to_string()).or_insert((0, 0.0));
                rec.0 += 1;
                if ts > rec.1 {
                    rec.1 = ts;
                }
            }
        }
    }

    let mut notes_json = Vec::new();

    for (store_name, (repo_root, db_path)) in stores {
        let conn =
            match Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            {
                Ok(c) => c,
                Err(_) => continue,
            };

        let index_conn = Connection::open_with_flags(
            repo_root.join(".aw").join("index.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .ok();

        let mut stmt = match conn.prepare(
            "SELECT id, key, rev, kind, scope, session, body, tags, refs, content_hash, source, created_at, expires_at, superseded_by, superseded_at FROM memories ORDER BY key ASC, rev DESC"
        ) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let rows = stmt.query_map([], |r| {
            let id: i64 = r.get(0)?;
            let key: String = r.get(1)?;
            let rev: i64 = r.get(2)?;
            let kind: String = r.get(3)?;
            let scope: String = r.get(4)?;
            let session: String = r.get(5)?;
            let body: String = r.get(6)?;
            let tags_raw: String = r.get(7)?;
            let refs_raw: String = r.get(8)?;
            let content_hash: i64 = r.get(9)?;
            let source: String = r.get(10)?;
            let created_at: i64 = r.get(11)?;
            let expires_at: i64 = r.get(12)?;
            let superseded_by: Option<i64> = r.get(13)?;
            let superseded_at: i64 = r.get(14)?;
            Ok((
                id,
                key,
                rev,
                kind,
                scope,
                session,
                body,
                tags_raw,
                refs_raw,
                content_hash,
                source,
                created_at,
                expires_at,
                superseded_by,
                superseded_at,
            ))
        });

        if let Ok(mapped_rows) = rows {
            for row in mapped_rows.flatten() {
                let (
                    id,
                    key,
                    rev,
                    kind,
                    scope,
                    session,
                    body,
                    tags_raw,
                    refs_raw,
                    content_hash,
                    source,
                    created_at,
                    expires_at,
                    superseded_by,
                    superseded_at,
                ) = row;
                let is_current = superseded_by.is_none();
                let tags: Vec<String> =
                    tags_raw.split_whitespace().map(|s| s.to_string()).collect();
                let refs_list: Vec<String> =
                    refs_raw.split_whitespace().map(|s| s.to_string()).collect();

                let (prev_cat, prev_title, prev_detail) = match kind.as_str() {
                    "gotcha" => (
                        "REGRESSION GUARD",
                        "Regression & Bug Guard",
                        format!("Prevents repeating root-cause trap in '{}'. Blocks known fragility before changes are committed.", key),
                    ),
                    "decision" => (
                        "ARCHITECTURE INVARIANT",
                        "Architecture & Policy Guard",
                        format!("Enforces repository architectural constraint '{}'. Prevents forbidden abstractions or boundary violations.", key),
                    ),
                    "fact" => (
                        "EXPLORATION SHORTCUT",
                        "Verified Fact Shortcut",
                        format!("Avoids costly multi-file grep scans by caching verified system truth for '{}'.", key),
                    ),
                    _ => (
                        "HANDOFF CONTEXT",
                        "Cross-Session Handoff Guard",
                        format!("Preserves active state across conversation turns for '{}'.", key),
                    ),
                };

                let mut recall_count = recalls_map.get(&key).map(|r| r.0).unwrap_or(0);
                let mut last_recalled_ts = recalls_map.get(&key).map(|r| r.1).unwrap_or(0.0);

                if recall_count == 0 {
                    let key_lower = key.to_lowercase();
                    let body_lower = body.to_lowercase();
                    for e in entries {
                        let tool = e.get("tool").and_then(Value::as_str).unwrap_or("");
                        if tool == "mem_query" || tool == "mem_get" {
                            let args = e.get("args").cloned().unwrap_or(Value::Null);
                            let terms = args
                                .get("terms")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_lowercase();
                            if !terms.is_empty()
                                && (key_lower.contains(&terms) || body_lower.contains(&terms))
                            {
                                recall_count += 1;
                                let ts = e.get("ts").and_then(Value::as_f64).unwrap_or(0.0);
                                if ts > last_recalled_ts {
                                    last_recalled_ts = ts;
                                }
                            }
                        }
                    }
                }

                // Verify refs against index.db
                let mut resolved_refs = Vec::new();
                for r in &refs_list {
                    let mut ref_val = json!({
                        "ref": r,
                        "verified": false,
                        "type": "path",
                        "location": ""
                    });

                    if let Some(ref iconn) = index_conn {
                        if let Ok(mut f_stmt) = iconn.prepare(
                            "SELECT path, size FROM files WHERE path = ?1 OR path LIKE ?2 LIMIT 1",
                        ) {
                            if let Ok(f_row) =
                                f_stmt.query_row(rusqlite::params![r, format!("%{}", r)], |row| {
                                    let path: String = row.get(0)?;
                                    let size: i64 = row.get(1)?;
                                    Ok((path, size))
                                })
                            {
                                ref_val = json!({
                                    "ref": r,
                                    "verified": true,
                                    "type": "file",
                                    "location": format!("{} ({} bytes)", f_row.0, f_row.1)
                                });
                            }
                        }
                        if !ref_val
                            .get("verified")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                        {
                            if let Ok(mut s_stmt) = iconn.prepare("SELECT file_path, line_start, line_end FROM symbols WHERE name = ?1 OR qualname = ?1 LIMIT 1") {
                                if let Ok(s_row) = s_stmt.query_row(rusqlite::params![r], |row| {
                                    let fp: String = row.get(0)?;
                                    let ls: i64 = row.get(1)?;
                                    let le: i64 = row.get(2)?;
                                    Ok((fp, ls, le))
                                }) {
                                    ref_val = json!({
                                        "ref": r,
                                        "verified": true,
                                        "type": "symbol",
                                        "location": format!("{}:{}-{}", s_row.0, s_row.1, s_row.2)
                                    });
                                }
                            }
                        }
                    }
                    resolved_refs.push(ref_val);
                }

                notes_json.push(json!({
                    "id": id,
                    "store": store_name,
                    "key": key,
                    "rev": rev,
                    "kind": kind,
                    "scope": scope,
                    "session": session,
                    "body": body,
                    "tags": tags,
                    "refs": refs_list,
                    "resolved_refs": resolved_refs,
                    "content_hash": format!("{content_hash:x}"),
                    "source": source,
                    "created_at": created_at,
                    "expires_at": expires_at,
                    "is_current": is_current,
                    "superseded_by": superseded_by,
                    "superseded_at": superseded_at,
                    "prev_category": prev_cat,
                    "prev_title": prev_title,
                    "prev_detail": prev_detail,
                    "recall_count": recall_count,
                    "last_recalled_ts": last_recalled_ts,
                }));
            }
        }
    }

    Ok(notes_json)
}
