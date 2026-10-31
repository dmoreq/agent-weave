//! Query surface: callers/callees/impact with qualname disambiguation,
//! result-set-scoped staleness, caps, tier tags, and empty-result reason lines
//! (response contract; CUJ2/CUJ3/CUJ4/CUJ7).

use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::Connection;

use crate::db;
use crate::error::{KgError, Result};
use crate::hash::fnv1a64;

pub const DEFAULT_LIMIT: usize = 50;
pub const MAX_LIMIT: usize = 10_000;

/// One resolved candidate: (path, line, qualname).
pub type Candidate = (String, i64, String);

pub struct Pointer {
    pub file: String,
    pub line: i64,
    pub tag: &'static str, // "" or " [ambiguous]"
}

impl Pointer {
    pub fn render(&self) -> String {
        format!("{}:{}{}", self.file, self.line, self.tag)
    }
}

pub struct QueryOutput {
    pub header: Option<String>,
    pub pointers: Vec<Pointer>,
    pub shown: usize,
    pub total: usize,
}

pub fn clamp_limit(limit: usize) -> Result<usize> {
    if limit == 0 {
        return Err(KgError::Usage("--limit must be >= 1".to_string()));
    }
    Ok(limit.min(MAX_LIMIT))
}

/// Resolve the query name to target symbol ids. Multiple distinct symbols ->
/// exit 2 with a `path:line qualname` candidate list on stdout (CUJ4).
fn target_ids(conn: &Connection, name: &str) -> Result<(Vec<i64>, Vec<Candidate>)> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.qualname, s.name, f.path, s.line_start
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE s.name = ?1 OR s.qualname = ?1
         ORDER BY s.qualname, f.path",
    )?;
    let rows = stmt.query_map(rusqlite::params![name], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?))
    })?;
    let mut ids = Vec::new();
    let mut cands = Vec::new();
    for row in rows {
        let (id, qualname, sname, path, line) = row?;
        // Qualname match is authoritative; a bare-name match that IS the same
        // symbol is deduped by id.
        if !ids.contains(&id) {
            ids.push(id);
        }
        let _ = sname;
        cands.push((path, line, qualname));
    }
    Ok((ids, cands))
}

pub fn callers(conn: &Connection, root: &Path, name: &str, limit: usize) -> Result<QueryOutput> {
    let limit = clamp_limit(limit)?;
    let (ids, cands) = target_ids(conn, name)?;
    if ids.is_empty() {
        return Err(KgError::NoResults(format!("symbol '{name}' not in index")));
    }
    if cands.len() > 1 {
        // CUJ4: no merged-silent result; name the Class.method form.
        for (path, line, qualname) in &cands {
            println!("{path}:{line} {qualname}");
        }
        return Err(KgError::Usage(format!(
            "ambiguous name '{name}' — {} candidates; use the Class.method qualname form",
            cands.len()
        )));
    }
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT e.site_line, e.confidence, f.path
         FROM edges e
         JOIN symbols s ON s.id = e.src_id
         JOIN files f ON f.id = s.file_id
         WHERE e.dst_id IN ({placeholders}) AND e.kind = 'calls'
         ORDER BY f.path, e.site_line"
    );
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<rusqlite::types::Value> = ids.iter().map(|i| rusqlite::types::Value::from(*i)).collect();
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        let site: i64 = r.get(0)?;
        let conf: String = r.get(1)?;
        let path: String = r.get(2)?;
        Ok(Pointer {
            file: path,
            line: site,
            tag: if conf == "AMBIGUOUS" { " [ambiguous]" } else { "" },
        })
    })?;
    collect_output(
        conn,
        root,
        rows,
        limit,
        "0 callers found (dynamic dispatch possible — grep fallback recommended)",
    )
}

pub fn callees(
    conn: &Connection,
    root: &Path,
    name: &str,
    limit: usize,
    depth: usize,
) -> Result<QueryOutput> {
    let limit = clamp_limit(limit)?;
    let depth = depth.clamp(1, 10);
    let (ids, cands) = target_ids(conn, name)?;
    if ids.is_empty() {
        return Err(KgError::NoResults(format!("symbol '{name}' not in index")));
    }
    if cands.len() > 1 {
        for (path, line, qualname) in &cands {
            println!("{path}:{line} {qualname}");
        }
        return Err(KgError::Usage(format!(
            "ambiguous name '{name}' — {} candidates; use the Class.method qualname form",
            cands.len()
        )));
    }
    // Callee pointer: the callee's definition line. AMBIGUOUS edges have no
    // dst symbol; they surface as the call site itself with the ambiguous tag.
    // depth > 1: recursive closure over call edges from the reached set.
    let (sql, params): (String, Vec<rusqlite::types::Value>) = if depth == 1 {
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        (
            format!(
                "SELECT COALESCE(ds.line_start, e.site_line) AS line, e.confidence,
                COALESCE(df.path, sf.path) AS path
         FROM edges e
         JOIN symbols s ON s.id = e.src_id JOIN files sf ON sf.id = s.file_id
         LEFT JOIN symbols ds ON ds.id = e.dst_id
         LEFT JOIN files df ON df.id = ds.file_id
         WHERE e.src_id IN ({placeholders}) AND e.kind = 'calls'
         ORDER BY path, line"
            ),
            ids.iter().map(|i| rusqlite::types::Value::from(*i)).collect(),
        )
    } else {
        let seed = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        (
            format!(
                "WITH RECURSIVE reach(sym, d) AS (
             SELECT id, 0 FROM symbols WHERE id IN ({seed})
             UNION
             SELECT e.dst_id, r.d + 1 FROM edges e JOIN reach r ON e.src_id = r.sym
             WHERE e.kind = 'calls' AND e.dst_id IS NOT NULL AND r.d < ?{d}
         )
         SELECT DISTINCT COALESCE(ds.line_start, e.site_line) AS line, e.confidence,
                COALESCE(df.path, sf.path) AS path
         FROM reach r
         JOIN edges e ON e.src_id = r.sym AND e.kind = 'calls'
         JOIN symbols ss ON ss.id = e.src_id JOIN files sf ON sf.id = ss.file_id
         LEFT JOIN symbols ds ON ds.id = e.dst_id
         LEFT JOIN files df ON df.id = ds.file_id
         ORDER BY path, line",
                d = 1 + ids.len()
            ),
            ids.iter()
                .map(|i| rusqlite::types::Value::from(*i))
                .chain(std::iter::once(rusqlite::types::Value::from(
                    depth as i64,
                )))
                .collect(),
        )
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        let line: i64 = r.get(0)?;
        let conf: String = r.get(1)?;
        let path: String = r.get(2)?;
        Ok(Pointer {
            file: path,
            line,
            tag: if conf == "AMBIGUOUS" { " [ambiguous]" } else { "" },
        })
    })?;
    collect_output(conn, root, rows, limit, "0 callees found")
}

