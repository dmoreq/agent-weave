//! P1.5 insight commands: `stats`, `hotspots`, `report`.
//!
//! hotspots score (spec G7): cyclomatic-proxy + LOC + churn-90d + fix-density,
//! each min-max normalized across the indexed set, summed. Cyclomatic proxy =
//! decision-node count (if/for/while/boolean_operator/except/match) from a
//! quick re-parse. Churn/fix counts come from `git log` on the repo (read-only).

use std::collections::HashMap;
use std::path::Path;

use rusqlite::Connection;
use tree_sitter::Node;

use crate::db;
use crate::error::{KgError, Result};
use crate::parse;

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

fn decision_nodes(root: Node<'_>) -> usize {
    fn rec(node: Node<'_>) -> usize {
        let mut n = usize::from(matches!(
            node.kind(),
            "if_statement" | "for_statement" | "while_statement" | "boolean_operator"
                | "except_clause" | "match_statement" | "conditional_expression"
        ));
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            n += rec(child);
        }
        n
    }
    rec(root)
}

fn git_counts(root: &Path) -> Result<(HashMap<String, f64>, HashMap<String, f64>)> {
    let run = |args: &[&str]| -> Result<String> {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .map_err(|e| KgError::Internal(format!("git log failed: {e}")))?;
        if !out.status.success() {
            return Err(KgError::Internal("git log failed".to_string()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let mut churn = HashMap::new();
    for line in run(&["log", "--since=90.days", "--name-only", "--pretty=format:"])?
        .lines()
        .filter(|l| l.ends_with(".py"))
    {
        *churn.entry(line.to_string()).or_insert(0.0) += 1.0;
    }
    let mut fixes = HashMap::new();
    for line in run(&[
        "log",
        "--since=180.days",
        "--grep=fix\\|bug\\|crash\\|wrong\\|incorrect\\|sai",
        "-i",
        "--name-only",
        "--pretty=format:",
    ])?
    .lines()
    .filter(|l| l.ends_with(".py"))
    {
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
    let mut stmt = conn.prepare("SELECT path FROM files")?;
    let paths: Vec<String> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let (churn, fixes) = git_counts(root)?;

    let mut scores: Vec<FileScore> = Vec::new();
    for rel in &paths {
        let abs = root.join(rel);
        let Ok(bytes) = std::fs::read(&abs) else { continue };
        let loc = bytes.split(|b: &u8| *b == b'\n').count() as f64;
        let cyc = match parse::parse_python(&bytes) {
            Ok(p) => decision_nodes(p.tree.root_node()) as f64,
            Err(_) => 0.0,
        };
        scores.push(FileScore {
            rel: rel.clone(),
            loc,
            cyclomatic: cyc,
            churn: churn.get(rel).copied().unwrap_or(0.0),
            fixes: fixes.get(rel).copied().unwrap_or(0.0),
            score: 0.0,
        });
    }
    if scores.is_empty() {
        return Err(KgError::NoResults("no indexed files".to_string()));
    }
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for s in &scores {
        for v in [s.loc, s.cyclomatic, s.churn, s.fixes] {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    for s in &mut scores {
        s.score = normalize(s.loc, lo, hi)
            + normalize(s.cyclomatic, lo, hi)
            + normalize(s.churn, lo, hi)
            + normalize(s.fixes, lo, hi);
    }
    scores.sort_by(|a, b| b.score.total_cmp(&a.score));
    Ok(scores
        .into_iter()
        .take(limit.clamp(1, 100))
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
