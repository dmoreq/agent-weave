//! P1.5 insight commands: `stats`, `hotspots`, `report`.
//!
//! hotspots score (spec G7): cyclomatic-proxy + LOC + churn-90d + fix-density,
//! each min-max normalized across the indexed set, summed. Cyclomatic proxy =
//! decision-node count (if/for/while/boolean_operator/except/match), computed
//! once per file at build time (Amendment 11d: files.loc / files.cyclomatic) —
//! no per-invocation re-parse. Churn/fix counts come from `git log` on the
//! repo (read-only).

use std::collections::HashMap;
use std::path::Path;

use rusqlite::Connection;

use crate::db;
use crate::error::{AwError, Result};

pub fn stats(conn: &Connection, root: &Path) -> Result<String> {
    let q = |sql: &str| -> Result<i64> { Ok(conn.query_row(sql, [], |r| r.get(0))?) };
    // Build-time counters first (R7 L4): every filtered COUNT scanned rows;
    // the manifest already stores the authoritative totals (written from SQL
    // at build end), so stats is O(1). Fallback keeps pre-counter indexes
    // working.
    let meta_int =
        |key: &str| -> Option<i64> { db::meta_get(conn, key).and_then(|v| v.parse::<i64>().ok()) };
    // `unwrap_or` would evaluate the COUNT eagerly, defeating O(1); resolve
    // only when the manifest key is absent (R7 review).
    let count_or_meta = |key: &str, sql: &str| -> Result<i64> {
        match meta_int(key) {
            Some(v) => Ok(v),
            None => q(sql),
        }
    };
    let files = count_or_meta("file_count", "SELECT COUNT(*) FROM files")?;
    let generated = q("SELECT COUNT(*) FROM files WHERE generated = 1")?;
    let symbols = count_or_meta("symbol_count", "SELECT COUNT(*) FROM symbols")?;
    let edges = count_or_meta("edge_count", "SELECT COUNT(*) FROM edges")?;
    let calls = count_or_meta(
        "edge_calls",
        "SELECT COUNT(*) FROM edges WHERE kind = 'calls'",
    )?;
    let imports = count_or_meta(
        "edge_imports",
        "SELECT COUNT(*) FROM edges WHERE kind = 'imports'",
    )?;
    let inherits = count_or_meta(
        "edge_inherits",
        "SELECT COUNT(*) FROM edges WHERE kind = 'inherits'",
    )?;
    let perr = count_or_meta(
        "parse_error_count",
        "SELECT COALESCE(SUM(parse_status != 'ok'), 0) FROM files",
    )?;
    let unresolved = meta_int("unresolved_callee_count").unwrap_or(0);
    let size = std::fs::metadata(db::db_path(root))
        .map(|m| m.len())
        .unwrap_or(0);
    let aw_version: String = conn
        .query_row("SELECT value FROM meta WHERE key = 'aw_version'", [], |r| {
            r.get(0)
        })
        .unwrap_or_else(|_| "?".into());
    let grammar: String = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'grammar_version'",
            [],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "?".into());
    Ok(format!(
        "aw stats: files={files} generated={generated} symbols={symbols} edges={edges} (calls={calls} imports={imports} inherits={inherits}) parse_errors={perr} unresolved={unresolved} db_bytes={size} aw={aw_version} grammar={grammar}"
    ))
}

struct FileScore {
    rel: String,
    loc: f64,
    cyclomatic: f64,
    churn: f64,
    fixes: f64,
    score: f64,
}

fn is_source_path(line: &str) -> bool {
    line.rsplit('.')
        .next()
        .is_some_and(|ext| crate::walk::SOURCE_EXTS.contains(&ext))
}

