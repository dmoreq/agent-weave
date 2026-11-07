//! Call-graph queries: imports, callers, callees, path, context.

use super::*;
use crate::db::EDGE_SRC_FILE_JOIN;
use crate::extract::EdgeSiteKind;

pub fn imports(ctx: &QueryCtx<'_>, file: &str, limit: usize) -> Result<QueryOutput> {
    let QueryCtx { conn, .. } = *ctx;
    let limit = clamp_limit(limit)?;
    let rel = indexed_file(conn, file)?;
    let total: usize = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM edges e
         {EDGE_SRC_FILE_JOIN}
         WHERE f.path = ?1 AND e.kind = '{}'",
            EdgeKind::Imports.as_str()
        ),
        [&rel],
        |r| r.get::<_, i64>(0),
    )? as usize;
    if total == 0 {
        return Err(AwError::NoResultsCaveated {
            caveat: IMPORTS_CAVEAT,
            detail: "0 imports found (external and unresolved imports are omitted)".to_string(),
        });
    }
    let sql = format!(
        "SELECT e.site_line, e.dst_name FROM edges e
         {EDGE_SRC_FILE_JOIN}
         WHERE f.path = ?1 AND e.kind = '{}'
         ORDER BY e.site_line, e.dst_name LIMIT ?2",
        EdgeKind::Imports.as_str()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![rel, limit as i64], |r| {
        Ok(Pointer {
            file: rel.clone(),
            line: r.get(0)?,
            tag: "",
            detail: r.get(1)?,
        })
    })?;
    let mut out = collect_output(ctx, rows, limit, total)?;
    prepend_header(&mut out, IMPORTS_CAVEAT);
    Ok(out)
}

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

