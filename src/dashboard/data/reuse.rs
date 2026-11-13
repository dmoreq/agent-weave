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

mod usage;
use super::read_snippet;
use usage::{aggregate_candidate_usage, decorate_usage};

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

/// One cached detector scan: candidates plus the corpus-wide total, keyed by
/// the index files it read. The journal is NOT part of the key — prewrite
/// metrics are recomputed per request — so appending a journal line never
/// forces a 2.2 s rescan.
struct ScanCache {
    key: u64,
    computed_at: std::time::Instant,
    candidates: Vec<Value>,
    total_candidates: usize,
}

fn scan_key(repos: &HashMap<String, PathBuf>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut items: Vec<(String, u64, u64)> = repos
        .values()
        .map(|root| {
            let db = root.join(".engram").join("index.db");
            let meta = std::fs::metadata(&db);
            let mtime = meta
                .as_ref()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            let len = meta.map(|m| m.len()).unwrap_or(0);
            (db.to_string_lossy().into_owned(), mtime, len)
        })
        .collect();
    items.sort();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    items.hash(&mut hasher);
    hasher.finish()
}

fn cache_enabled() -> bool {
    std::env::var("ENGRAM_DASHBOARD_NO_CACHE").as_deref() != Ok("1")
}

fn cached_scan(repos: &HashMap<String, PathBuf>) -> ((Vec<Value>, usize), bool, u64) {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<Option<ScanCache>>> = OnceLock::new();
    let cell = CACHE.get_or_init(|| Mutex::new(None));
    let key = scan_key(repos);
    if cache_enabled() {
        if let Ok(guard) = cell.lock() {
            if let Some(cached) = guard.as_ref().filter(|c| c.key == key) {
                return (
                    (cached.candidates.clone(), cached.total_candidates),
                    true,
                    cached.computed_at.elapsed().as_millis() as u64,
                );
            }
        }
    }
    // Compute outside the lock; publish only a complete scan.
    let mut candidates_json = Vec::new();
    let mut total = 0usize;
    for (r_name, r_root) in repos {
        total += collect_repo_candidates(r_name, r_root, &mut candidates_json);
    }
    if cache_enabled() {
        if let Ok(mut guard) = cell.lock() {
            *guard = Some(ScanCache {
                key,
                computed_at: std::time::Instant::now(),
                candidates: candidates_json.clone(),
                total_candidates: total,
            });
        }
    }
    ((candidates_json, total), false, 0)
}

/// Attach usage from the current journal to cached candidates. The scan cache
/// is keyed on index files only, so usage is applied after the cache on every
/// request instead of being frozen into the cached JSON.
pub fn collect_reuse_candidates(
    root: Option<&Path>,
    entries: &[Value],
) -> Result<(Vec<Value>, Value, Vec<Value>)> {
    let mut repos: HashMap<String, PathBuf> = HashMap::new();
    discover_repos(root, entries, "index.db", &mut repos);
    let (errors, blocked) = super::retain_untracked(&mut repos);

    let usage = aggregate_candidate_usage(entries);
    let ((mut candidates_json, total_corpus_candidates), cache_hit, cache_age_ms) =
        cached_scan(&repos);
    decorate_usage(&mut candidates_json, &usage);

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

    let mut summary =
        summarize_reuse_metrics(&candidates_json, total_corpus_candidates, prewrite_count);
    summary["cache_hit"] = json!(cache_hit);
    summary["cache_age_ms"] = json!(cache_age_ms);
    summary["degraded"] = json!(blocked > 0);
    summary["errors"] = json!(errors);
    Ok((candidates_json, summary, prewrite_searches))
}

fn collect_repo_candidates(r_name: &str, r_root: &Path, out: &mut Vec<Value>) -> usize {
    // The dashboard opens index.db directly; apply the same provenance gate as
    // every CLI/MCP query so a tracked, attacker-authored index is not read.
    if crate::db::reject_tracked_eg_dir(r_root).is_err() {
        return 0;
    }
    let db_path = r_root.join(".engram").join("index.db");
    if crate::db::reject_symlink(&r_root.join(".engram"), ".engram directory").is_err()
        || crate::db::reject_symlink(&db_path, "index.db").is_err()
        || !db_path.is_file()
    {
        return 0;
    }
    let conn =
        match Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(c) => c,
            Err(_) => return 0,
        };
    let (candidates, total, _) =
        match crate::query::reuse::analyze_corpus(&conn, None, None, 25, false) {
            Ok(res) => res,
            Err(_) => return 0,
        };
    for c in candidates {
        out.push(format_candidate_json(r_name, r_root, &conn, &c));
    }
    total
}

