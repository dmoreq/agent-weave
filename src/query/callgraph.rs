//! Call-graph queries: imports, callers, callees, path, context.

use super::*;

pub fn imports(conn: &Connection, root: &Path, file: &str, limit: usize) -> Result<QueryOutput> {
    let limit = clamp_limit(limit)?;
    let rel = file.trim_start_matches("./").to_string();
    let exists: bool = conn
        .query_row("SELECT COUNT(*) FROM files WHERE path = ?1", [&rel], |r| {
            r.get(0)
        })
        .map(|c: i64| c > 0)?;
    if !exists {
        return Err(AwError::NoResults(format!("path '{file}' not indexed")));
    }
    let total: usize = conn.query_row(
        "SELECT COUNT(*) FROM edges e
         JOIN symbols s ON s.id = e.src_id JOIN files f ON f.id = s.file_id
         WHERE f.path = ?1 AND e.kind = 'imports'",
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
         WHERE f.path = ?1 AND e.kind = 'imports'
         ORDER BY e.site_line, e.dst_name LIMIT {limit}"
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
        conn,
        root,
        rows,
        limit,
        "0 imports found (external and unresolved imports are omitted)",
        Some(total),
    )?;
    out.header = Some(match out.header.take() {
        Some(stale) => format!("{IMPORTS_CAVEAT}\n{stale}"),
        None => IMPORTS_CAVEAT.to_string(),
    });
    Ok(out)
}

pub fn callers(
    conn: &Connection,
    root: &Path,
    name: &str,
    limit: usize,
    depth: usize,
    explain: bool,
) -> Result<QueryOutput> {
    let limit = clamp_limit(limit)?;
    let depth = depth.clamp(1, 10);
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
            role: None,
        });
    }
    if depth > 1 {
        let (rows, total) = transitive_callers(conn, &ids, depth, limit, explain)?;
        let mut out = collect_output(
            conn,
            root,
            rows.into_iter().map(Ok),
            limit,
            "0 callers found (dynamic dispatch possible — grep fallback recommended)",
            Some(total),
        )?;
        let caveat =
            format!("aw: transitive callers — resolved static call edges only; depth {depth}");
        out.header = Some(match out.header.take() {
            Some(stale) => format!("{caveat}\n{stale}"),
            None => caveat,
        });
        return Ok(with_explain_header(out, explain));
    }

    let (callers, total) = direct_callers(conn, &ids, limit)?;
    let rows = callers.into_iter().map(|row| {
        let mut detail = row.qualname;
        if explain {
            detail.push_str(&explain_edge(
                "calls",
                &row.confidence,
                row.site_line,
                &row.dst_name,
            ));
            if let Some(receiver) = receiver_hint(conn, row.src_id, row.site_line) {
                detail.push_str(&format!(" receiver={receiver}"));
            }
        }
        Ok(Pointer {
            file: row.path,
            line: row.site_line,
            tag: if row.confidence == "AMBIGUOUS" {
                " [ambiguous]"
            } else {
                ""
            },
            detail,
        })
    });
    let out = collect_output(
        conn,
        root,
        rows,
        limit,
        "0 callers found (dynamic dispatch possible — grep fallback recommended)",
        Some(total),
    )?;
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
        for chunk in frontier.chunks(500) {
            let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT e.src_id, e.site_line, e.confidence, f.path, s.qualname,
                        COALESCE(ds.qualname, e.dst_name)
                 FROM edges e
                 JOIN symbols s ON s.id = e.src_id
                 JOIN files f ON f.id = s.file_id
                 LEFT JOIN symbols ds ON ds.id = e.dst_id
                 WHERE e.dst_id IN ({placeholders}) AND e.kind = 'calls'
                 ORDER BY f.path, e.site_line, s.qualname"
            );
            let mut stmt = conn.prepare(&sql)?;
            let params: Vec<rusqlite::types::Value> = chunk
                .iter()
                .map(|i| rusqlite::types::Value::from(*i))
                .collect();
            let mut rows = stmt.query(rusqlite::params_from_iter(params.iter()))?;
            while let Some(row) = rows.next()? {
                let src: i64 = row.get(0)?;
                if visited.contains(&src) {
                    continue;
                }
                let site: i64 = row.get(1)?;
                let conf: String = row.get(2)?;
                let path: String = row.get(3)?;
                let qualname: String = row.get(4)?;
                let dst: String = row.get(5)?;
                if queued.insert(src) {
                    next.push(src);
                }
                total += 1;
                if displayed.len() < limit {
                    let detail = if explain {
                        format!(
                            "{qualname} [depth={level}] [edge=calls confidence={conf} dst={dst}]"
                        )
                    } else {
                        format!("{qualname} [depth={level}]")
                    };
                    displayed.push(Pointer {
                        file: path,
                        line: site,
                        tag: if conf == "AMBIGUOUS" {
                            " [ambiguous]"
                        } else {
                            ""
                        },
                        detail,
                    });
                }
            }
        }
        for id in &next {
            visited.insert(*id);
        }
        frontier = next;
    }
    Ok((displayed, total))
}

