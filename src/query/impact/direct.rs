//! Depth-1 impact phases: grouped per-file rows and per-site rows.

use super::*;
use crate::db::{EDGE_SRC_FILE_JOIN, EDGE_SRC_JOIN};

/// Empty default (calls-only) detail: name the import/inherit files the
/// calls-only default skipped, so a file that only has importers points the
/// user at `--edges imports,inherits` instead of a bare zero.
fn importer_caveat_detail(conn: &Connection, rel: &str) -> Result<String> {
    let count_files = |kind: EdgeKind| -> Result<usize> {
        Ok(conn.query_row(
            &format!(
                "SELECT COUNT(DISTINCT f.path) FROM edges e
                 {IMPACT_DST_JOINS}
                 {EDGE_SRC_FILE_JOIN}
                 WHERE df.path = ?1 AND e.kind = ?2"
            ),
            rusqlite::params![rel, kind.as_str()],
            |r| r.get::<_, i64>(0),
        )? as usize)
    };
    let imports = count_files(EdgeKind::Imports)?;
    let inherits = count_files(EdgeKind::Inherits)?;
    Ok(format!(
        "0 impacting files found (calls only); {imports} file(s) import and {inherits} file(s) inherit from this file — rerun with --edges imports,inherits"
    ))
}

/// Depth-1 grouped total: distinct source files with any matching edge.
fn impact_grouped_total(conn: &Connection, ctx: &ImpactCtx<'_>) -> Result<usize> {
    Ok(conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT f.path) FROM edges e
             {IMPACT_DST_JOINS}
             {EDGE_SRC_FILE_JOIN}
             WHERE df.path = ?1 AND e.kind IN ({})",
            ctx.kind_sql
        ),
        [ctx.rel],
        |r| r.get::<_, i64>(0),
    )? as usize)
}

/// Depth-1 grouped rows: one pointer per impacting file with its site count.
fn impact_grouped_rows(conn: &Connection, ctx: &ImpactCtx<'_>) -> Result<Vec<Pointer>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT f.path, MIN(e.site_line) AS line,
                COUNT(DISTINCT {}) AS n
         FROM edges e
         {IMPACT_DST_JOINS}
         {EDGE_SRC_FILE_JOIN}
         WHERE df.path = ?1 AND e.kind IN ({})
         GROUP BY f.path ORDER BY f.path LIMIT ?2",
        impact_site_key(),
        ctx.kind_sql
    ))?;
    let rows = stmt.query_map(rusqlite::params![ctx.rel, ctx.limit as i64], |r| {
        Ok(Pointer {
            file: r.get::<_, String>(0)?,
            line: r.get::<_, i64>(1)?,
            tag: "",
            detail: format!("({} sites)", r.get::<_, i64>(2)?),
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Depth-1 grouped rows: one row per impacting file, with its site count.
pub(super) fn impact_direct_grouped(
    qctx: &QueryCtx<'_>,
    ctx: &ImpactCtx<'_>,
) -> Result<QueryOutput> {
    let conn = qctx.conn;
    let total_files = impact_grouped_total(conn, ctx)?;
    if total_files == 0 {
        if ctx.extended {
            return Err(impact_no_results(ctx));
        }
        // Default calls-only mode: a file with importers but no inbound calls
        // is not a bare zero — name the skipped relationships.
        return Err(AwError::NoResultsCaveated {
            caveat: IMPACT_CALLS_CAVEAT,
            detail: importer_caveat_detail(conn, ctx.rel)?,
        });
    }
    let rows = impact_grouped_rows(conn, ctx)?;
    let out = collect_output(qctx, rows.into_iter().map(Ok), ctx.limit, total_files)?;
    Ok(impact_with_caveat(out, ctx.caveat, ctx.extended))
}

/// Depth-1 per-site rows: one row per call site, with optional context/explain.
fn impact_site_total(conn: &Connection, ctx: &ImpactCtx<'_>) -> Result<usize> {
    Ok(conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT {}) FROM edges e
             {IMPACT_DST_JOINS}
             {EDGE_SRC_JOIN}
             WHERE df.path = ?1 AND e.kind IN ({})",
            impact_site_key(),
            ctx.kind_sql
        ),
        [ctx.rel],
        |r| r.get::<_, i64>(0),
    )? as usize)
}

fn impact_site_rows(conn: &Connection, ctx: &ImpactCtx<'_>) -> Result<Vec<Pointer>> {
    let sql = format!(
        "SELECT MIN(e.site_line), MIN(e.confidence), MIN(f.path),
                MIN(s.qualname), MIN(s.kind), MIN(e.kind)
         FROM edges e
         {IMPACT_DST_JOINS}
         {EDGE_SRC_FILE_JOIN}
         WHERE df.path = ?1 AND e.kind IN ({})
         GROUP BY {}
         ORDER BY 3, 1, 4 LIMIT ?2",
        ctx.kind_sql,
        impact_site_key()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![ctx.rel, ctx.limit as i64], |r| {
        let site: i64 = r.get(0)?;
        let conf: String = r.get(1)?;
        let path: String = r.get(2)?;
        Ok(Pointer {
            file: path,
            line: site,
            tag: confidence_tag(&conf),
            detail: String::new(),
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub(super) fn impact_direct_sites(qctx: &QueryCtx<'_>, ctx: &ImpactCtx<'_>) -> Result<QueryOutput> {
    let conn = qctx.conn;
    let total = impact_site_total(conn, ctx)?;
    if total == 0 {
        if ctx.extended {
            return Err(impact_no_results(ctx));
        }
        return Err(AwError::NoResults("0 impacting files found".to_string()));
    }
    let rows = impact_site_rows(conn, ctx)?;
    let out = collect_output(qctx, rows.into_iter().map(Ok), ctx.limit, total)?;
    Ok(impact_with_caveat(out, ctx.caveat, ctx.extended))
}
