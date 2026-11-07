//! Call-graph queries: imports, callers, callees, path, context.

use super::*;

/// Joins shared by caller-edge queries: the caller symbol, its file, and the
/// optional resolved destination.
const CALLER_EDGE_JOINS: &str = "JOIN symbols s ON s.id = e.src_id
         JOIN files f ON f.id = s.file_id
         LEFT JOIN symbols ds ON ds.id = e.dst_id";

pub fn imports(ctx: &QueryCtx<'_>, file: &str, limit: usize) -> Result<QueryOutput> {
    let QueryCtx { conn, .. } = *ctx;
    let limit = clamp_limit(limit)?;
    let rel = indexed_file(conn, file)?;
    let total: usize = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM edges e
         JOIN symbols s ON s.id = e.src_id JOIN files f ON f.id = s.file_id
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
         JOIN symbols s ON s.id = e.src_id JOIN files f ON f.id = s.file_id
         WHERE f.path = ?1 AND e.kind = '{}'
         ORDER BY e.site_line, e.dst_name LIMIT {limit}",
        EdgeKind::Imports.as_str()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([&rel], |r| {
        Ok(Pointer {
            file: rel.clone(),
            line: r.get(0)?,
            tag: "",
            detail: r.get(1)?,
        })
    })?;
    let mut out = collect_output(
        ctx,
        rows,
        limit,
        "0 imports found (external and unresolved imports are omitted)",
        total,
    )?;
    prepend_header(&mut out, IMPORTS_CAVEAT);
    Ok(out)
}

/// Shared traversal knobs for `callers`/`callees` (mirrors `ImpactOpts`).
pub struct GraphQueryOpts {
    pub limit: usize,
    pub depth: usize,
    pub explain: bool,
}

/// Shared `callers`/`callees` prologue: clamp the knobs and resolve the
/// target name once. Returns the clamped limit/depth, the explain flag, and
/// the target symbol ids.
fn prepare_graph_query(
    conn: &Connection,
    name: &str,
    opts: GraphQueryOpts,
) -> Result<(usize, usize, bool, Vec<i64>)> {
    let GraphQueryOpts {
        limit,
        depth,
        explain,
    } = opts;
    let limit = clamp_limit(limit)?;
    let depth = depth.clamp(1, 10);
    let (ids, _) = resolve_targets(conn, name, None)?;
    Ok((limit, depth, explain, ids))
}

pub fn callers(ctx: &QueryCtx<'_>, name: &str, opts: GraphQueryOpts) -> Result<QueryOutput> {
    let QueryCtx { conn, .. } = *ctx;
    let (limit, depth, explain, ids) = prepare_graph_query(conn, name, opts)?;
    if depth > 1 {
        return callers_transitive(ctx, &ids, limit, depth, explain);
    }

    let (callers, total) = direct_callers(conn, &ids, limit)?;
    let rows = callers.into_iter().map(|row| {
        let detail = caller_detail(conn, &row, explain);
        Ok(Pointer {
            file: row.path,
            line: row.site_line,
            tag: confidence_tag(&row.confidence),
            detail,
        })
    });
    let out = collect_output(
        ctx,
        rows,
        limit,
        "0 callers found (dynamic dispatch possible — grep fallback recommended)",
        total,
    )?;
    Ok(with_explain_header(out, explain))
}

/// Transitive callers (depth > 1), symmetric with `callees_transitive`.
fn callers_transitive(
    ctx: &QueryCtx<'_>,
    ids: &[i64],
    limit: usize,
    depth: usize,
    explain: bool,
) -> Result<QueryOutput> {
    let QueryCtx { conn, .. } = *ctx;
    let (rows, total) = transitive_callers(conn, ids, depth, limit, explain)?;
    let mut out = collect_output(
        ctx,
        rows.into_iter().map(Ok),
        limit,
        "0 callers found (dynamic dispatch possible — grep fallback recommended)",
        total,
    )?;
    let caveat = format!("aw: transitive callers — resolved static call edges only; depth {depth}");
    prepend_header(&mut out, &caveat);
    Ok(with_explain_header(out, explain))
}