/// Edge metadata suffix for `--explain` on direct caller rows.
fn explain_edge(kind: &str, confidence: &str, site: i64, dst: &str) -> String {
    let mut text = format!(" [edge={kind} confidence={confidence} site={site} dst={dst}]");
    if confidence == "AMBIGUOUS" {
        text.push_str(" resolution=ambiguous (candidate set not stored)");
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
    if confidence == "AMBIGUOUS" {
        text.push_str(" resolution=ambiguous (candidate set not stored)");
    }
    text
}

/// Typed-receiver hint stored as a `type_ref` edge at the same call site.
fn receiver_hint(conn: &Connection, src_id: i64, site_line: i64) -> Option<String> {
    conn.query_row(
        "SELECT ds.qualname
         FROM edges e JOIN symbols ds ON ds.id = e.dst_id
         WHERE e.src_id = ?1 AND e.site_line = ?2 AND e.kind = 'type_ref'
         ORDER BY ds.qualname LIMIT 1",
        rusqlite::params![src_id, site_line],
        |row| row.get::<_, String>(0),
    )
    .ok()
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
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let params: Vec<rusqlite::types::Value> = ids
        .iter()
        .map(|i| rusqlite::types::Value::from(*i))
        .collect();
    let count_sql = format!(
        "SELECT COUNT(*) FROM edges e WHERE e.dst_id IN ({placeholders}) AND e.kind = 'calls'"
    );
    let total: usize =
        conn.query_row(&count_sql, rusqlite::params_from_iter(params.iter()), |r| {
            r.get::<_, i64>(0)
        })? as usize;
    let sql = format!(
        "SELECT e.site_line, e.confidence, f.path, s.qualname, e.src_id,
                COALESCE(ds.qualname, e.dst_name)
         FROM edges e
         JOIN symbols s ON s.id = e.src_id
         JOIN files f ON f.id = s.file_id
         LEFT JOIN symbols ds ON ds.id = e.dst_id
         WHERE e.dst_id IN ({placeholders}) AND e.kind = 'calls'
         ORDER BY f.path, e.site_line LIMIT {limit}"
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
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let params: Vec<rusqlite::types::Value> = ids
        .iter()
        .map(|i| rusqlite::types::Value::from(*i))
        .collect();
    let count_sql = format!(
        "SELECT COUNT(DISTINCT COALESCE(ds.id, -(1000000 + e.id)))
         FROM edges e LEFT JOIN symbols ds ON ds.id = e.dst_id
         WHERE e.src_id IN ({placeholders}) AND e.kind = 'calls'"
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
         WHERE e.src_id IN ({placeholders}) AND e.kind = 'calls'
         GROUP BY COALESCE(ds.id, -(1000000 + e.id))
         ORDER BY path, line LIMIT {limit}"
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

pub fn implementers(
    conn: &Connection,
    root: &Path,
    name: &str,
    limit: usize,
) -> Result<QueryOutput> {
    let limit = clamp_limit(limit)?;
    let (ids, cands) = target_ids(conn, name)?;
    if ids.is_empty() {
        return Err(AwError::NoResults(format!("symbol '{name}' not in index")));
    }
    if cands.len() > 1 {
        return Err(AwError::Ambiguous {
            name: name.to_string(),
            total: ids.len(),
            candidates: cands,
            role: None,
        });
    }
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let params: Vec<rusqlite::types::Value> = ids
        .iter()
        .map(|i| rusqlite::types::Value::from(*i))
        .collect();
    let total: usize = conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT e.src_id) FROM edges e
             WHERE e.dst_id IN ({placeholders}) AND e.kind = 'inherits'"
        ),
        rusqlite::params_from_iter(params.iter()),
        |r| r.get::<_, i64>(0),
    )? as usize;
    let sql = format!(
        "SELECT DISTINCT f.path, s.line_start, s.qualname
         FROM edges e
         JOIN symbols s ON s.id = e.src_id
         JOIN files f ON f.id = s.file_id
         WHERE e.dst_id IN ({placeholders}) AND e.kind = 'inherits'
         ORDER BY f.path, s.line_start, s.qualname LIMIT {limit}"
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
        return Err(AwError::NoResultsCaveated {
            caveat: IMPLEMENTERS_CAVEAT,
            detail,
        });
    }
    let mut out = collect_output(
        conn,
        root,
        rows,
        limit,
        "0 implementers found (resolved inherits edges only)",
        Some(total),
    )?;
    out.header = Some(match out.header.take() {
        Some(stale) => format!("{IMPLEMENTERS_CAVEAT}\n{stale}"),
        None => IMPLEMENTERS_CAVEAT.to_string(),
    });
    Ok(out)
}

/// Explain an empty `callees` result instead of reporting a resolver gap as
/// absence. Unresolved call sites are attributed to the queried symbol by
/// its file and line range; names are capped so the detail stays one line.
fn callee_empty_reason(conn: &Connection, ids: &[i64]) -> Result<String> {
    const SHOWN: usize = 5;
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut file: Option<String> = None;
    for id in ids {
        if file.is_none() {
            file = conn
                .query_row(
                    "SELECT f.path FROM symbols s JOIN files f ON f.id = s.file_id WHERE s.id = ?1",
                    [*id],
                    |r| r.get::<_, String>(0),
                )
                .ok();
        }
        let mut stmt = conn.prepare(
            "SELECT DISTINCT u.dst_name
               FROM unresolved_calls u
               JOIN symbols s ON s.file_id = u.file_id
              WHERE s.id = ?1 AND u.kind = 'call'
                AND u.site_line >= s.line_start
                AND u.site_line <= s.line_end
              ORDER BY u.dst_name",
        )?;
        let rows = stmt.query_map([*id], |r| r.get::<_, String>(0))?;
        for row in rows {
            names.insert(row?);
        }
    }
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
    let hint = match file {
        Some(path) => format!(" — run aw unresolved --names --path {path}"),
        None => " — run aw unresolved --names".to_string(),
    };
    Ok(format!(
        "0 callees found (resolved static call edges only); {} unresolved call site(s) inside the queried symbol: {shown}{more}{hint}",
        names.len()
    ))
}

pub fn callees(
    conn: &Connection,
    root: &Path,
    name: &str,
    limit: usize,
    depth: usize,
    explain: bool,
) -> Result<QueryOutput> {
    let limit = clamp_limit(limit)?;
    let depth = depth.clamp(1, 10);
    let (ids, cands) = target_ids(conn, name)?;
    if ids.is_empty() {
        return Err(AwError::NoResults(format!("symbol '{name}' not in index")));
    }
    if cands.len() > 1 {
        return Err(AwError::Ambiguous {
            name: name.to_string(),
            total: ids.len(),
            candidates: cands,
            role: None,
        });
    }
    // Callee pointer: the callee's definition line. AMBIGUOUS edges have no
    // dst symbol; they surface as the call site itself with the ambiguous tag.
    if depth == 1 {
        let (callees, total) = direct_callees(conn, &ids, limit)?;
        let rows = callees.into_iter().map(|row| {
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
            Ok(Pointer {
                file: row.path,
                line: row.line,
                tag: if row.confidence == "AMBIGUOUS" {
                    " [ambiguous]"
                } else {
                    ""
                },
                detail,
            })
        });
        let empty_reason = if total == 0 {
            callee_empty_reason(conn, &ids)?
        } else {
            "0 callees found".to_string()
        };
        let out = collect_output(conn, root, rows, limit, &empty_reason, Some(total))?;
        return Ok(with_explain_header(out, explain));
    }
    // depth > 1: recursive closure over call edges from the reached set.
    let mut params: Vec<rusqlite::types::Value> = ids
        .iter()
        .map(|i| rusqlite::types::Value::from(*i))
        .collect();
    let seed = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let cte = format!(
        "WITH RECURSIVE reach(sym, d) AS (
             SELECT id, 0 FROM symbols WHERE id IN ({seed})
             UNION
             SELECT e.dst_id, r.d + 1 FROM edges e JOIN reach r ON e.src_id = r.sym
             WHERE e.kind = 'calls' AND e.dst_id IS NOT NULL AND r.d < ?{d}
         )",
        d = 1 + ids.len()
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
         WHERE e.src_id = r.sym AND e.kind = 'calls' AND r.d < ?{d2}",
        d2 = 2 + ids.len()
    );
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
            tag: if conf == "AMBIGUOUS" {
                " [ambiguous]"
            } else {
                ""
            },
            detail,
        })
    })?;
    let empty_reason = if total == 0 {
        callee_empty_reason(conn, &ids)?
    } else {
        "0 callees found".to_string()
    };
    let out = collect_output(conn, root, rows, limit, &empty_reason, Some(total))?;
    Ok(with_explain_header(out, explain))
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
pub fn path(
    conn: &Connection,
    root: &Path,
    from: &str,
    to: &str,
    depth: usize,
) -> Result<Vec<String>> {
    let depth = depth.clamp(1, 10);
    let (from_ids, from_cands) = target_ids(conn, from)?;
    if from_ids.is_empty() {
        return Err(AwError::NoResults(format!("symbol '{from}' not in index")));
    }
    if from_cands.len() > 1 {
        return Err(AwError::Ambiguous {
            name: from.to_string(),
            total: from_ids.len(),
            candidates: from_cands,
            role: Some("from"),
        });
    }
    let (to_ids, to_cands) = target_ids(conn, to)?;
    if to_ids.is_empty() {
        return Err(AwError::NoResults(format!("symbol '{to}' not in index")));
    }
    if to_cands.len() > 1 {
        return Err(AwError::Ambiguous {
            name: to.to_string(),
            total: to_ids.len(),
            candidates: to_cands,
            role: Some("to"),
        });
    }
    let from_id = from_ids[0];
    let to_id = to_ids[0];
    let from_qn = from_cands[0].2.clone();
    let to_qn = to_cands[0].2.clone();
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
    let hops = hop_sites.len();
    // One batched lookup for the chain qualnames (final line) and the hop
    // caller files (pointer lines: the call site lives in the caller's file).
    let placeholders = chain_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT s.id, s.qualname, f.path
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE s.id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<rusqlite::types::Value> = chain_ids
        .iter()
        .map(|i| rusqlite::types::Value::from(*i))
        .collect();
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
    let n_files = hop_files.len();
    let mut changed = 0usize;
    for f in &hop_files {
        if file_changed(conn, root, f) {
            changed += 1;
        }
    }
    let stale = changed > 0;
    crate::journal::record_result(hops, hops, hops, stale, 0);
    let mut out = Vec::with_capacity(hops + 4);
    out.push(PATH_CAVEAT.to_string());
    if stale {
        out.push(format!(
            "aw: {changed} of {n_files} result files changed since build — run aw build"
        ));
    }
    out.extend(pointers);
    let mut chain_names: Vec<&str> = Vec::with_capacity(chain_ids.len());
    for id in &chain_ids {
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
    let level_edges = |ids: &[i64]| -> Result<LevelEdges> {
        let mut grouped: LevelEdges = Default::default();
        for chunk in ids.chunks(500) {
            let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT e.src_id, e.dst_id, e.site_line, ds.qualname, df.path
                 FROM edges e
                 JOIN symbols ds ON ds.id = e.dst_id
                 JOIN files df ON df.id = ds.file_id
                 WHERE e.kind = 'calls' AND e.dst_id IS NOT NULL
                   AND e.src_id IN ({placeholders})"
            );
            let mut stmt = conn.prepare(&sql)?;
            let params: Vec<rusqlite::types::Value> = chunk
                .iter()
                .map(|i| rusqlite::types::Value::from(*i))
                .collect();
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
        }
        for neighbors in grouped.values_mut() {
            neighbors.sort();
        }
        Ok(grouped)
    };
    let mut parent: std::collections::HashMap<i64, (i64, i64)> = Default::default();
    let mut visited: std::collections::HashSet<i64> = Default::default();
    let mut frontier: Vec<i64> = vec![from_id];
    visited.insert(from_id);
    let mut found = false;
    let mut d = 0usize;
    while !found && !frontier.is_empty() && d < depth {
        let grouped = level_edges(&frontier)?;
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
        // Walk parents back to the seed: chain node ids + per-hop call sites.
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
        return Ok(PathSearch::Found {
            chain_ids,
            hop_sites,
        });
    }
    if !frontier.is_empty() && d == depth {
        // Cap-truncation probe: an unvisited out-neighbor of any node at the
        // maximum distance means a longer path may exist.
        let grouped = level_edges(&frontier)?;
        for neighbors in grouped.values() {
            if neighbors.iter().any(|(_, _, _, w)| !visited.contains(w)) {
                return Ok(PathSearch::CapTruncated);
            }
        }
    }
    Ok(PathSearch::Exhausted)
}