/// One direct call-edge row (caller or callee). Private loader output;
/// rendering and journaling stay at the public surface. `confidence` is
/// carried so `callees` and `context` keep the same ambiguous tagging.
struct CallEdgeRow {
    site_line: i64,
    confidence: String,
    path: String,
    qualname: String,
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
struct TransitiveQuery {
    cte: String,
    select: String,
    order_by: &'static str,
    caveat: Option<String>,
}

/// Count-then-bounded-fetch frame shared by the transitive caller/callee
/// traversals (R6 round-2), mirroring `count_then_page` for the recursive
/// closures: the count and the display set bind the seed ids plus the two
/// depth-cap parameters, then the display limit joins as the trailing
/// parameter (`?N`, N = id count + 3). The [`TransitiveQuery`] knobs, row
/// mapper, and empty-result detail carry everything that differs between the
/// directions, so the bind mechanics exist exactly once.
fn transitive_page<E, M>(
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
        return Err(AwError::NoResults(empty_detail()?));
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
fn direct_callers(
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
fn direct_callees(
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
fn callee_empty_reason(conn: &Connection, ids: &[i64]) -> Result<String> {
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
        Some(path) => format!(" — run aw unresolved --names --path {path}"),
        None => " — run aw unresolved --names".to_string(),
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

/// Shortest call path (Amendment 13): does `from` reach `to` through resolved
/// call edges, and via which call sites? Forward direction only (reverse
/// question = swap arguments, OQ5). Pinned determinism: multi-edges dedup to
/// the smallest `site_line`, neighbors expand in
/// `ORDER BY ds.qualname, df.path, e.site_line`, FIFO frontier with
/// visited-on-enqueue, parent = first discoverer. No limit surface: a single
/// shortest chain has at most `depth` hops (recorded deviation). Output:
/// caveat header, hop call-site pointers in path order, final chain line of
/// resolved full qualnames.
pub fn path(ctx: &QueryCtx<'_>, from: &str, to: &str, depth: usize) -> Result<LineQuery> {
    let QueryCtx { conn, .. } = *ctx;
    let depth = depth.clamp(1, 10);
    let (from_id, from_qn) = resolve_endpoint(conn, from, "from")?;
    let (to_id, to_qn) = resolve_endpoint(conn, to, "to")?;
    if from_id == to_id {
        return Err(AwError::NoResults(format!(
            "'{from_qn}' and '{to_qn}' resolve to the same symbol"
        )));
    }
    let (chain_ids, hop_sites) = match path_bfs(conn, from_id, to_id, depth)? {
        PathSearch::Exhausted => {
            return Err(AwError::NoResults(format!(
                "no call path from '{from_qn}' to '{to_qn}' at any depth <= {depth} (resolved edges only)"
            )));
        }
        PathSearch::CapTruncated => {
            return Err(AwError::NoResults(format!(
                "no call path from '{from_qn}' to '{to_qn}' within depth {depth}; a longer path may exist (resolved edges only)"
            )));
        }
        PathSearch::Found {
            chain_ids,
            hop_sites,
        } => (chain_ids, hop_sites),
    };
    render_path(ctx, &chain_ids, &hop_sites)
}

/// Resolve a query name to target ids, failing on a missing or ambiguous
/// name. `role` labels the candidate list for the two `path` endpoints.
fn resolve_targets(
    conn: &Connection,
    name: &str,
    role: Option<&'static str>,
) -> Result<(Vec<i64>, Vec<NameCandidate>)> {
    let (ids, cands) = target_ids(conn, name)?;
    if ids.is_empty() {
        return Err(AwError::NoResults(format!("symbol '{name}' not in index")));
    }
    if cands.len() > 1 {
        // CUJ4: no merged-silent result; name the Class.method form. The
        // candidate list travels IN the error so non-CLI surfaces render it.
        return Err(AwError::Ambiguous {
            name: name.to_string(),
            total: ids.len(),
            candidates: cands,
            role,
        });
    }
    Ok((ids, cands))
}

/// Resolve one `path` endpoint (bare or dotted name) to its single symbol id
/// and display qualname.
fn resolve_endpoint(conn: &Connection, name: &str, role: &'static str) -> Result<(i64, String)> {
    let (ids, cands) = resolve_targets(conn, name, Some(role))?;
    Ok((ids[0], cands[0].2.clone()))
}

/// Render the found chain: staleness header, hop call-site pointers, and the
/// resolved qualname chain.
fn render_path(ctx: &QueryCtx<'_>, chain_ids: &[i64], hop_sites: &[i64]) -> Result<LineQuery> {
    let QueryCtx { conn, .. } = *ctx;
    let hops = hop_sites.len();
    // One batched lookup for the chain qualnames (final line) and the hop
    // caller files (pointer lines: the call site lives in the caller's file).
    let (qn, file) = chain_lookup(conn, chain_ids)?;
    let mut pointers: Vec<String> = Vec::with_capacity(hops);
    let mut hop_files: BTreeSet<String> = BTreeSet::new();
    for (i, &site) in hop_sites.iter().enumerate() {
        let caller = chain_ids[i];
        let Some(f) = file.get(&caller) else {
            return Err(AwError::Internal("path hop file missing".to_string()));
        };
        pointers.push(format!("{f}:{site}"));
        hop_files.insert(f.clone());
    }
    let (header, stale) = staleness_header(ctx, &hop_files);
    crate::journal::record_result(crate::journal::OutcomeCounts::uniform(hops), stale, 0);
    let mut out = Vec::with_capacity(hops + 4);
    out.push(PATH_CAVEAT.to_string());
    if let Some(header) = header {
        out.push(header);
    }
    out.extend(pointers);
    let mut chain_names: Vec<&str> = Vec::with_capacity(chain_ids.len());
    for id in chain_ids {
        let Some(q) = qn.get(id) else {
            return Err(AwError::Internal("path chain qualname missing".to_string()));
        };
        chain_names.push(q.as_str());
    }
    out.push(format!(
        "aw: chain {} ({hops} hops)",
        chain_names.join(" -> ")
    ));
    Ok(LineQuery {
        lines: out,
        stderr: String::new(),
        shown: hops,
    })
}

fn chain_lookup(
    conn: &Connection,
    chain_ids: &[i64],
) -> Result<(
    std::collections::HashMap<i64, String>,
    std::collections::HashMap<i64, String>,
)> {
    let placeholders = crate::db::sql_placeholders(chain_ids.len());
    let sql = format!(
        "SELECT s.id, s.qualname, f.path
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE s.id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let params = id_params(chain_ids);
    let mut qn: std::collections::HashMap<i64, String> = Default::default();
    let mut file: std::collections::HashMap<i64, String> = Default::default();
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    for r in rows {
        let (id, qualname, fpath) = r?;
        qn.insert(id, qualname);
        file.insert(id, fpath);
    }
    Ok((qn, file))
}

/// `path` BFS outcome (Amendment 13): the found chain with its per-hop call
/// sites, or the two negative shapes the exit-1 contract distinguishes.
enum PathSearch {
    Found {
        chain_ids: Vec<i64>,
        hop_sites: Vec<i64>,
    },
    Exhausted,
    CapTruncated,
}

/// Level-batched BFS over the resolved call-edge set (`kind = 'calls' AND
/// dst_id IS NOT NULL`). One indexed query per frontier level (chunked at
/// 500 ids per the build's insert-batch precedent), not a full-corpus
/// adjacency load and not a query per node. Expansion order matches the
/// pinned discipline exactly: frontier nodes in FIFO discovery order, each
/// node's neighbors sorted by `(dst qualname, dst path, site_line)`,
/// multi-edges deduped by the visited guard (the first row per pair carries
/// the smallest `site_line`), parent = first discoverer. Worst case fetches
/// every resolved call edge once; typical queries touch only the reachable
/// subgraph.
fn path_bfs(conn: &Connection, from_id: i64, to_id: i64, depth: usize) -> Result<PathSearch> {
    let mut parent: std::collections::HashMap<i64, (i64, i64)> = Default::default();
    let mut visited: std::collections::HashSet<i64> = Default::default();
    let mut frontier: Vec<i64> = vec![from_id];
    visited.insert(from_id);
    let mut found = false;
    let mut d = 0usize;
    while !found && !frontier.is_empty() && d < depth {
        let grouped = path_level_edges(conn, &frontier)?;
        let mut next: Vec<i64> = Vec::new();
        for &v in &frontier {
            let Some(neighbors) = grouped.get(&v) else {
                continue;
            };
            // Sorted by (dst qualname, dst path, site_line): the first
            // unvisited row per destination carries the smallest site_line.
            for (_, _, site, w) in neighbors {
                if visited.contains(w) {
                    continue;
                }
                visited.insert(*w);
                parent.insert(*w, (v, *site));
                if *w == to_id {
                    found = true;
                    break;
                }
                next.push(*w);
            }
            if found {
                break;
            }
        }
        frontier = next;
        d += 1;
    }
    if found {
        return reconstruct_chain(&parent, from_id, to_id);
    }
    if !frontier.is_empty() && d == depth {
        // Cap-truncation probe: an unvisited out-neighbor of any node at the
        // maximum distance means a longer path may exist.
        let grouped = path_level_edges(conn, &frontier)?;
        for neighbors in grouped.values() {
            if neighbors.iter().any(|(_, _, _, w)| !visited.contains(w)) {
                return Ok(PathSearch::CapTruncated);
            }
        }
    }
    Ok(PathSearch::Exhausted)
}

/// Out-edges of one BFS frontier, grouped per source and sorted by the pinned
/// neighbor key `(dst qualname, dst path, site_line)`.
fn path_level_edges(conn: &Connection, ids: &[i64]) -> Result<LevelEdges> {
    let mut grouped: LevelEdges = Default::default();
    crate::db::for_each_chunk(conn, ids, |conn, placeholders, chunk| {
        let sql = format!(
            "SELECT e.src_id, e.dst_id, e.site_line, ds.qualname, df.path
             FROM edges e
             JOIN symbols ds ON ds.id = e.dst_id
             JOIN files df ON df.id = ds.file_id
             WHERE e.kind = '{}' AND e.dst_id IS NOT NULL
               AND e.src_id IN ({placeholders})",
            EdgeKind::Calls.as_str()
        );
        let mut stmt = conn.prepare(&sql)?;
        let params = id_params(chunk);
        let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        for r in rows {
            let (src, dst, site, qn, fpath) = r?;
            grouped.entry(src).or_default().push((qn, fpath, site, dst));
        }
        Ok(())
    })?;
    for neighbors in grouped.values_mut() {
        neighbors.sort();
    }
    Ok(grouped)
}

/// Walk BFS parents back to the seed: chain node ids plus per-hop call sites.
fn reconstruct_chain(
    parent: &std::collections::HashMap<i64, (i64, i64)>,
    from_id: i64,
    to_id: i64,
) -> Result<PathSearch> {
    let mut chain_ids: Vec<i64> = vec![to_id];
    let mut hop_sites: Vec<i64> = Vec::new();
    let mut cur = to_id;
    while cur != from_id {
        let Some(&(p, site)) = parent.get(&cur) else {
            return Err(AwError::Internal(
                "path reconstruction lost a parent link".to_string(),
            ));
        };
        hop_sites.push(site);
        chain_ids.push(p);
        cur = p;
    }
    chain_ids.reverse();
    hop_sites.reverse();
    Ok(PathSearch::Found {
        chain_ids,
        hop_sites,
    })
}

/// One-call symbol context (Amendment 17): definition line with signature,
/// direct callers, and direct callees — replaces composing callers/callees/
/// symbols for "what does this symbol do and who uses it" questions.
pub fn context(ctx: &QueryCtx<'_>, name: &str, limit: usize) -> Result<LineQuery> {
    let QueryCtx { conn, .. } = *ctx;
    // The CLI documents max 50 per section; clamp here so MCP callers cannot
    // bypass the contract.
    let limit = clamp_limit(limit)?.min(50);
    let (ids, _) = resolve_targets(conn, name, None)?;
    let data = fetch_context(conn, ids[0], limit)?;
    render_context(ctx, &data)
}

/// Definition row plus one page of direct callers and callees.
struct ContextData {
    def_path: String,
    def_line: i64,
    def_end: i64,
    kind: String,
    qualname: String,
    signature: String,
    callers: Vec<CallEdgeRow>,
    callers_total: usize,
    callees: Vec<CallEdgeRow>,
    callees_total: usize,
}

fn fetch_context(conn: &Connection, id: i64, limit: usize) -> Result<ContextData> {
    let def_sql = "SELECT f.path, s.line_start, s.line_end, s.kind, s.qualname, s.signature
                   FROM symbols s JOIN files f ON f.id = s.file_id WHERE s.id = ?1";
    let (def_path, def_line, def_end, kind, qualname, signature): (
        String,
        i64,
        i64,
        String,
        String,
        String,
    ) = conn.query_row(def_sql, [id], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
        ))
    })?;
    let (callers, callers_total) = direct_callers(conn, &[id], limit)?;
    let (callees, callees_total) = direct_callees(conn, &[id], limit)?;
    Ok(ContextData {
        def_path,
        def_line,
        def_end,
        kind,
        qualname,
        signature,
        callers,
        callers_total,
        callees,
        callees_total,
    })
}

fn render_context(ctx: &QueryCtx<'_>, data: &ContextData) -> Result<LineQuery> {
    // Staleness over the displayed set (definition + shown callers/callees).
    let files = context_file_set(data);
    let (header, stale) = staleness_header(ctx, &files);
    // Caller/callee lines carry the identity suffix added in R7.
    let (caller_lines, callee_lines) = context_rows(data);
    let total = data.callers_total + data.callees_total;
    let shown = caller_lines.len() + callee_lines.len();
    crate::journal::record_result(crate::journal::OutcomeCounts::of(shown, total), stale, 0);
    let mut out = vec![CONTEXT_CAVEAT.to_string()];
    if let Some(header) = header {
        out.push(header);
    }
    let sig = if data.signature.is_empty() {
        String::new()
    } else {
        format!(" {}", data.signature)
    };
    out.push(format!(
        "{}:{}-{} {} {}{sig}",
        data.def_path, data.def_line, data.def_end, data.kind, data.qualname
    ));
    out.push(format!(
        "callers (showing {} of {}):",
        data.callers.len(),
        data.callers_total
    ));
    out.extend(caller_lines);
    out.push(format!(
        "callees (showing {} of {}):",
        data.callees.len(),
        data.callees_total
    ));
    out.extend(callee_lines);
    Ok(LineQuery {
        lines: out,
        stderr: String::new(),
        shown,
    })
}

fn context_file_set(data: &ContextData) -> BTreeSet<String> {
    let mut files = BTreeSet::new();
    files.insert(data.def_path.clone());
    files.extend(data.callers.iter().map(|row| row.path.clone()));
    files.extend(data.callees.iter().map(|row| row.path.clone()));
    files
}

fn context_rows(data: &ContextData) -> (Vec<String>, Vec<String>) {
    let caller_lines = data
        .callers
        .iter()
        .map(|row| format!("{}:{} {}", row.path, row.site_line, row.qualname))
        .collect();
    let callee_lines = data
        .callees
        .iter()
        .map(|row| {
            Pointer {
                file: row.path.clone(),
                line: row.site_line,
                tag: confidence_tag(&row.confidence),
                detail: row.qualname.clone(),
            }
            .render()
        })
        .collect();
    (caller_lines, callee_lines)
}
