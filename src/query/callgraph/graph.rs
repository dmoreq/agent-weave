//! Callers and callees: direct depth-1 lookups and recursive transitive
//! closures over the resolved call-edge set.

use super::frame::{
    callee_empty_reason, direct_callees, direct_callers, transitive_page, TransitiveQuery,
};
use super::*;
use crate::db::EDGE_SRC_FILE_JOIN;

/// Shared traversal knobs for `callers`/`callees` (mirrors `ImpactOpts`).
pub struct GraphQueryOpts {
    pub limit: usize,
    pub depth: usize,
}

/// Shared `callers`/`callees` prologue: clamp the knobs and resolve the
/// target name once. Returns the clamped limit/depth and the target symbol ids.
fn prepare_graph_query(
    conn: &Connection,
    name: &str,
    opts: GraphQueryOpts,
) -> Result<(usize, usize, Vec<i64>)> {
    let GraphQueryOpts { limit, depth } = opts;
    let limit = clamp_limit(limit)?;
    let depth = depth.clamp(1, 10);
    let (ids, _) = resolve_targets(conn, name, None)?;
    Ok((limit, depth, ids))
}

pub fn callers(ctx: &QueryCtx<'_>, name: &str, opts: GraphQueryOpts) -> Result<QueryOutput> {
    let QueryCtx { conn, .. } = *ctx;
    let (limit, depth, ids) = prepare_graph_query(conn, name, opts)?;
    if depth > 1 {
        return callers_transitive(ctx, &ids, limit, depth);
    }

    let (callers, total) = direct_callers(conn, &ids, limit)?;
    if total == 0 {
        return Err(AwError::NoResults(
            "0 callers found (dynamic dispatch possible — grep fallback recommended)".to_string(),
        ));
    }
    let rows = callers.into_iter().map(|row| {
        Ok(Pointer {
            file: row.path,
            line: row.site_line,
            tag: confidence_tag(&row.confidence),
            detail: row.qualname,
        })
    });
    collect_output(ctx, rows, limit, total)
}

/// Transitive callers (depth > 1), symmetric with `callees_transitive`.
fn callers_transitive(
    ctx: &QueryCtx<'_>,
    ids: &[i64],
    limit: usize,
    depth: usize,
) -> Result<QueryOutput> {
    // depth > 1: recursive closure over reverse call edges into the reached
    // set. `reached` keeps one row per caller at its shortest distance, and
    // the NOT EXISTS guard drops callers already reached at or before the
    // destination's distance — so each symbol displays once, at the depth it
    // is first reached, and the seed symbols never reappear as callers.
    let caveat = format!("aw: transitive callers — resolved static call edges only; depth {depth}");
    let (cte, select) = callers_transitive_sql(ids.len());
    transitive_page(
        ctx,
        ids,
        depth,
        limit,
        TransitiveQuery {
            cte,
            select,
            order_by: "lvl, path, line, qn",
            caveat: Some(caveat),
        },
        || {
            Ok(
                "0 callers found (dynamic dispatch possible — grep fallback recommended)"
                    .to_string(),
            )
        },
        |r| {
            let line: i64 = r.get(0)?;
            let conf: String = r.get(1)?;
            let path: String = r.get(2)?;
            let qualname: String = r.get(3)?;
            let level: i64 = r.get(4)?;
            Ok(Pointer {
                file: path,
                line,
                tag: confidence_tag(&conf),
                detail: format!("{qualname} [depth={level}]"),
            })
        },
    )
}

/// Recursive closure SQL for transitive callers, mirroring `transitive_sql`
/// in reverse: the first `ids_len` bind parameters are the seed ids, the next
/// two bind the depth cap (closure, then display filter).
fn callers_transitive_sql(ids_len: usize) -> (String, String) {
    let seed = crate::db::sql_placeholders(ids_len);
    let cte = format!(
        "WITH RECURSIVE reach(sym, d) AS (
             SELECT id, 0 FROM symbols WHERE id IN ({seed})
             UNION
             SELECT e.src_id, r.d + 1 FROM edges e JOIN reach r ON e.dst_id = r.sym
             WHERE e.kind = '{}' AND r.d < ?{d}
         ),
         reached(sym, d) AS (SELECT sym, MIN(d) FROM reach GROUP BY sym)",
        EdgeKind::Calls.as_str(),
        d = 1 + ids_len
    );
    let select = format!(
        "SELECT e.site_line AS line, e.confidence AS confidence,
                f.path AS path, s.qualname AS qn, r.d + 1 AS lvl
         FROM reached r
         JOIN edges e ON e.dst_id = r.sym
         {EDGE_SRC_FILE_JOIN}
         WHERE e.kind = '{}' AND r.d < ?{d2}
           AND NOT EXISTS (
               SELECT 1 FROM reach r2 WHERE r2.sym = e.src_id AND r2.d <= r.d
           )",
        EdgeKind::Calls.as_str(),
        d2 = 2 + ids_len
    );
    (cte, select)
}