/// Evidence chip text for one candidate: the detector-specific noun plus the
/// shared items. The narrative title/summary fields were removed from the
/// payload; the UI derives its label from `detector`.
fn format_shared_evidence(c: &ReuseCandidate) -> String {
    let evidence_kind = match c.detector {
        DetectorId::D1 => "shared callees",
        DetectorId::D2 => "shared tokens",
        DetectorId::D3 => "client bypass",
        DetectorId::D4 => "details",
        DetectorId::D5 => "class methods",
        DetectorId::D1L => "unresolved callees",
        DetectorId::DT => "shared test tokens",
    };
    if c.detector == DetectorId::D3 {
        return c
            .raw_client
            .as_deref()
            .map(|raw| format!("client bypass: [{raw}]"))
            .unwrap_or_default();
    }
    if c.shared_items.is_empty() {
        String::new()
    } else {
        format!("{}: [{}]", evidence_kind, c.shared_items.join(", "))
    }
}

/// Resolve one snippet path inside the repository root. Absolute paths and
/// `..` components come from the index and must never reach a file read.
fn contained_path(root: &Path, rel: &str) -> Option<std::path::PathBuf> {
    let p = Path::new(rel);
    if p.is_absolute() || rel.split('/').any(|c| c == "..") {
        return None;
    }
    let joined = root.join(p);
    let canon_root = std::fs::canonicalize(root).ok()?;
    let canon = std::fs::canonicalize(&joined).ok()?;
    canon.starts_with(&canon_root).then_some(canon)
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
    let cand_snippet = contained_path(r_root, &cand_loc.file)
        .map(|p| read_snippet(&p, cand_loc.start, cand_loc.end, 30))
        .unwrap_or_default();
    let can_snippet = contained_path(r_root, &can_loc.file)
        .map(|p| read_snippet(&p, can_loc.start, can_loc.end, 30))
        .unwrap_or_default();
    (cand_loc, can_loc, cand_snippet, can_snippet)
}

