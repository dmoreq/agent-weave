//! Anti-reinvention candidate collection for the dashboard reuse panel.
//! Scans every known repo index with the D1-D4 detectors and renders
//! candidates with snippets and prewrite-search telemetry.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::{json, Value};

use crate::error::Result;

use super::discover_repos;
use super::read_snippet;

/// A resolved source location for a reuse candidate or its canonical twin:
/// backed by the symbol index when the qualname exists, an estimate from the
/// extraction header otherwise.
struct Loc {
    file: String,
    start: i64,
    end: i64,
    loc: i64,
}

impl Loc {
    /// Estimate for a symbol with no index row: the extraction header plus a
    /// 10-line body assumption.
    fn estimated(file: String, line: i64) -> Self {
        Self {
            file,
            start: line,
            end: line + 10,
            loc: 10,
        }
    }
}

/// Resolve one qualname's location from the index (exact match, then suffix
/// match), falling back to `estimate` when the symbols table has no row.
fn symbol_loc(conn: &Connection, qualname: &str, estimate: Loc) -> Loc {
    let Ok(mut stmt) = conn.prepare(
        "SELECT file_path, line_start, line_end FROM symbols WHERE qualname = ?1 OR qualname LIKE ?2 LIMIT 1",
    ) else {
        return estimate;
    };
    match stmt.query_row(rusqlite::params![qualname, format!("%{qualname}")], |r| {
        let fp: String = r.get(0)?;
        let ls: i64 = r.get(1)?;
        let le: i64 = r.get(2)?;
        Ok((fp, ls, le))
    }) {
        Ok((file, start, end)) => Loc {
            loc: (end - start + 1).max(1),
            file,
            start,
            end,
        },
        Err(_) => estimate,
    }
}

pub fn collect_reuse_candidates(
    root: Option<&Path>,
    entries: &[Value],
) -> Result<(Vec<Value>, Value, Vec<Value>)> {
    let mut repos: HashMap<String, PathBuf> = HashMap::new();
    discover_repos(root, entries, "index.db", &mut repos);

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
            let cand_loc = symbol_loc(&conn, &c.symbol, Loc::estimated(c.file.clone(), c.line));
            let can_loc = symbol_loc(
                &conn,
                &c.canonical,
                Loc::estimated(c.canonical_file.clone(), c.canonical_line),
            );
            let cand_snippet = read_snippet(
                &r_root.join(&cand_loc.file),
                cand_loc.start,
                cand_loc.end,
                30,
            );
            let can_snippet =
                read_snippet(&r_root.join(&can_loc.file), can_loc.start, can_loc.end, 30);

            let loc_saving = cand_loc.loc;
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
                "candidate_info": json!({
                    "file": cand_loc.file,
                    "line_start": cand_loc.start,
                    "line_end": cand_loc.end,
                    "loc": cand_loc.loc,
                }),
                "canonical_info": json!({
                    "file": can_loc.file,
                    "line_start": can_loc.start,
                    "line_end": can_loc.end,
                    "loc": can_loc.loc,
                }),
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
