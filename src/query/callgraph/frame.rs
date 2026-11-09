//! Shared traversal frame for the call-graph queries: the direct and
//! transitive count-then-page loaders that both directions reuse.

use super::*;
use crate::db::EDGE_SRC_FILE_JOIN;
use crate::extract::EdgeSiteKind;

/// One direct call-edge row (caller or callee). Private loader output;
/// rendering and journaling stay at the public surface. `confidence` is
/// carried so `callees` and `context` keep the same ambiguous tagging.
pub(super) struct CallEdgeRow {
    pub(super) site_line: i64,
    pub(super) confidence: String,
    pub(super) path: String,
    pub(super) qualname: String,
}

/// Count-then-bounded-fetch frame shared by the direct caller/callee loads:
/// the count and the display set bind the same id list, then the display
/// limit joins as the trailing parameter (`?N`, N = id count + 1). The two
/// SQL strings carry everything that differs between the directions, so the
/// fetch/order mechanics exist exactly once.
fn count_then_page(
    conn: &Connection,
    ids: &[i64],
    limit: usize,
    count_sql: &str,
    page_sql: &str,
) -> Result<(Vec<CallEdgeRow>, usize)> {
    let mut params = id_params(ids);
    let total: usize =
        conn.query_row(count_sql, rusqlite::params_from_iter(params.iter()), |r| {
            r.get::<_, i64>(0)
        })? as usize;
    // The display limit is the trailing bind parameter after the id list.
    params.push(rusqlite::types::Value::from(limit as i64));
    let mut stmt = conn.prepare(page_sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok(CallEdgeRow {
                site_line: r.get(0)?,
                confidence: r.get(1)?,
                path: r.get(2)?,
                qualname: r.get(3)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok((rows, total))
}

/// Per-direction knobs for `transitive_page`: the recursive-closure SQL pair,
/// the display ORDER BY, and the trailing caveat. One struct keeps the two
/// call sites from re-spelling the argument list (the `MemFilter` pattern).
pub(super) struct TransitiveQuery {
    pub(super) cte: String,
    pub(super) select: String,
    pub(super) order_by: &'static str,
    pub(super) caveat: Option<String>,
}

/// Count-then-bounded-fetch frame shared by the transitive caller/callee
/// traversals (R6 round-2), mirroring `count_then_page` for the recursive
/// closures: the count and the display set bind the seed ids plus the two
/// depth-cap parameters, then the display limit joins as the trailing
/// parameter (`?N`, N = id count + 3). The [`TransitiveQuery`] knobs, row
/// mapper, and empty-result detail carry everything that differs between the
/// directions, so the bind mechanics exist exactly once.
pub(super) fn transitive_page<E, M>(
    ctx: &QueryCtx<'_>,
    ids: &[i64],
    depth: usize,
    limit: usize,
    query: TransitiveQuery,
    empty_detail: E,
    map: M,
) -> Result<QueryOutput>
where
    E: FnOnce() -> Result<String>,
    M: Fn(&rusqlite::Row<'_>) -> rusqlite::Result<Pointer>,
{
    let QueryCtx { conn, .. } = *ctx;
    let TransitiveQuery {
        cte,
        select,
        order_by,
        caveat,
    } = query;
    let mut params = id_params(ids);
    params.push(rusqlite::types::Value::from(depth as i64));
    params.push(rusqlite::types::Value::from(depth as i64));
    let count_sql = format!("{cte} SELECT COUNT(*) FROM ({select})");
    let total: usize =
        conn.query_row(&count_sql, rusqlite::params_from_iter(params.iter()), |r| {
            r.get::<_, i64>(0)
        })? as usize;
    if total == 0 {
        return Err(EgError::NoResults(empty_detail()?));
    }
    // Bounded materialization: the closure result is counted above and
    // fetched with LIMIT, never fully collected.
    let sql = format!(
        "{cte} {select} ORDER BY {order_by} LIMIT ?{limit_param}",
        limit_param = ids.len() + 3
    );
    let mut stmt = conn.prepare(&sql)?;
    // The display limit binds after the two depth-cap parameters.
    params.push(rusqlite::types::Value::from(limit as i64));
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), map)?;
    let mut out = collect_output(ctx, rows, limit, total)?;
    if let Some(caveat) = caveat {
        prepend_header(&mut out, &caveat);
    }
    Ok(out)
}

/// Side-effect-free direct-caller load. The count and the bounded display set
/// use the same predicate; callers/context render them differently.
pub(super) fn direct_callers(
    conn: &Connection,
    ids: &[i64],
    limit: usize,
) -> Result<(Vec<CallEdgeRow>, usize)> {
    let placeholders = crate::db::sql_placeholders(ids.len());
    let count_sql = format!(
        "SELECT COUNT(*) FROM edges e WHERE e.dst_id IN ({placeholders}) AND e.kind = '{}'",
        EdgeKind::Calls.as_str()
    );
    let page_sql = format!(
        "SELECT e.site_line, e.confidence, f.path, s.qualname
         FROM edges e
         {EDGE_SRC_FILE_JOIN}
         LEFT JOIN symbols ds ON ds.id = e.dst_id
         WHERE e.dst_id IN ({placeholders}) AND e.kind = '{}'
         ORDER BY f.path, e.site_line LIMIT ?{limit_param}",
        EdgeKind::Calls.as_str(),
        limit_param = ids.len() + 1
    );
    count_then_page(conn, ids, limit, &count_sql, &page_sql)
}

/// Side-effect-free direct-callee load. Mirrors the depth-1 SQL used by
/// `callees`; `context` reuses it with a single id.
pub(super) fn direct_callees(
    conn: &Connection,
    ids: &[i64],
    limit: usize,
) -> Result<(Vec<CallEdgeRow>, usize)> {
    let placeholders = crate::db::sql_placeholders(ids.len());
    let count_sql = format!(
        "SELECT COUNT(DISTINCT COALESCE(ds.id, -(1000000 + e.id)))
         FROM edges e LEFT JOIN symbols ds ON ds.id = e.dst_id
         WHERE e.src_id IN ({placeholders}) AND e.kind = '{}'",
        EdgeKind::Calls.as_str()
    );
    let page_sql = format!(
        "SELECT MIN(COALESCE(ds.line_start, e.site_line)) AS line,
                MAX(e.confidence) AS confidence,
                MAX(COALESCE(df.path, f.path)) AS path,
                COALESCE(ds.qualname, e.dst_name) AS qn
         FROM edges e
         {EDGE_SRC_FILE_JOIN}
         LEFT JOIN symbols ds ON ds.id = e.dst_id
         LEFT JOIN files df ON df.id = ds.file_id
         WHERE e.src_id IN ({placeholders}) AND e.kind = '{}'
         GROUP BY COALESCE(ds.id, -(1000000 + e.id))
         ORDER BY path, line LIMIT ?{limit_param}",
        EdgeKind::Calls.as_str(),
        limit_param = ids.len() + 1
    );
    count_then_page(conn, ids, limit, &count_sql, &page_sql)
}

/// Explain an empty `callees` result instead of reporting a resolver gap as
/// absence. Called only when the callee count is 0. Unresolved call sites are
/// attributed to the queried symbol by its file and line range; names are
/// capped so the detail stays one line.
pub(super) fn callee_empty_reason(conn: &Connection, ids: &[i64]) -> Result<String> {
    const SHOWN: usize = 5;
    let names = unresolved_names_in(conn, ids)?;
    if names.is_empty() {
        return Ok("0 callees found".to_string());
    }
    let shown = names
        .iter()
        .take(SHOWN)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    let more = if names.len() > SHOWN {
        format!(" (+{} more)", names.len() - SHOWN)
    } else {
        String::new()
    };
    let hint = match symbol_file(conn, ids) {
        Some(path) => format!(" — run eg unresolved --names --path {path}"),
        None => " — run eg unresolved --names".to_string(),
    };
    Ok(format!(
        "0 callees found (resolved static call edges only); {} unresolved call site(s) inside the queried symbol: {shown}{more}{hint}",
        names.len()
    ))
}

/// Unresolved call names attributed to the queried symbols by file and line range.
fn unresolved_names_in(
    conn: &Connection,
    ids: &[i64],
) -> Result<std::collections::BTreeSet<String>> {
    let mut names = std::collections::BTreeSet::new();
    // One prepare for the whole id set; each id re-executes the statement.
    let mut stmt = conn.prepare(&format!(
        "SELECT DISTINCT u.dst_name
           FROM unresolved_calls u
           JOIN symbols s ON s.file_id = u.file_id
          WHERE s.id = ?1 AND u.kind = '{}'
            AND u.site_line >= s.line_start
            AND u.site_line <= s.line_end
          ORDER BY u.dst_name",
        EdgeSiteKind::Call.as_str()
    ))?;
    for id in ids {
        let rows = stmt.query_map([*id], |r| r.get::<_, String>(0))?;
        for row in rows {
            names.insert(row?);
        }
    }
    Ok(names)
}

fn symbol_file(conn: &Connection, ids: &[i64]) -> Option<String> {
    ids.iter().find_map(|id| {
        conn.query_row(
            "SELECT f.path FROM symbols s JOIN files f ON f.id = s.file_id WHERE s.id = ?1",
            [*id],
            |r| r.get::<_, String>(0),
        )
        .ok()
    })
}
