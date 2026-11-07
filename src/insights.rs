//! P1.5 insight command: `stats`.

use std::path::Path;

use rusqlite::Connection;

use crate::db;
use crate::error::{AwError, Result};

/// Counter set shared by the global and scoped `stats` renderings.
struct StatsTotals {
    files: i64,
    generated: i64,
    symbols: i64,
    edges: i64,
    calls: i64,
    imports: i64,
    inherits: i64,
    perr: i64,
    unresolved: i64,
}

pub fn stats(conn: &Connection, root: &Path, path: Option<&str>) -> Result<String> {
    // `--path` scopes every count to one repo-relative prefix; absent keeps
    // the global build-time counters (O(1)).
    let scope = match path {
        Some(p) => {
            crate::query::validate_path_scope(p, false)?;
            Some(
                p.trim()
                    .trim_start_matches("./")
                    .trim_end_matches('/')
                    .to_string(),
            )
        }
        None => None,
    };
    let totals = match &scope {
        None => global_totals(conn)?,
        Some(scope) => scoped_totals(conn, scope)?,
    };
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
    Ok(stats_line(
        scope.as_deref(),
        &totals,
        size,
        &aw_version,
        &grammar,
    ))
}

fn global_totals(conn: &Connection) -> Result<StatsTotals> {
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
    Ok(StatsTotals {
        files: count_or_meta("file_count", "SELECT COUNT(*) FROM files")?,
        generated: q("SELECT COUNT(*) FROM files WHERE generated = 1")?,
        symbols: count_or_meta("symbol_count", "SELECT COUNT(*) FROM symbols")?,
        edges: count_or_meta("edge_count", "SELECT COUNT(*) FROM edges")?,
        calls: count_or_meta(
            "edge_calls",
            "SELECT COUNT(*) FROM edges WHERE kind = 'calls'",
        )?,
        imports: count_or_meta(
            "edge_imports",
            "SELECT COUNT(*) FROM edges WHERE kind = 'imports'",
        )?,
        inherits: count_or_meta(
            "edge_inherits",
            "SELECT COUNT(*) FROM edges WHERE kind = 'inherits'",
        )?,
        perr: count_or_meta(
            "parse_error_count",
            "SELECT COALESCE(SUM(parse_status != 'ok'), 0) FROM files",
        )?,
        unresolved: meta_int("unresolved_callee_count").unwrap_or(0),
    })
}

/// Scoped totals: one range scan per counter. Edges attribute to the source
/// symbol's file, matching `report`.
fn scoped_totals(conn: &Connection, scope: &str) -> Result<StatsTotals> {
    let (clause, value) = crate::query::path_scope_sql_pub("f", Some(scope), 1);
    let params: Vec<rusqlite::types::Value> = value
        .map(rusqlite::types::Value::from)
        .into_iter()
        .collect();
    let scalar = |expr: &str, from: &str, extra: &str| -> Result<i64> {
        Ok(conn.query_row(
            &format!("SELECT {expr} FROM {from} WHERE 1=1 {extra} {clause}"),
            rusqlite::params_from_iter(params.iter()),
            |r| r.get(0),
        )?)
    };
    let files = scalar("COUNT(*)", "files f", "")?;
    if files == 0 {
        return Err(AwError::NoResults(format!("no indexed files in '{scope}'")));
    }
    let edge_from = "edges e JOIN symbols s ON s.id = e.src_id JOIN files f ON f.id = s.file_id";
    Ok(StatsTotals {
        files,
        generated: scalar("COUNT(*)", "files f", "AND f.generated = 1")?,
        symbols: scalar("COUNT(*)", "symbols s JOIN files f ON f.id = s.file_id", "")?,
        edges: scalar("COUNT(*)", edge_from, "")?,
        calls: scalar("COUNT(*)", edge_from, "AND e.kind = 'calls'")?,
        imports: scalar("COUNT(*)", edge_from, "AND e.kind = 'imports'")?,
        inherits: scalar("COUNT(*)", edge_from, "AND e.kind = 'inherits'")?,
        perr: scalar("COALESCE(SUM(f.parse_status != 'ok'), 0)", "files f", "")?,
        unresolved: scalar("COALESCE(SUM(f.unresolved), 0)", "files f", "")?,
    })
}

fn stats_line(
    scope: Option<&str>,
    totals: &StatsTotals,
    size: u64,
    aw_version: &str,
    grammar: &str,
) -> String {
    let scope_prefix = scope.map(|s| format!("scope={s} ")).unwrap_or_default();
    let StatsTotals {
        files,
        generated,
        symbols,
        edges,
        calls,
        imports,
        inherits,
        perr,
        unresolved,
    } = totals;
    format!(
        "aw stats: {scope_prefix}files={files} generated={generated} symbols={symbols} edges={edges} (calls={calls} imports={imports} inherits={inherits}) parse_errors={perr} unresolved={unresolved} db_bytes={size} aw={aw_version} grammar={grammar}"
    )
}
