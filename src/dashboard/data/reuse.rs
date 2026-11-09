//! Anti-reinvention candidate collection for the dashboard reuse panel.
//! Scans every known repo index with the D1-D4 detectors and renders
//! candidates with snippets and prewrite-search telemetry.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::{json, Value};

use crate::error::Result;
use crate::query::reuse::{DetectorId, ReuseCandidate};

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
    let mut total_corpus_candidates = 0;
    for (r_name, r_root) in &repos {
        total_corpus_candidates += collect_repo_candidates(r_name, r_root, &mut candidates_json);
    }

    let prewrite_searches = collect_prewrite_searches(entries);
    let prewrite_count = entries
        .iter()
        .filter(|e| {
            matches!(
                e.get("tool").and_then(Value::as_str),
                Some("search") | Some("reuse")
            )
        })
        .count();

    let summary =
        summarize_reuse_metrics(&candidates_json, total_corpus_candidates, prewrite_count);
    Ok((candidates_json, summary, prewrite_searches))
}

fn collect_repo_candidates(r_name: &str, r_root: &Path, out: &mut Vec<Value>) -> usize {
    let db_path = r_root.join(".engram").join("index.db");
    if !db_path.is_file() {
        return 0;
    }
    let conn =
        match Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(c) => c,
            Err(_) => return 0,
        };
    let (candidates, total) = match crate::query::reuse::analyze_corpus(&conn, None, None, 25) {
        Ok(res) => res,
        Err(_) => return 0,
    };
    for c in candidates {
        out.push(format_candidate_json(r_name, r_root, &conn, &c));
    }
    total
}

fn format_detector_metadata(c: &ReuseCandidate) -> (&'static str, String, String) {
    let (title, summary) = match c.detector {
        DetectorId::D1 => (
            "D1 Callee Footprint Overlap",
            format!("Workflow '{}' calls identical downstream functions as canonical '{}' ([callers={}]).", c.symbol, c.canonical, c.canonical_callers),
        ),
        DetectorId::D2 => (
            "D2 Interface Token Overlap",
            format!("Signature and parameters of '{}' duplicate canonical '{}' ([callers={}]).", c.symbol, c.canonical, c.canonical_callers),
        ),
        DetectorId::D3 => (
            "D3 Raw Client Bypass",
            format!("Symbol '{}' calls raw external clients directly instead of canonical '{}' ([callers={}]).", c.symbol, c.canonical, c.canonical_callers),
        ),
        DetectorId::D4 => (
            "D4 Redundant Pass-Through Wrapper",
            format!("Single-caller wrapper '{}' adds zero logic around canonical '{}' ([callers={}]).", c.symbol, c.canonical, c.canonical_callers),
        ),
    };

    let evidence_kind = match c.detector {
        DetectorId::D1 => "shared callees",
        DetectorId::D2 => "shared tokens",
        DetectorId::D3 => "client bypass",
        DetectorId::D4 => "details",
    };
    let shared = if c.shared_items.is_empty() {
        String::new()
    } else {
        format!("{}: [{}]", evidence_kind, c.shared_items.join(", "))
    };

    (title, summary, shared)
}

fn resolve_candidate_snippets(
    r_root: &Path,
    conn: &Connection,
    c: &ReuseCandidate,
) -> (Loc, Loc, String, String) {
    let cand_loc = symbol_loc(conn, &c.symbol, Loc::estimated(c.file.clone(), c.line));
    let can_loc = symbol_loc(
        conn,
        &c.canonical,
        Loc::estimated(c.canonical_file.clone(), c.canonical_line),
    );
    let cand_snippet = read_snippet(
        &r_root.join(&cand_loc.file),
        cand_loc.start,
        cand_loc.end,
        30,
    );
    let can_snippet = read_snippet(&r_root.join(&can_loc.file), can_loc.start, can_loc.end, 30);
    (cand_loc, can_loc, cand_snippet, can_snippet)
}

fn format_candidate_json(
    r_name: &str,
    r_root: &Path,
    conn: &Connection,
    c: &ReuseCandidate,
) -> Value {
    let (det_title, det_summary, shared_evidence) = format_detector_metadata(c);
    let (cand_loc, can_loc, cand_snippet, can_snippet) =
        resolve_candidate_snippets(r_root, conn, c);

    let loc_saving = cand_loc.loc;
    let narrative = format!(
        "Consolidating '{}' into authoritative '{}' ([callers={}]) deletes ~{} duplicate lines and eliminates drift between twin implementations.",
        c.symbol, c.canonical, c.canonical_callers, loc_saving
    );

    json!({
        "id": format!("{}:{}->{}", r_name, c.symbol, c.canonical),
        "repo": r_name,
        "detector": c.detector.as_str(),
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
    })
}

fn collect_prewrite_searches(entries: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for e in entries.iter().rev() {
        let tool = e.get("tool").and_then(Value::as_str).unwrap_or("");
        if tool != "search" && tool != "reuse" {
            continue;
        }
        let args = e.get("args").cloned().unwrap_or(Value::Null);
        let terms = args
            .get("terms")
            .or_else(|| args.get("paths"))
            .and_then(Value::as_str)
            .unwrap_or("");
        out.push(json!({
            "ts": e.get("ts").unwrap_or(&Value::Null),
            "repo_name": e.get("repo_name").unwrap_or(&Value::Null),
            "tool": tool,
            "terms": terms,
            "shown": e.get("shown").or_else(|| e.get("result_count")).unwrap_or(&json!(0)),
            "latency_us": e.get("latency_us").unwrap_or(&json!(0)),
        }));
        if out.len() >= 40 {
            break;
        }
    }
    out
}

fn summarize_reuse_metrics(
    candidates_json: &[Value],
    total_corpus_candidates: usize,
    prewrite_count: usize,
) -> Value {
    let sample_cands = candidates_json.len();
    let total_cands = total_corpus_candidates.max(sample_cands);
    let sample_loc: i64 = candidates_json
        .iter()
        .map(|c| c.get("loc_saving").and_then(Value::as_i64).unwrap_or(0))
        .sum();
    let prev_loc = if sample_cands > 0 && total_cands > sample_cands {
        (sample_loc as f64 * (total_cands as f64 / sample_cands as f64)).round() as i64
    } else {
        sample_loc
    };
    let avg_callers = if sample_cands > 0 {
        let callers_sum: i64 = candidates_json
            .iter()
            .map(|c| {
                c.get("canonical_callers")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
            })
            .sum();
        callers_sum as f64 / sample_cands as f64
    } else {
        0.0
    };

    json!({
        "total_candidates": total_cands,
        "sample_candidates": sample_cands,
        "preventable_loc": prev_loc,
        "prewrite_count": prewrite_count,
        "avg_callers": (avg_callers * 10.0).round() / 10.0,
        "detector_counts": {
            "D1": candidates_json.iter().filter(|c| c.get("detector") == Some(&json!("D1"))).count(),
            "D2": candidates_json.iter().filter(|c| c.get("detector") == Some(&json!("D2"))).count(),
            "D3": candidates_json.iter().filter(|c| c.get("detector") == Some(&json!("D3"))).count(),
            "D4": candidates_json.iter().filter(|c| c.get("detector") == Some(&json!("D4"))).count(),
        }
    })
}