pub fn impact(conn: &Connection, root: &Path, file: &str, limit: usize) -> Result<QueryOutput> {
    let limit = clamp_limit(limit)?;
    let rel = file.trim_start_matches("./").to_string();
    let exists: bool = conn
        .query_row("SELECT COUNT(*) FROM files WHERE path = ?1", [&rel], |r| r.get(0))
        .map(|c: i64| c > 0)?;
    if !exists {
        return Err(KgError::NoResults(format!("path '{file}' not indexed")));
    }
    // Direction semantics (CLI help): files that call INTO <file>.
    let sql = "SELECT e.site_line, e.confidence, sf.path
         FROM edges e
         JOIN symbols ds ON ds.id = e.dst_id JOIN files df ON df.id = ds.file_id
         JOIN symbols ss ON ss.id = e.src_id JOIN files sf ON sf.id = ss.file_id
         WHERE df.path = ?1 AND e.kind = 'calls'
         ORDER BY sf.path, e.site_line";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([&rel], |r| {
        let site: i64 = r.get(0)?;
        let conf: String = r.get(1)?;
        let path: String = r.get(2)?;
        Ok(Pointer {
            file: path,
            line: site,
            tag: if conf == "AMBIGUOUS" { " [ambiguous]" } else { "" },
        })
    })?;
    collect_output(conn, root, rows, limit, "0 impacting files found")
}

/// Cap, then staleness-check the post-cap displayed set only (cost bounded by
/// the displayed count): stat short-circuits, FNV-1a re-hash on mismatch.
fn collect_output(
    conn: &Connection,
    root: &Path,
    rows: impl Iterator<Item = std::result::Result<Pointer, rusqlite::Error>>,
    limit: usize,
    empty_reason: &str,
) -> Result<QueryOutput> {
    let mut all: Vec<Pointer> = Vec::new();
    for r in rows {
        all.push(r?);
    }
    let total = all.len();
    if total == 0 {
        return Err(KgError::NoResults(empty_reason.to_string()));
    }
    let shown = all.len().min(limit);
    let displayed: Vec<Pointer> = all.into_iter().take(shown).collect();
    let files: BTreeSet<&String> = displayed.iter().map(|p| &p.file).collect();
    let n_result_files = files.len();
    let mut changed = 0usize;
    for f in &files {
        if file_changed(conn, root, f) {
            changed += 1;
        }
    }
    let header = if changed > 0 {
        Some(format!(
            "kg: {changed} of {n_result_files} result files changed since build — run kg build"
        ))
    } else {
        None
    };
    Ok(QueryOutput { header, pointers: displayed, shown, total })
}

pub fn print_output(out: &QueryOutput) {
    if let Some(h) = &out.header {
        println!("{h}");
    }
    for p in &out.pointers {
        println!("{}", p.render());
    }
    if out.total > out.shown {
        println!("showing {} of {}", out.shown, out.total);
        eprintln!("[kg] capped at {} results ({} total)", out.shown, out.total);
    }
}

/// Staleness: (mtime, size) stat short-circuit; FNV-1a re-hash on mismatch.
pub fn file_changed(conn: &Connection, root: &Path, rel: &str) -> bool {
    let row = conn.query_row(
        "SELECT mtime, size, content_hash FROM files WHERE path = ?1",
        [rel],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)),
    );
    let Ok((stored_mtime, stored_size, stored_hash)) = row else { return false };
    let abs = root.join(rel);
    let Some(size) = db::stat_size(&abs) else { return true };
    if size != stored_size {
        return true;
    }
    let Some(mtime) = db::stat_mtime(&abs) else { return true };
    if mtime != stored_mtime {
        let Ok(bytes) = std::fs::read(&abs) else { return true };
        let hash = fnv1a64(&bytes) as i64;
        return hash != stored_hash;
    }
    false
}
