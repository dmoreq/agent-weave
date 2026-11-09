//! P1.5 insight command: `stats`.

use rusqlite::Connection;

use crate::db;
use crate::error::{EgError, Result};
use crate::extract::EdgeKind;
use crate::query::QueryCtx;

/// Counter set shared by the global and scoped `stats` renderings.
struct StatsTotals {
    files: i64,
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
    let eg_version =
        db::meta_get(conn, db::meta_keys::ENGRAM_VERSION).unwrap_or_else(|| "?".into());
    let grammar = db::meta_get(conn, db::meta_keys::GRAMMAR_VERSION).unwrap_or_else(|| "?".into());
    Ok(stats_line(
        scope.as_deref(),
        &totals,
        size,
        &eg_version,
        &grammar,
    ))
}

fn global_totals(conn: &Connection) -> Result<StatsTotals> {
    // Build-time counters first (R7 L4): every filtered COUNT scanned rows;
    // the manifest already stores the authoritative totals (written from SQL
    // at build end), so stats is O(1). The (key, fallback SQL) pairs and the
    // meta-or-COUNT fallback live in db (META_COUNTERS / meta_count), shared
    // with the no-op build line.
    Ok(StatsTotals {
        files: db::meta_count(conn, db::meta_keys::FILE_COUNT)?,
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

/// Scoped totals: one range scan per table. Edges attribute to the source
/// symbol's file, matching `report`; the three edge kinds come out of the
/// same conditional-SUM scan as the build-time `meta_counts` (one query, not
/// one per kind).
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
        return Err(EgError::NoResults(format!("no indexed files in '{scope}'")));
    }
    let edge_from = format!("edges e {}", db::EDGE_SRC_FILE_JOIN);
    let (edges, calls, imports, inherits) = conn.query_row(
        &format!(
            "SELECT COUNT(*),
            COALESCE(SUM(e.kind = '{}'), 0),
            COALESCE(SUM(e.kind = '{}'), 0),
            COALESCE(SUM(e.kind = '{}'), 0)
     FROM {edge_from} WHERE 1=1 {clause}",
            EdgeKind::Calls.as_str(),
            EdgeKind::Imports.as_str(),
            EdgeKind::Inherits.as_str(),
        ),
        rusqlite::params_from_iter(params.iter()),
        crate::db::ints4,
    )?;
    Ok(StatsTotals {
        files,
        symbols: scalar("COUNT(*)", "symbols s JOIN files f ON f.id = s.file_id", "")?,
        edges,
        calls,
        imports,
        inherits,
        perr: scalar("COALESCE(SUM(f.parse_status != 'ok'), 0)", "files f", "")?,
        unresolved: scalar("COALESCE(SUM(f.unresolved), 0)", "files f", "")?,
    })
}

fn stats_line(
    scope: Option<&str>,
    totals: &StatsTotals,
    size: u64,
    eg_version: &str,
    grammar: &str,
) -> String {
    let scope_prefix = scope.map(|s| format!("scope={s} ")).unwrap_or_default();
    let StatsTotals {
        files,
        symbols,
        edges,
        calls,
        imports,
        inherits,
        perr,
        unresolved,
    } = totals;
    format!(
        "eg stats: {scope_prefix}files={files} symbols={symbols} edges={edges} (calls={calls} imports={imports} inherits={inherits}) parse_errors={perr} unresolved={unresolved} db_bytes={size} eg={eg_version} grammar={grammar}"
    )
}