/// Reverse call-edge BFS. Each caller symbol is expanded once at its shortest
/// distance; all call sites at that distance are kept. Rows are bounded by the
/// display limit while `total` counts every reachable caller row.
fn transitive_callers(
    conn: &Connection,
    targets: &[i64],
    depth: usize,
    limit: usize,
    explain: bool,
) -> Result<(Vec<Pointer>, usize)> {
    let mut visited: HashSet<i64> = targets.iter().copied().collect();
    let mut frontier: Vec<i64> = targets.to_vec();
    let mut displayed: Vec<Pointer> = Vec::new();
    let mut total = 0usize;
    for level in 1..=depth {
        if frontier.is_empty() {
            break;
        }
        let mut next: Vec<i64> = Vec::new();
        let mut queued: HashSet<i64> = HashSet::new();
        crate::db::for_each_chunk(conn, &frontier, |conn, _, chunk| {
            for row in fetch_level_rows(conn, chunk)? {
                if visited.contains(&row.src_id) {
                    continue;
                }
                if queued.insert(row.src_id) {
                    next.push(row.src_id);
                }
                total += 1;
                if displayed.len() < limit {
                    displayed.push(caller_pointer(row, level, explain));
                }
            }
            Ok(())
        })?;
        for id in &next {
            visited.insert(*id);
        }
        frontier = next;
    }
    Ok((displayed, total))
}

/// One transitive-caller pointer row: qualname plus the depth suffix, with
/// the edge/confidence/dst metadata appended under `--explain`.
fn caller_pointer(row: CallerLevelRow, level: usize, explain: bool) -> Pointer {
    let CallerLevelRow {
        src_id: _,
        site_line,
        confidence,
        path,
        qualname,
        dst,
    } = row;
    let tag = confidence_tag(&confidence);
    let detail = if explain {
        format!("{qualname} [depth={level}] [edge=calls confidence={confidence} dst={dst}]")
    } else {
        format!("{qualname} [depth={level}]")
    };
    Pointer {
        file: path,
        line: site_line,
        tag,
        detail,
    }
}

/// One transitive-caller row at a BFS level. Named fields replace the
/// positional 6-tuple so the three adjacent `String` columns cannot be
/// silently swapped between the SELECT mapping and the BFS destructure.
struct CallerLevelRow {
    src_id: i64,
    site_line: i64,
    confidence: String,
    path: String,
    qualname: String,
    dst: String,
}

fn fetch_level_rows(conn: &Connection, chunk: &[i64]) -> Result<Vec<CallerLevelRow>> {
    let placeholders = crate::db::sql_placeholders(chunk.len());
    let sql = format!(
        "SELECT e.src_id, e.site_line, e.confidence, f.path, s.qualname,
                COALESCE(ds.qualname, e.dst_name)
         FROM edges e
         {CALLER_EDGE_JOINS}
         WHERE e.dst_id IN ({placeholders}) AND e.kind = '{}'
         ORDER BY f.path, e.site_line, s.qualname",
        EdgeKind::Calls.as_str()
    );
    let mut stmt = conn.prepare(&sql)?;
    let params = id_params(chunk);
    let mut rows = stmt.query(rusqlite::params_from_iter(params.iter()))?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(CallerLevelRow {
            src_id: row.get(0)?,
            site_line: row.get(1)?,
            confidence: row.get(2)?,
            path: row.get(3)?,
            qualname: row.get(4)?,
            dst: row.get(5)?,
        });
    }
    Ok(out)
}

/// Shared `--explain` suffix for AMBIGUOUS edges.
const AMBIGUOUS_RESOLUTION: &str = " resolution=ambiguous (candidate set not stored)";