pub fn callees(ctx: &QueryCtx<'_>, name: &str, opts: GraphQueryOpts) -> Result<QueryOutput> {
    let QueryCtx { conn, .. } = *ctx;
    let (limit, depth, ids) = prepare_graph_query(conn, name, opts)?;
    if depth == 1 {
        callees_direct(ctx, &ids, limit)
    } else {
        callees_transitive(ctx, &ids, limit, depth)
    }
}

fn callees_direct(ctx: &QueryCtx<'_>, ids: &[i64], limit: usize) -> Result<QueryOutput> {
    let QueryCtx { conn, .. } = *ctx;
    let (callees, total) = direct_callees(conn, ids, limit)?;
    if total == 0 {
        return Err(AwError::NoResults(callee_empty_reason(conn, ids)?));
    }
    let rows = callees.into_iter().map(|row| {
        Ok(Pointer {
            file: row.path,
            line: row.site_line,
            tag: confidence_tag(&row.confidence),
            detail: row.qualname,
        })
    });
    collect_output(ctx, rows, limit, total)
}

fn callees_transitive(
    ctx: &QueryCtx<'_>,
    ids: &[i64],
    limit: usize,
    depth: usize,
) -> Result<QueryOutput> {
    // depth > 1: recursive closure over call edges from the reached set.
    let QueryCtx { conn, .. } = *ctx;
    let (cte, select) = transitive_sql(ids.len());
    transitive_page(
        ctx,
        ids,
        depth,
        limit,
        TransitiveQuery {
            cte,
            select,
            order_by: "path, line",
            caveat: None,
        },
        || callee_empty_reason(conn, ids),
        |r| {
            let line: i64 = r.get(0)?;
            let conf: String = r.get(1)?;
            let path: String = r.get(2)?;
            let qualname: String = r.get(3)?;
            Ok(Pointer {
                file: path,
                line,
                tag: confidence_tag(&conf),
                detail: qualname,
            })
        },
    )
}

/// Recursive closure SQL for transitive callees. The first `ids_len` bind
/// parameters are the seed ids; the next two bind the depth cap.
fn transitive_sql(ids_len: usize) -> (String, String) {
    let seed = crate::db::sql_placeholders(ids_len);
    let cte = format!(
        "WITH RECURSIVE reach(sym, d) AS (
             SELECT id, 0 FROM symbols WHERE id IN ({seed})
             UNION
             SELECT e.dst_id, r.d + 1 FROM edges e JOIN reach r ON e.src_id = r.sym
             WHERE e.kind = '{}' AND e.dst_id IS NOT NULL AND r.d < ?{d}
         )",
        EdgeKind::Calls.as_str(),
        d = 1 + ids_len
    );
    let select = format!(
        "SELECT DISTINCT COALESCE(ds.line_start, e.site_line) AS line, e.confidence,
                COALESCE(df.path, f.path) AS path, COALESCE(ds.qualname, e.dst_name) AS qn
         FROM reach r
         -- CROSS JOIN + WHERE lets SQLite drive from the recursive set and
         -- use edges_src, instead of scanning the whole edge table (R7).
         CROSS JOIN edges e
         {EDGE_SRC_FILE_JOIN}
         LEFT JOIN symbols ds ON ds.id = e.dst_id
         LEFT JOIN files df ON df.id = ds.file_id
         WHERE e.src_id = r.sym AND e.kind = '{}' AND r.d < ?{d2}",
        EdgeKind::Calls.as_str(),
        d2 = 2 + ids_len
    );
    (cte, select)
}
