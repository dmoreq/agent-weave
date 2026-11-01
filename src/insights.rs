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
use crate::error::{KgError, Result};

pub fn stats(conn: &Connection, root: &Path) -> Result<String> {
    let q = |sql: &str| -> Result<i64> { Ok(conn.query_row(sql, [], |r| r.get(0))?) };
    let files = q("SELECT COUNT(*) FROM files")?;
    let symbols = q("SELECT COUNT(*) FROM symbols")?;
    let edges = q("SELECT COUNT(*) FROM edges")?;
    let calls = q("SELECT COUNT(*) FROM edges WHERE kind = 'calls'")?;
    let imports = q("SELECT COUNT(*) FROM edges WHERE kind = 'imports'")?;
    let inherits = q("SELECT COUNT(*) FROM edges WHERE kind = 'inherits'")?;
    let perr = q("SELECT COALESCE(SUM(parse_status != 'ok'), 0) FROM files")?;
    let unresolved: i64 = conn
        .query_row(
            "SELECT COALESCE(CAST(value AS INTEGER), 0) FROM meta WHERE key = 'unresolved_callee_count'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let size = std::fs::metadata(db::db_path(root)).map(|m| m.len()).unwrap_or(0);
    let kg_version: String = conn
        .query_row("SELECT value FROM meta WHERE key = 'kg_version'", [], |r| r.get(0))
        .unwrap_or_else(|_| "?".into());
    let grammar: String = conn
        .query_row("SELECT value FROM meta WHERE key = 'grammar_version'", [], |r| r.get(0))
        .unwrap_or_else(|_| "?".into());
    Ok(format!(
        "kg stats: files={files} symbols={symbols} edges={edges} (calls={calls} imports={imports} inherits={inherits}) parse_errors={perr} unresolved={unresolved} db_bytes={size} kg={kg_version} grammar={grammar}"
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

pub fn hotspots(conn: &Connection, root: &Path, limit: usize) -> Result<Vec<String>> {
    if limit == 0 {
        return Err(KgError::Usage("--limit must be >= 1".to_string()));
    }
    // Amendment 11d: loc/cyclomatic are stored per file at build time —
    // hotspots is a single scan + git log, not a whole-repo re-parse.
    let mut stmt = conn.prepare("SELECT path, loc, cyclomatic FROM files")?;
    let rows: Vec<(String, i64, i64)> = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if rows.is_empty() {
        return Err(KgError::NoResults("no indexed files".to_string()));
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
                s.score, s.rel, s.loc as usize, s.cyclomatic as usize, s.churn as usize, s.fixes as usize
            )
        })
        .collect())
}

pub fn report(conn: &Connection, root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    // Section 1: index overview.
    out.push(format!("## 1. index overview\n{}", stats(conn, root)?));
    // Section 2: architecture map — top packages by symbol count.
    out.push("## 2. architecture map (top packages by symbols)".to_string());
    let mut stmt = conn.prepare(
        "SELECT SUBSTR(f.path, 1, INSTR(f.path || '/', '/') - 1) AS pkg, COUNT(*) AS n
         FROM symbols s JOIN files f ON f.id = s.file_id
         GROUP BY pkg ORDER BY n DESC LIMIT 8",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (pkg, n) in rows {
        out.push(format!("  {pkg}: {n} symbols"));
    }
    // Section 3: hotspots top-5.
    out.push("## 3. hotspots (top 5)".to_string());
    for line in hotspots(conn, root, 5)? {
        out.push(format!("  {line}"));
    }
    // Section 4: most-called symbols (fan-in top 8).
    out.push("## 4. most-called symbols (fan-in top 8)".to_string());
    let mut stmt = conn.prepare(
        "SELECT ds.qualname, COUNT(*) AS n
         FROM edges e JOIN symbols ds ON ds.id = e.dst_id
         WHERE e.kind = 'calls' AND e.dst_id IS NOT NULL
         GROUP BY e.dst_id ORDER BY n DESC LIMIT 8",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (qn, n) in rows {
        out.push(format!("  {qn}: {n} callers"));
    }
    Ok(out)
}