/// One-call symbol context (Amendment 17): definition line with signature,
/// direct callers, and direct callees — replaces composing callers/callees/
/// symbols for "what does this symbol do and who uses it" questions.
pub fn context(
    conn: &Connection,
    root: &Path,
    name: &str,
    limit: usize,
    explain: bool,
) -> Result<Vec<String>> {
    // The CLI documents max 50 per section; clamp here so MCP callers cannot
    // bypass the contract.
    let limit = clamp_limit(limit)?.min(50);
    let (ids, cands) = target_ids(conn, name)?;
    if ids.is_empty() {
        return Err(AwError::NoResults(format!("symbol '{name}' not in index")));
    }
    if cands.len() > 1 {
        return Err(AwError::Ambiguous {
            name: name.to_string(),
            total: ids.len(),
            candidates: cands,
            role: None,
        });
    }
    let id = ids[0];
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
    let (caller_rows, callers_total) = direct_callers(conn, &[id], limit)?;
    let callers: Vec<(String, i64, String, String, i64, String)> = caller_rows
        .into_iter()
        .map(|row| {
            (
                row.path,
                row.site_line,
                row.qualname,
                row.confidence,
                row.src_id,
                row.dst_name,
            )
        })
        .collect();
    let (callee_rows, callees_total) = direct_callees(conn, &[id], limit)?;
    let callees: Vec<(String, i64, String, String, i64, i64, i64)> = callee_rows
        .into_iter()
        .map(|row| {
            (
                row.path,
                row.line,
                row.qualname,
                row.confidence,
                row.edges,
                row.first_site,
                row.last_site,
            )
        })
        .collect();
    // Staleness over the displayed set (definition + shown callers/callees).
    let mut files: BTreeSet<&String> = BTreeSet::new();
    files.insert(&def_path);
    for (p, _, _, _, _, _) in &callers {
        files.insert(p);
    }
    for (p, _, _, _, _, _, _) in &callees {
        files.insert(p);
    }
    let n_files = files.len();
    let mut changed = 0usize;
    for f in &files {
        if file_changed(conn, root, f) {
            changed += 1;
        }
    }
    let stale = changed > 0;
    // Caller/callee lines carry the identity suffix added in R7.
    let caller_lines: Vec<String> = callers
        .iter()
        .map(|(p, l, qn, conf, src, dst)| {
            let mut line = format!("{p}:{l} {qn}");
            if explain {
                line.push_str(&explain_edge("calls", conf, *l, dst));
                if let Some(receiver) = receiver_hint(conn, *src, *l) {
                    line.push_str(&format!(" receiver={receiver}"));
                }
            }
            line
        })
        .collect();
    let callee_lines: Vec<String> = callees
        .iter()
        .map(|(p, l, qn, conf, edges, first, last)| {
            let tag = if conf == "AMBIGUOUS" {
                " [ambiguous]"
            } else {
                ""
            };
            let mut line = format!("{p}:{l}{tag} {qn}");
            if explain {
                line.push_str(&explain_callee(conf, *edges, *first, *last, qn));
            }
            line
        })
        .collect();
    let total = callers_total + callees_total;
    let shown = caller_lines.len() + callee_lines.len();
    crate::journal::record_result(shown, shown, total, stale, 0);
    let mut out = vec![CONTEXT_CAVEAT.to_string()];
    if explain {
        out.push(EXPLAIN_CAVEAT.to_string());
    }
    if stale {
        out.push(format!(
            "aw: {changed} of {n_files} result files changed since build — run aw build"
        ));
    }
    let sig = if signature.is_empty() {
        String::new()
    } else {
        format!(" {signature}")
    };
    out.push(format!(
        "{def_path}:{def_line}-{def_end} {kind} {qualname}{sig}"
    ));
    out.push(format!(
        "callers (showing {} of {}):",
        callers.len(),
        callers_total
    ));
    out.extend(caller_lines);
    out.push(format!(
        "callees (showing {} of {}):",
        callees.len(),
        callees_total
    ));
    out.extend(callee_lines);
    Ok(out)
}