fn git_counts(root: &Path) -> Result<(HashMap<String, f64>, HashMap<String, f64>)> {
    // Non-git repos (or git failures) degrade to zero churn/fix signal —
    // hotspots stays useful, just LOC+cyc-driven.
    let run = |args: &[&str]| -> Option<String> {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let Some(log_all) = run(&["log", "--since=90.days", "--name-only", "--pretty=format:"]) else {
        return Ok((HashMap::new(), HashMap::new()));
    };
    let Some(log_fix) = run(&[
        "log",
        "--since=180.days",
        "--grep=fix\\|bug\\|crash\\|wrong\\|incorrect\\|sai",
        "-i",
        "--name-only",
        "--pretty=format:",
    ]) else {
        return Ok((HashMap::new(), HashMap::new()));
    };
    let mut churn = HashMap::new();
    for line in log_all.lines().filter(|l| is_source_path(l)) {
        *churn.entry(line.to_string()).or_insert(0.0) += 1.0;
    }
    let mut fixes = HashMap::new();
    for line in log_fix.lines().filter(|l| is_source_path(l)) {
        *fixes.entry(line.to_string()).or_insert(0.0) += 1.0;
    }
    Ok((churn, fixes))
}

fn normalize(v: f64, lo: f64, hi: f64) -> f64 {
    if hi <= lo {
        0.0
    } else {
        (v - lo) / (hi - lo)
    }
}

pub fn hotspots(
    conn: &Connection,
    root: &Path,
    limit: usize,
    path: Option<&str>,
) -> Result<Vec<String>> {
    if limit == 0 {
        return Err(AwError::Usage("--limit must be >= 1".to_string()));
    }
    if let Some(p) = path {
        crate::query::validate_path_scope(p, false)?;
    }
    // Amendment 11d: loc/cyclomatic are stored per file at build time —
    // hotspots is a single scan + git log, not a whole-repo re-parse.
    let scope = crate::query::path_scope_sql_pub("f", path);
    let mut stmt = conn.prepare(&format!(
        "SELECT f.path, f.loc, f.cyclomatic FROM files f WHERE 1=1 {scope}"
    ))?;
    let rows: Vec<(String, i64, i64)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if rows.is_empty() {
        return Err(AwError::NoResults("no indexed files".to_string()));
    }
    let (churn, fixes) = git_counts(root)?;

    let mut scores: Vec<FileScore> = Vec::with_capacity(rows.len());
    for (rel, loc, cyc) in rows {
        let churn_n = churn.get(&rel).copied().unwrap_or(0.0);
        let fixes_n = fixes.get(&rel).copied().unwrap_or(0.0);
        scores.push(FileScore {
            rel,
            loc: loc as f64,
            cyclomatic: cyc as f64,
            churn: churn_n,
            fixes: fixes_n,
            score: 0.0,
        });
    }
    // Per-metric min-max bounds (R1.5 fix): pooling one (lo, hi) across all
    // four metrics let LOC swamp churn/fix and the score stopped meaning
    // what the output claims.
    fn bounds(scores: &[FileScore], get: impl Fn(&FileScore) -> f64) -> (f64, f64) {
        let mut lo = f64::MAX;
        let mut hi = f64::MIN;
        for s in scores {
            let v = get(s);
            lo = lo.min(v);
            hi = hi.max(v);
        }
        (lo, hi)
    }
    let (loc_lo, loc_hi) = bounds(&scores, |s| s.loc);
    let (cyc_lo, cyc_hi) = bounds(&scores, |s| s.cyclomatic);
    let (ch_lo, ch_hi) = bounds(&scores, |s| s.churn);
    let (fx_lo, fx_hi) = bounds(&scores, |s| s.fixes);
    for s in &mut scores {
        s.score = normalize(s.loc, loc_lo, loc_hi)
            + normalize(s.cyclomatic, cyc_lo, cyc_hi)
            + normalize(s.churn, ch_lo, ch_hi)
            + normalize(s.fixes, fx_lo, fx_hi);
    }
    scores.sort_by(|a, b| b.score.total_cmp(&a.score));
    Ok(scores
        .into_iter()
        .take(limit.min(100))
        .map(|s| {
            format!(
                "{:.3} {} (loc={} cyc={} churn90={} fix180={})",
                s.score,
                s.rel,
                s.loc as usize,
                s.cyclomatic as usize,
                s.churn as usize,
                s.fixes as usize
            )
        })
        .collect())
}

pub fn report(conn: &Connection, root: &Path, path: Option<&str>) -> Result<Vec<String>> {
    if let Some(p) = path {
        crate::query::validate_path_scope(p, false)?;
    }
    let mut out = Vec::new();
    // Freshness (R7 L4): the report is a build snapshot, but agents read it
    // as current. Check a bounded sample of indexed files and say so.
    let scope = crate::query::path_scope_sql_pub("f", path);
    let mut stmt = conn.prepare(&format!(
        "SELECT f.path FROM files f WHERE 1=1 {scope} LIMIT 5000"
    ))?;
    let paths = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let changed = paths
        .iter()
        .filter(|rel| crate::query::file_changed(conn, root, rel))
        .count();
    if changed > 0 {
        out.push(format!(
            "aw: {changed} of {} sampled files changed since build — run aw build",
            paths.len()
        ));
    }
    // Section 1: index overview.
    out.push(format!("## 1. index overview\n{}", stats(conn, root)?));
    if let Some(scope) = path {
        let scope = scope
            .trim_start_matches("./")
            .trim_end_matches('/')
            .replace('\'', "''");
        let scoped_files: i64 = conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM files f WHERE f.path = '{scope}' OR substr(f.path, 1, length('{scope}') + 1) = '{scope}' || '/'"
            ),
            [],
            |r| r.get(0),
        )?;
        let scoped_symbols: i64 = conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM symbols s JOIN files f ON f.id = s.file_id WHERE f.path = '{scope}' OR substr(f.path, 1, length('{scope}') + 1) = '{scope}' || '/'"
            ),
            [],
            |r| r.get(0),
        )?;
        out.push(format!(
            "  scoped files={scoped_files} symbols={scoped_symbols}"
        ));
    }
    // Section 2: architecture map — top packages by symbol count.
    out.push("## 2. architecture map (top packages by symbols)".to_string());
    let mut stmt = conn.prepare(&format!(
        "SELECT SUBSTR(f.path, 1, INSTR(f.path || '/', '/') - 1) AS pkg, COUNT(*) AS n
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE 1=1 {scope}
         GROUP BY pkg ORDER BY n DESC LIMIT 8"
    ))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (pkg, n) in rows {
        out.push(format!("  {pkg}: {n} symbols"));
    }
    // Section 3: hotspots top-5.
    out.push("## 3. hotspots (top 5)".to_string());
    for line in hotspots(conn, root, 5, path)? {
        out.push(format!("  {line}"));
    }
    // Section 4: most-called symbols (fan-in top 8).
    out.push("## 4. most-called symbols (fan-in top 8)".to_string());
    let fan_scope = crate::query::path_scope_sql_pub("df", path);
    let mut stmt = conn.prepare(&format!(
        "SELECT ds.qualname, COUNT(*) AS n
         FROM edges e JOIN symbols ds ON ds.id = e.dst_id
         JOIN files df ON df.id = ds.file_id
         WHERE e.kind = 'calls' AND e.dst_id IS NOT NULL {fan_scope}
         GROUP BY e.dst_id ORDER BY n DESC LIMIT 8"
    ))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (qn, n) in rows {
        out.push(format!("  {qn}: {n} callers"));
    }
    Ok(out)
}
