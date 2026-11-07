//! P1.5 insight command: `stats`.

use rusqlite::Connection;

use crate::db;
use crate::error::{AwError, Result};
use crate::extract::EdgeKind;
use crate::query::QueryCtx;

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

pub fn stats(ctx: &QueryCtx<'_>, path: Option<&str>) -> Result<String> {
    let QueryCtx { conn, root } = *ctx;
    // `--path` scopes every count to one repo-relative prefix; absent keeps
    // the global build-time counters (O(1)).
    let scope = match path {
        Some(p) => {
            crate::query::validate_path_scope(p, false)?;
            Some(crate::query::normalize_scope(p.trim()).to_string())
        }
        None => None,
    };
    let totals = match &scope {
        None => global_totals(conn)?,
        Some(scope) => scoped_totals(conn, scope)?,
    };
    let size = db::file_size(&db::db_path(root));
    let aw_version = db::meta_get(conn, db::meta_keys::AW_VERSION).unwrap_or_else(|| "?".into());
    let grammar = db::meta_get(conn, db::meta_keys::GRAMMAR_VERSION).unwrap_or_else(|| "?".into());
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
    // at build end), so stats is O(1). The (key, fallback SQL) pairs and the
    // meta-or-COUNT fallback live in db (META_COUNTERS / meta_count), shared
    // with the no-op build line.
    Ok(StatsTotals {
        files: db::meta_count(conn, db::meta_keys::FILE_COUNT)?,
        generated: q("SELECT COUNT(*) FROM files WHERE generated = 1")?,
        symbols: db::meta_count(conn, db::meta_keys::SYMBOL_COUNT)?,
        edges: db::meta_count(conn, db::meta_keys::EDGE_COUNT)?,
        calls: db::meta_count(conn, db::meta_keys::EDGE_CALLS)?,
        imports: db::meta_count(conn, db::meta_keys::EDGE_IMPORTS)?,
        inherits: db::meta_count(conn, db::meta_keys::EDGE_INHERITS)?,
        perr: db::meta_count(conn, db::meta_keys::PARSE_ERROR_COUNT)?,
        // Meta only: this read path has no COUNT fallback (pre-counter
        // indexes report 0 here, as before).
        unresolved: db::meta_get(conn, db::meta_keys::UNRESOLVED_CALLEE_COUNT)
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0),
    })
}

/// Scoped totals: one range scan per counter. Edges attribute to the source
/// symbol's file, matching `report`.
fn scoped_totals(conn: &Connection, scope: &str) -> Result<StatsTotals> {
    let (clause, params) = crate::query::scoped_params("f", "path", Some(scope), 1);
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
        calls: scalar(
            "COUNT(*)",
            edge_from,
            &format!("AND e.kind = '{}'", EdgeKind::Calls.as_str()),
        )?,
        imports: scalar(
            "COUNT(*)",
            edge_from,
            &format!("AND e.kind = '{}'", EdgeKind::Imports.as_str()),
        )?,
        inherits: scalar(
            "COUNT(*)",
            edge_from,
            &format!("AND e.kind = '{}'", EdgeKind::Inherits.as_str()),
        )?,
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
