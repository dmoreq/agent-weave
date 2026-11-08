//! Anti-reinvention candidate collection for the dashboard reuse panel.
//! Scans every known repo index with the D1-D4 detectors and renders
//! candidates with snippets and prewrite-search telemetry.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::{json, Value};

use crate::error::Result;

use super::read_snippet;

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