/// Edge metadata suffix for `--explain` on direct caller rows.
fn explain_edge(confidence: &str, site: i64, dst: &str) -> String {
    let mut text = format!(" [edge=calls confidence={confidence} site={site} dst={dst}]");
    if Confidence::parse(confidence) == Some(Confidence::Ambiguous) {
        text.push_str(AMBIGUOUS_RESOLUTION);
    }
    text
}

/// Edge metadata suffix for grouped direct callee rows.
fn explain_callee(confidence: &str, edges: i64, first: i64, last: i64, dst: &str) -> String {
    let sites = if edges <= 1 || first == last {
        format!("site={first}")
    } else {
        format!("sites={first}..{last}")
    };
    let mut text = format!(" [edge=calls confidence={confidence} edges={edges} {sites} dst={dst}]");
    if Confidence::parse(confidence) == Some(Confidence::Ambiguous) {
        text.push_str(AMBIGUOUS_RESOLUTION);
    }
    text
}

/// Typed-receiver hint stored as a `type_ref` edge at the same call site.
fn receiver_hint(conn: &Connection, src_id: i64, site_line: i64) -> Option<String> {
    conn.query_row(
        &format!(
            "SELECT ds.qualname
         FROM edges e JOIN symbols ds ON ds.id = e.dst_id
         WHERE e.src_id = ?1 AND e.site_line = ?2 AND e.kind = '{}'
         ORDER BY ds.qualname LIMIT 1",
            EdgeKind::TypeRef.as_str()
        ),
        rusqlite::params![src_id, site_line],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

/// Rendering detail shared by `callers` and `context`: qualname plus the
/// `--explain` edge suffix and typed-receiver hint.
fn caller_detail(conn: &Connection, row: &CallerRow, explain: bool) -> String {
    let mut detail = row.qualname.clone();
    if explain {
        detail.push_str(&explain_edge(&row.confidence, row.site_line, &row.dst_name));
        if let Some(receiver) = receiver_hint(conn, row.src_id, row.site_line) {
            detail.push_str(&format!(" receiver={receiver}"));
        }
    }
    detail
}

/// Rendering detail shared by `callees` and `context`: qualname plus the
/// `--explain` edge suffix.
fn callee_detail(row: &CalleeRow, explain: bool) -> String {
    let mut detail = row.qualname.clone();
    if explain {
        detail.push_str(&explain_callee(
            &row.confidence,
            row.edges,
            row.first_site,
            row.last_site,
            &row.qualname,
        ));
    }
    detail
}

/// One direct caller row. Private loader output; rendering and journaling stay
/// at the public surface.
struct CallerRow {
    site_line: i64,
    confidence: String,
    path: String,
    qualname: String,
    src_id: i64,
    dst_name: String,
}

/// One direct callee row (depth 1). `confidence` is carried so `callees` and
/// `context` keep the same ambiguous tagging.
struct CalleeRow {
    line: i64,
    confidence: String,
    path: String,
    qualname: String,
    edges: i64,
    first_site: i64,
    last_site: i64,
}

/// Side-effect-free direct-caller load. The count and the bounded display set
/// use the same predicate; callers/context render them differently.
fn direct_callers(conn: &Connection, ids: &[i64], limit: usize) -> Result<(Vec<CallerRow>, usize)> {
    let placeholders = crate::db::sql_placeholders(ids.len());
    let params = id_params(ids);
    let count_sql = format!(
        "SELECT COUNT(*) FROM edges e WHERE e.dst_id IN ({placeholders}) AND e.kind = '{}'",
        EdgeKind::Calls.as_str()
    );
    let total: usize =
        conn.query_row(&count_sql, rusqlite::params_from_iter(params.iter()), |r| {
            r.get::<_, i64>(0)
        })? as usize;
    let sql = format!(
        "SELECT e.site_line, e.confidence, f.path, s.qualname, e.src_id,
                COALESCE(ds.qualname, e.dst_name)
         FROM edges e
         {CALLER_EDGE_JOINS}
         WHERE e.dst_id IN ({placeholders}) AND e.kind = '{}'
         ORDER BY f.path, e.site_line LIMIT {limit}",
        EdgeKind::Calls.as_str()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok(CallerRow {
                site_line: r.get(0)?,
                confidence: r.get(1)?,
                path: r.get(2)?,
                qualname: r.get(3)?,
                src_id: r.get(4)?,
                dst_name: r.get(5)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok((rows, total))
}

/// Side-effect-free direct-callee load. Mirrors the depth-1 SQL used by
/// `callees`; `context` reuses it with a single id.
fn direct_callees(conn: &Connection, ids: &[i64], limit: usize) -> Result<(Vec<CalleeRow>, usize)> {
    let placeholders = crate::db::sql_placeholders(ids.len());
    let params = id_params(ids);
    let count_sql = format!(
        "SELECT COUNT(DISTINCT COALESCE(ds.id, -(1000000 + e.id)))
         FROM edges e LEFT JOIN symbols ds ON ds.id = e.dst_id
         WHERE e.src_id IN ({placeholders}) AND e.kind = '{}'",
        EdgeKind::Calls.as_str()
    );
    let total: usize =
        conn.query_row(&count_sql, rusqlite::params_from_iter(params.iter()), |r| {
            r.get::<_, i64>(0)
        })? as usize;
    let sql = format!(
        "SELECT MIN(COALESCE(ds.line_start, e.site_line)) AS line,
                MAX(e.confidence) AS confidence,
                MAX(COALESCE(df.path, sf.path)) AS path,
                COALESCE(ds.qualname, e.dst_name) AS qn,
                COUNT(*) AS edges,
                MIN(e.site_line) AS first_site,
                MAX(e.site_line) AS last_site
         FROM edges e
         JOIN symbols s ON s.id = e.src_id JOIN files sf ON sf.id = s.file_id
         LEFT JOIN symbols ds ON ds.id = e.dst_id
         LEFT JOIN files df ON df.id = ds.file_id
         WHERE e.src_id IN ({placeholders}) AND e.kind = '{}'
         GROUP BY COALESCE(ds.id, -(1000000 + e.id))
         ORDER BY path, line LIMIT {limit}",
        EdgeKind::Calls.as_str()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok(CalleeRow {
                line: r.get(0)?,
                confidence: r.get(1)?,
                path: r.get(2)?,
                qualname: r.get(3)?,
                edges: r.get(4)?,
                first_site: r.get(5)?,
                last_site: r.get(6)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok((rows, total))
}

pub fn implementers(ctx: &QueryCtx<'_>, name: &str, limit: usize) -> Result<QueryOutput> {
    let QueryCtx { conn, .. } = *ctx;
    let limit = clamp_limit(limit)?;
    let (ids, _) = resolve_targets(conn, name, None)?;
    let placeholders = crate::db::sql_placeholders(ids.len());
    let params = id_params(&ids);
    let total: usize = conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT e.src_id) FROM edges e
             WHERE e.dst_id IN ({placeholders}) AND e.kind = '{}'",
            EdgeKind::Inherits.as_str()
        ),
        rusqlite::params_from_iter(params.iter()),
        |r| r.get::<_, i64>(0),
    )? as usize;
    let sql = format!(
        "SELECT DISTINCT f.path, s.line_start, s.qualname
         FROM edges e
         JOIN symbols s ON s.id = e.src_id
         JOIN files f ON f.id = s.file_id
         WHERE e.dst_id IN ({placeholders}) AND e.kind = '{}'
         ORDER BY f.path, s.line_start, s.qualname LIMIT {limit}",
        EdgeKind::Inherits.as_str()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        Ok(Pointer {
            file: r.get(0)?,
            line: r.get(1)?,
            tag: "",
            detail: r.get(2)?,
        })
    })?;
    if total == 0 {
        return Err(implementer_empty_reason(conn, name)?);
    }
    let mut out = collect_output(
        ctx,
        rows,
        limit,
        "0 implementers found (resolved inherits edges only)",
        total,
    )?;
    prepend_header(&mut out, IMPLEMENTERS_CAVEAT);
    Ok(out)
}

fn implementer_empty_reason(conn: &Connection, name: &str) -> Result<AwError> {
    let bare = name.rsplit('.').next().unwrap_or(name);
    let unresolved: i64 = conn.query_row(
        "SELECT COUNT(*) FROM unresolved_calls
         WHERE kind = 'inherit' AND (dst_name = ?1 OR dst_name = ?2)",
        [name, bare],
        |r| r.get(0),
    )?;
    let detail = if unresolved > 0 {
        format!(
            "0 implementers found (resolved inherits edges only; {unresolved} unresolved inherit sites may refer to this symbol)"
        )
    } else {
        "0 implementers found (resolved inherits edges only)".to_string()
    };
    Ok(AwError::NoResultsCaveated {
        caveat: IMPLEMENTERS_CAVEAT,
        detail,
    })
}

/// Explain an empty `callees` result instead of reporting a resolver gap as
/// absence. Unresolved call sites are attributed to the queried symbol by
/// its file and line range; names are capped so the detail stays one line.
fn callee_empty_reason(conn: &Connection, ids: &[i64], total: usize) -> Result<String> {
    if total > 0 {
        return Ok("0 callees found".to_string());
    }
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
    let mut stmt = conn.prepare(
        "SELECT DISTINCT u.dst_name
           FROM unresolved_calls u
           JOIN symbols s ON s.file_id = u.file_id
          WHERE s.id = ?1 AND u.kind = 'call'
            AND u.site_line >= s.line_start
            AND u.site_line <= s.line_end
          ORDER BY u.dst_name",
    )?;
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
    let (limit, depth, explain, ids) = prepare_graph_query(conn, name, opts)?;
    if depth == 1 {
        callees_direct(ctx, &ids, limit, explain)
    } else {
        callees_transitive(ctx, &ids, limit, depth, explain)
    }
}

fn callees_direct(
    ctx: &QueryCtx<'_>,
    ids: &[i64],
    limit: usize,
    explain: bool,
) -> Result<QueryOutput> {
    let QueryCtx { conn, .. } = *ctx;
    let (callees, total) = direct_callees(conn, ids, limit)?;
    let rows = callees.into_iter().map(|row| {
        let detail = callee_detail(&row, explain);
        Ok(Pointer {
            file: row.path,
            line: row.line,
            tag: confidence_tag(&row.confidence),
            detail,
        })
    });
    let empty_reason = callee_empty_reason(conn, ids, total)?;
    let out = collect_output(ctx, rows, limit, &empty_reason, total)?;
    Ok(with_explain_header(out, explain))
}

fn callees_transitive(
    ctx: &QueryCtx<'_>,
    ids: &[i64],
    limit: usize,
    depth: usize,
    explain: bool,
) -> Result<QueryOutput> {
    let QueryCtx { conn, .. } = *ctx;
    // depth > 1: recursive closure over call edges from the reached set.
    let mut params = id_params(ids);
    let (cte, select) = transitive_sql(ids.len());
    params.push(rusqlite::types::Value::from(depth as i64));
    params.push(rusqlite::types::Value::from(depth as i64));
    let count_sql = format!("{cte} SELECT COUNT(*) FROM ({select})");
    let total: usize =
        conn.query_row(&count_sql, rusqlite::params_from_iter(params.iter()), |r| {
            r.get::<_, i64>(0)
        })? as usize;
    // Bounded materialization (R6 round-2): the closure result is counted
    // above and fetched with LIMIT, never fully collected.
    let sql = format!("{cte} {select} ORDER BY path, line LIMIT {limit}");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        let line: i64 = r.get(0)?;
        let conf: String = r.get(1)?;
        let path: String = r.get(2)?;
        let qualname: String = r.get(3)?;
        let detail = if explain {
            format!("{qualname} [edge=calls confidence={conf} projection=path]")
        } else {
            qualname
        };
        Ok(Pointer {
            file: path,
            line,
            tag: confidence_tag(&conf),
            detail,
        })
    })?;
    let empty_reason = callee_empty_reason(conn, ids, total)?;
    let out = collect_output(ctx, rows, limit, &empty_reason, total)?;
    Ok(with_explain_header(out, explain))
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
                COALESCE(df.path, sf.path) AS path, COALESCE(ds.qualname, e.dst_name) AS qn
         FROM reach r
         -- CROSS JOIN + WHERE lets SQLite drive from the recursive set and
         -- use edges_src, instead of scanning the whole edge table (R7).
         CROSS JOIN edges e
         JOIN symbols ss ON ss.id = e.src_id JOIN files sf ON sf.id = ss.file_id
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
pub fn path(ctx: &QueryCtx<'_>, from: &str, to: &str, depth: usize) -> Result<Vec<String>> {
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
fn render_path(ctx: &QueryCtx<'_>, chain_ids: &[i64], hop_sites: &[i64]) -> Result<Vec<String>> {
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
    Ok(out)
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
pub fn context(ctx: &QueryCtx<'_>, name: &str, limit: usize, explain: bool) -> Result<Vec<String>> {
    let QueryCtx { conn, .. } = *ctx;
    // The CLI documents max 50 per section; clamp here so MCP callers cannot
    // bypass the contract.
    let limit = clamp_limit(limit)?.min(50);
    let (ids, _) = resolve_targets(conn, name, None)?;
    let data = fetch_context(conn, ids[0], limit)?;
    render_context(ctx, &data, explain)
}

/// Definition row plus one page of direct callers and callees.
struct ContextData {
    def_path: String,
    def_line: i64,
    def_end: i64,
    kind: String,
    qualname: String,
    signature: String,
    callers: Vec<CallerRow>,
    callers_total: usize,
    callees: Vec<CalleeRow>,
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

fn render_context(ctx: &QueryCtx<'_>, data: &ContextData, explain: bool) -> Result<Vec<String>> {
    let QueryCtx { conn, .. } = *ctx;
    // Staleness over the displayed set (definition + shown callers/callees).
    let files = context_file_set(data);
    let (header, stale) = staleness_header(ctx, &files);
    // Caller/callee lines carry the identity suffix added in R7.
    let (caller_lines, callee_lines) = context_rows(conn, data, explain);
    let total = data.callers_total + data.callees_total;
    let shown = caller_lines.len() + callee_lines.len();
    crate::journal::record_result(crate::journal::OutcomeCounts::of(shown, total), stale, 0);
    let mut out = vec![CONTEXT_CAVEAT.to_string()];
    if explain {
        out.push(EXPLAIN_CAVEAT.to_string());
    }
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
    Ok(out)
}

fn context_file_set(data: &ContextData) -> BTreeSet<String> {
    let mut files = BTreeSet::new();
    files.insert(data.def_path.clone());
    files.extend(data.callers.iter().map(|row| row.path.clone()));
    files.extend(data.callees.iter().map(|row| row.path.clone()));
    files
}

fn context_rows(
    conn: &Connection,
    data: &ContextData,
    explain: bool,
) -> (Vec<String>, Vec<String>) {
    let caller_lines = data
        .callers
        .iter()
        .map(|row| {
            format!(
                "{}:{} {}",
                row.path,
                row.site_line,
                caller_detail(conn, row, explain)
            )
        })
        .collect();
    let callee_lines = data
        .callees
        .iter()
        .map(|row| {
            Pointer {
                file: row.path.clone(),
                line: row.line,
                tag: confidence_tag(&row.confidence),
                detail: callee_detail(row, explain),
            }
            .render()
        })
        .collect();
    (caller_lines, callee_lines)
}