fn format_candidate_json(
    r_name: &str,
    r_root: &Path,
    conn: &Connection,
    c: &ReuseCandidate,
) -> Value {
    let shared_evidence = format_shared_evidence(c);
    let (cand_loc, can_loc, cand_snippet, can_snippet) =
        resolve_candidate_snippets(r_root, conn, c);
    let ctx = crate::query::QueryCtx { conn, root: r_root };
    let stale = crate::query::file_changed(&ctx, &c.file)
        || crate::query::file_changed(&ctx, &c.canonical_file);
    let confidence = if stale {
        crate::query::reuse::ReuseConfidence::Low
    } else {
        c.confidence
    };
    let mut reasons: Vec<&str> = c.reasons.iter().map(|r| r.as_str()).collect();
    if stale {
        reasons.push("stale-index");
    }

    let loc_saving = cand_loc.loc;
    let symbol = crate::query::reuse::sanitize(&c.symbol);
    let canonical = crate::query::reuse::sanitize(&c.canonical);
    let narrative = format!(
        "Consolidating '{}' into authoritative '{}' ([callers={}]) deletes ~{} duplicate lines and eliminates drift between twin implementations.",
        symbol, canonical, c.canonical_callers, loc_saving
    );

    json!({
        "id": format!("{}:{}->{}", r_name, symbol, canonical),
        "repo": r_name,
        "detector": c.detector.as_str(),
        "symbol": symbol,
        "canonical": canonical,
        "canonical_callers": c.canonical_callers,
        "canonical_file": crate::query::reuse::sanitize(&c.canonical_file),
        "canonical_line": c.canonical_line,
        // D3/D4 emit no overlap metric: their match is an evidence class, not
        // a measured percentage. The UI renders a label from `detector`.
        "overlap_pct": c.overlap_pct,
        "overlap_applicable": c.overlap_pct.is_some(),
        "confidence": confidence.as_str(),
        "reasons": reasons,
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
        "repo_path": r_root.to_string_lossy(),
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

fn candidate_reason_counts(candidates_json: &[Value]) -> Value {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for candidate in candidates_json {
        if let Some(reasons) = candidate.get("reasons").and_then(Value::as_array) {
            for reason in reasons.iter().filter_map(Value::as_str) {
                *counts.entry(reason.to_string()).or_default() += 1;
            }
        }
    }
    Value::Object(
        counts
            .into_iter()
            .map(|(key, count)| (key, json!(count)))
            .collect(),
    )
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
    let avg_callers = if sample_cands > 0 {
        let callers_sum: i64 = candidates_json
            .iter()
            .map(|c| {
                c.get("canonical_callers")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
            })
            .sum();
        (callers_sum as f64 / sample_cands as f64 * 10.0).round() / 10.0
    } else {
        0.0
    };
    let prev_loc = if sample_cands > 0 && total_cands > sample_cands {
        (sample_loc as f64 * (total_cands as f64 / sample_cands as f64)).round() as i64
    } else {
        sample_loc
    };

    json!({
        "schema_version": 2,
        "total_candidates": total_cands,
        "sample_strategy": "top_overlap_per_repo",
        // Extrapolated from the sampled candidates; flag it in-band so a
        // consumer cannot quote it as a measured figure.
        "preventable_loc": prev_loc,
        "preventable_loc_estimated": true,
        "prewrite_count": prewrite_count,
        "avg_callers": avg_callers,
        "avg_callers_estimated": true,
        // Counts come from the sampled candidate list, not the whole corpus.
        "detector_counts": {
            "D1": candidates_json.iter().filter(|c| c.get("detector") == Some(&json!("D1"))).count(),
            "D2": candidates_json.iter().filter(|c| c.get("detector") == Some(&json!("D2"))).count(),
            "D3": candidates_json.iter().filter(|c| c.get("detector") == Some(&json!("D3"))).count(),
            "D4": candidates_json.iter().filter(|c| c.get("detector") == Some(&json!("D4"))).count(),
        },
        "detector_counts_estimated": true,
        "confidence_counts": {
            "low": candidates_json.iter().filter(|c| c.get("confidence") == Some(&json!("low"))).count(),
            "provisional": candidates_json.iter().filter(|c| c.get("confidence") == Some(&json!("provisional"))).count(),
            "medium": candidates_json.iter().filter(|c| c.get("confidence") == Some(&json!("medium"))).count(),
            "high": candidates_json.iter().filter(|c| c.get("confidence") == Some(&json!("high"))).count(),
        },
        "reason_counts": candidate_reason_counts(candidates_json),
        "confidence_counts_estimated": true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reuse_scan_cache_hits_until_the_index_changes() {
        let dir = crate::db::test_support::temp_root("reuse-scan-cache");
        let repo = dir.join("repo");
        crate::db::test_support::make_dir_all(repo.join(".engram")).unwrap();
        crate::db::test_support::write_file(repo.join(".engram").join("index.db"), b"").unwrap();
        let mut repos: HashMap<String, PathBuf> = HashMap::new();
        repos.insert("repo".to_string(), repo.clone());

        let (_, first_hit, _) = cached_scan(&repos);
        let (_, second_hit, _) = cached_scan(&repos);
        assert!(!first_hit, "first scan must compute");
        assert!(second_hit, "second scan with an unchanged index must hit");

        // A changed index mtime invalidates the cache.
        std::thread::sleep(std::time::Duration::from_millis(5));
        crate::db::test_support::write_file(repo.join(".engram").join("index.db"), b"x").unwrap();
        let (_, third_hit, _) = cached_scan(&repos);
        assert!(!third_hit, "changed index must recompute");
        crate::db::test_support::remove_tree(&dir);
    }

    #[test]
    fn summary_emits_sample_markers_and_estimate_flags() {
        let summary = summarize_reuse_metrics(&[], 0, 0);
        assert_eq!(summary["schema_version"], 2);
        assert_eq!(summary["total_candidates"], 0);
        assert_eq!(summary["prewrite_count"], 0);
        assert_eq!(summary["sample_strategy"], "top_overlap_per_repo");
        assert_eq!(summary["preventable_loc_estimated"], true);
        assert_eq!(summary["detector_counts_estimated"], true);
        assert!(summary.get("judged_calls").is_none());
        assert_eq!(summary["avg_callers"], 0.0);
        assert_eq!(summary["avg_callers_estimated"], true);
    }
}

#[cfg(test)]
mod containment_tests {
    use super::*;

    #[test]
    fn snippet_paths_stay_inside_the_repository_root() {
        let root = crate::db::test_support::temp_root("reuse-contain");
        crate::db::test_support::write_file(root.join("ok.rs"), b"fn a() {}\n").unwrap();
        crate::db::test_support::make_dir_all(root.join("src")).unwrap();
        crate::db::test_support::write_file(root.join("src").join("in.rs"), b"fn b() {}\n")
            .unwrap();

        assert!(contained_path(&root, "ok.rs").is_some());
        assert!(contained_path(&root, "src/in.rs").is_some());
        // Escapes and absolute paths never resolve.
        assert!(contained_path(&root, "../outside.rs").is_none());
        assert!(contained_path(&root, "/etc/passwd").is_none());
        assert!(contained_path(&root, "src/../../outside.rs").is_none());
        crate::db::test_support::remove_tree(&root);
    }
}
