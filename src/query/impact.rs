//! Reverse-dependency impact queries and their traversal bookkeeping.

use super::*;

fn parse_edge_kinds(spec: &str) -> Result<Vec<&'static str>> {
    let mut out: Vec<&'static str> = Vec::new();
    let mut saw_all = false;
    for raw in spec.split(',') {
        let part = raw.trim();
        match part {
            "calls" => out.push("calls"),
            "imports" => out.push("imports"),
            "inherits" => out.push("inherits"),
            "all" => {
                saw_all = true;
                out.extend(["calls", "imports", "inherits"]);
            }
            "" => {
                return Err(AwError::Usage(
                    "--edges requires calls,imports,inherits,all".to_string(),
                ))
            }
            other => {
                return Err(AwError::Usage(format!(
                    "unknown --edges value '{other}' (calls,imports,inherits,all)"
                )))
            }
        }
    }
    if saw_all && spec.split(',').any(|p| p.trim() != "all") {
        return Err(AwError::Usage(
            "--edges all cannot be combined with explicit kinds".to_string(),
        ));
    }
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        return Err(AwError::Usage(
            "--edges requires at least one kind".to_string(),
        ));
    }
    Ok(out)
}

fn impact_detail(
    context: bool,
    qualname: &str,
    src_kind: &str,
    explain: bool,
    kind: &str,
    depth: usize,
) -> String {
    let mut parts = Vec::new();
    if context && !qualname.is_empty() && matches!(src_kind, "function" | "method") {
        parts.push(format!("[in {qualname}]"));
    }
    if explain {
        parts.push(format!("[{kind} depth={depth}]"));
    }
    parts.join(" ")
}

fn impact_empty_detail(kinds: &[&str]) -> String {
    let has_imports = kinds.contains(&"imports");
    let has_inherits = kinds.contains(&"inherits");
    match (has_imports, has_inherits) {
        (true, true) => "0 impacting files found (unresolved imports/bases are omitted)",
        (true, false) => "0 impacting files found (unresolved imports are omitted)",
        (false, true) => "0 impacting files found (unresolved bases are omitted)",
        (false, false) => "0 impacting files found",
    }
    .to_string()
}

#[allow(clippy::too_many_arguments)] // CLI surface; a params struct adds no value here.
pub fn impact(
    conn: &Connection,
    root: &Path,
    file: &str,
    limit: usize,
    sites: bool,
    depth: usize,
    edges: &str,
    context: bool,
    explain: bool,
) -> Result<QueryOutput> {
    let limit = clamp_limit(limit)?;
    let depth = depth.clamp(1, 10);
    let kinds = parse_edge_kinds(edges)?;
    let kind_sql = edge_kind_sql(&kinds);
    let rel = file.trim_start_matches("./").to_string();
    let exists: bool = conn
        .query_row("SELECT COUNT(*) FROM files WHERE path = ?1", [&rel], |r| {
            r.get(0)
        })
        .map(|c: i64| c > 0)?;
    if !exists {
        return Err(AwError::NoResults(format!("path '{file}' not indexed")));
    }
    let extended = depth > 1 || context || kinds.iter().any(|k| *k != "calls");
    let caveat = if extended {
        if kinds.iter().any(|k| *k != "calls") {
            IMPACT_EDGES_CAVEAT
        } else {
            IMPACT_CALLS_CAVEAT
        }
    } else {
        ""
    };
    // Grouped mode cannot carry a meaningful per-edge tag; --context and
    // --explain switch to per-site rows at depth 1.
    let sites = sites || (depth == 1 && (context || explain));
    if depth > 1 && sites {
        return Err(AwError::Usage(
            "--sites is only valid with --depth 1".to_string(),
        ));
    }

    let ctx = ImpactCtx {
        file,
        rel: &rel,
        limit,
        depth,
        kind_sql: &kind_sql,
        caveat,
        kinds: &kinds,
        context,
        explain,
        extended,
    };
    if depth == 1 {
        if !sites {
            return impact_direct_grouped(conn, root, &ctx);
        }
        return impact_direct_sites(conn, root, &ctx);
    }
    impact_deep(conn, root, &ctx)
}

/// Shared, immutable arguments for the three impact phases. The phases stay
/// contiguous; this is not a generic query framework.
struct ImpactCtx<'a> {
    file: &'a str,
    rel: &'a str,
    limit: usize,
    depth: usize,
    kind_sql: &'a str,
    caveat: &'static str,
    kinds: &'a [&'static str],
    context: bool,
    explain: bool,
    extended: bool,
}

fn impact_with_caveat(mut out: QueryOutput, caveat: &str, extended: bool) -> QueryOutput {
    if extended {
        out.header = Some(match out.header.take() {
            Some(stale) => format!("{caveat}\n{stale}"),
            None => caveat.to_string(),
        });
    }
    out
}

/// Depth-1 grouped rows: one row per impacting file, with its site count.
fn impact_direct_grouped(
    conn: &Connection,
    root: &Path,
    ctx: &ImpactCtx<'_>,
) -> Result<QueryOutput> {
    let total_files: usize = conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT sf.path) FROM edges e
             JOIN symbols ds ON ds.id = e.dst_id JOIN files df ON df.id = ds.file_id
             JOIN symbols ss ON ss.id = e.src_id JOIN files sf ON sf.id = ss.file_id
             WHERE df.path = ?1 AND e.kind IN ({})",
            ctx.kind_sql
        ),
        [ctx.rel],
        |r| r.get::<_, i64>(0),
    )? as usize;
    if total_files == 0 {
        if ctx.extended {
            return Err(AwError::NoResultsCaveated {
                caveat: ctx.caveat,
                detail: impact_empty_detail(ctx.kinds),
            });
        }
        return Err(AwError::NoResults("0 impacting files found".to_string()));
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT sf.path, MIN(e.site_line) AS line,
                COUNT(DISTINCT {}) AS n
         FROM edges e
         JOIN symbols ds ON ds.id = e.dst_id JOIN files df ON df.id = ds.file_id
         JOIN symbols ss ON ss.id = e.src_id JOIN files sf ON sf.id = ss.file_id
         WHERE df.path = ?1 AND e.kind IN ({})
         GROUP BY sf.path ORDER BY sf.path LIMIT {}",
        IMPACT_SITE_KEY, ctx.kind_sql, ctx.limit
    ))?;
    let rows = stmt.query_map([ctx.rel], |r| {
        Ok(Pointer {
            file: r.get::<_, String>(0)?,
            line: r.get::<_, i64>(1)?,
            tag: "",
            detail: format!("({} sites)", r.get::<_, i64>(2)?),
        })
    })?;
    let out = collect_output(
        conn,
        root,
        ctx.rel,
        rows,
        ctx.limit,
        "0 impacting files found",
        Some(total_files),
    )?;
    Ok(impact_with_caveat(out, ctx.caveat, ctx.extended))
}

/// Depth-1 per-site rows: one row per call site, with optional context/explain.
fn impact_direct_sites(conn: &Connection, root: &Path, ctx: &ImpactCtx<'_>) -> Result<QueryOutput> {
    let total: usize = conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT {}) FROM edges e
             JOIN symbols ds ON ds.id = e.dst_id JOIN files df ON df.id = ds.file_id
             JOIN symbols ss ON ss.id = e.src_id
             WHERE df.path = ?1 AND e.kind IN ({})",
            IMPACT_SITE_KEY, ctx.kind_sql
        ),
        [ctx.rel],
        |r| r.get::<_, i64>(0),
    )? as usize;
    if total == 0 {
        if ctx.extended {
            return Err(AwError::NoResultsCaveated {
                caveat: ctx.caveat,
                detail: impact_empty_detail(ctx.kinds),
            });
        }
        return Err(AwError::NoResults("0 impacting files found".to_string()));
    }
    let sql = format!(
        "SELECT MIN(e.site_line), MIN(e.confidence), MIN(sf.path),
                MIN(ss.qualname), MIN(ss.kind), MIN(e.kind)
         FROM edges e
         JOIN symbols ds ON ds.id = e.dst_id JOIN files df ON df.id = ds.file_id
         JOIN symbols ss ON ss.id = e.src_id JOIN files sf ON sf.id = ss.file_id
         WHERE df.path = ?1 AND e.kind IN ({})
         GROUP BY {}
         ORDER BY 3, 1, 4 LIMIT {}",
        ctx.kind_sql, IMPACT_SITE_KEY, ctx.limit
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([ctx.rel], |r| {
        let site: i64 = r.get(0)?;
        let conf: String = r.get(1)?;
        let path: String = r.get(2)?;
        let qn: String = r.get(3)?;
        let src_kind: String = r.get(4)?;
        let kind: String = r.get(5)?;
        Ok(Pointer {
            file: path,
            line: site,
            tag: if conf == "AMBIGUOUS" {
                " [ambiguous]"
            } else {
                ""
            },
            detail: impact_detail(ctx.context, &qn, &src_kind, ctx.explain, &kind, 1),
        })
    })?;
    let out = collect_output(
        conn,
        root,
        ctx.rel,
        rows,
        ctx.limit,
        "0 impacting files found",
        Some(total),
    )?;
    Ok(impact_with_caveat(out, ctx.caveat, ctx.extended))
}

/// Depth > 1: reverse BFS. R1 is the exact depth-1 edge set; deeper levels
/// expand from R1's source symbols, so deeper search is monotone.
fn impact_deep(conn: &Connection, root: &Path, ctx: &ImpactCtx<'_>) -> Result<QueryOutput> {
    if total_impact_targets(conn, ctx.rel)? == 0 {
        return Err(AwError::NoResults(format!(
            "path '{}' not indexed",
            ctx.file
        )));
    }
    let mut records = ImpactTop::new(ctx.limit);
    let detail = ctx.context || ctx.explain;
    let mut frontier = Vec::new();
    let mut visited = HashSet::new();
    if detail {
        (frontier, visited) = direct_impact_level(conn, ctx.rel, ctx.kind_sql, &mut records, 1)?;
    } else {
        direct_impact_records(conn, ctx.rel, ctx.kind_sql, &mut records)?;
    }
    let total_files = impact_source_files(conn, ctx.kinds, ctx.kind_sql)?;
    let mut truncated = false;
    for d in 2..=ctx.depth {
        if records.len() >= total_files {
            break;
        }
        if frontier.is_empty() && visited.is_empty() {
            (frontier, visited) = direct_impact_frontier(conn, ctx.rel, ctx.kind_sql)?;
        }
        let next = level_impact(conn, &frontier, &mut visited, ctx.kind_sql, &mut records, d)?;
        if next.is_empty() {
            break;
        }
        if d == ctx.depth {
            truncated = true;
        }
        frontier = next;
    }
    let (total, paths) = records.finish();
    if total == 0 {
        return Err(AwError::NoResultsCaveated {
            caveat: ctx.caveat,
            detail: impact_empty_detail(ctx.kinds),
        });
    }
    let pointers: Vec<Pointer> = paths
        .into_iter()
        .map(|(path, d, line, kind, qn, src_kind)| Pointer {
            file: path,
            line,
            tag: "",
            detail: impact_detail(ctx.context, &qn, &src_kind, ctx.explain, &kind, d),
        })
        .collect();
    let mut out = collect_output(
        conn,
        root,
        ctx.rel,
        pointers.into_iter().map(Ok),
        ctx.limit,
        "0 impacting files found",
        Some(total),
    )?;
    let mut header = ctx.caveat.to_string();
    if let Some(stale) = out.header.take() {
        header.push('\n');
        header.push_str(&stale);
    }
    if truncated {
        header.push_str(&format!(
            "\naw: depth cap {} reached; deeper dependents may exist",
            ctx.depth
        ));
    }
    out.header = Some(header);
    Ok(out)
}

fn total_impact_targets(conn: &Connection, rel: &str) -> Result<usize> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM symbols s JOIN files f ON f.id = s.file_id WHERE f.path = ?1",
        [rel],
        |r| r.get::<_, i64>(0),
    )? as usize)
}

/// Exact upper bound on files that can be an edge source for the selected
/// kinds. New indexes store per-kind and all-kind counters; older indexes and
/// mixed subsets fall back to an edge scan. The calls-only case must include
/// module-level call sites: a module symbol can be a `calls` edge source.
fn impact_source_files(conn: &Connection, kinds: &[&str], kind_sql: &str) -> Result<usize> {
    let key = match kinds {
        [kind] => match *kind {
            "calls" => Some("edge_calls_src_files"),
            "imports" => Some("edge_imports_src_files"),
            "inherits" => Some("edge_inherits_src_files"),
            _ => None,
        },
        _ if kinds.len() == 3
            && kinds
                .iter()
                .all(|k| matches!(*k, "calls" | "imports" | "inherits")) =>
        {
            Some("edge_all_src_files")
        }
        _ => None,
    };
    if let Some(key) = key {
        if let Some(value) = db::meta_get(conn, key) {
            if let Ok(n) = value.parse::<usize>() {
                return Ok(n);
            }
        }
    }
    Ok(conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT ss.file_path) FROM edges e
             JOIN symbols ss ON ss.id = e.src_id
             WHERE e.kind IN ({kind_sql})"
        ),
        [],
        |r| r.get::<_, i64>(0),
    )? as usize)
}

fn direct_impact_records(
    conn: &Connection,
    rel: &str,
    kind_sql: &str,
    records: &mut ImpactTop,
) -> Result<()> {
    let sql = format!(
        "SELECT MIN(e.site_line), MIN(e.kind), ss.file_path, MIN(ss.qualname), MIN(ss.kind)
         FROM edges e
         JOIN symbols ds ON ds.id = e.dst_id JOIN files df ON df.id = ds.file_id
         JOIN symbols ss ON ss.id = e.src_id
         WHERE df.path = ?1 AND e.kind IN ({kind_sql})
         GROUP BY ss.file_path"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([rel], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
        ))
    })?;
    for row in rows {
        let (line, kind, path, qn, src_kind) = row?;
        records.insert(path, 1, line, kind, qn, src_kind);
    }
    Ok(())
}

fn direct_impact_frontier(
    conn: &Connection,
    rel: &str,
    kind_sql: &str,
) -> Result<(Vec<i64>, HashSet<i64>)> {
    let seed_sql = "SELECT s.id FROM symbols s JOIN files f ON f.id = s.file_id WHERE f.path = ?1";
    let mut seed_stmt = conn.prepare(seed_sql)?;
    let mut visited: HashSet<i64> = seed_stmt
        .query_map([rel], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    let sql = format!(
        "SELECT DISTINCT e.src_id FROM edges e
         JOIN symbols ds ON ds.id = e.dst_id JOIN files df ON df.id = ds.file_id
         WHERE df.path = ?1 AND e.kind IN ({kind_sql})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut frontier = Vec::new();
    for id in stmt.query_map([rel], |r| r.get::<_, i64>(0))? {
        let id = id?;
        if visited.insert(id) {
            frontier.push(id);
        }
    }
    Ok((frontier, visited))
}

fn direct_impact_level(
    conn: &Connection,
    rel: &str,
    kind_sql: &str,
    records: &mut ImpactTop,
    depth: usize,
) -> Result<(Vec<i64>, HashSet<i64>)> {
    let sql = format!(
        "SELECT e.src_id, e.site_line, e.kind, ss.file_path, ss.qualname, ss.kind
         FROM edges e
         JOIN symbols ds ON ds.id = e.dst_id JOIN files df ON df.id = ds.file_id
         JOIN symbols ss ON ss.id = e.src_id
         WHERE df.path = ?1 AND e.kind IN ({kind_sql})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([rel], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
        ))
    })?;
    let mut frontier = Vec::new();
    let mut visited = HashSet::new();
    let target_ids: Vec<i64> = {
        let mut stmt = conn.prepare(
            "SELECT s.id FROM symbols s JOIN files f ON f.id = s.file_id WHERE f.path = ?1",
        )?;
        let rows = stmt.query_map([rel], |r| r.get(0))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    for id in target_ids {
        visited.insert(id);
    }
    for r in rows {
        let (src, line, kind, path, qn, src_kind) = r?;
        records.insert(path, depth, line, kind, qn, src_kind);
        if visited.insert(src) {
            frontier.push(src);
        }
    }
    Ok((frontier, visited))
}

fn level_impact(
    conn: &Connection,
    frontier: &[i64],
    visited: &mut HashSet<i64>,
    kind_sql: &str,
    records: &mut ImpactTop,
    depth: usize,
) -> Result<Vec<i64>> {
    if frontier.is_empty() {
        return Ok(Vec::new());
    }
    let mut next = Vec::new();
    for chunk in frontier.chunks(500) {
        let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT e.src_id, e.site_line, e.kind, ss.file_path, ss.qualname, ss.kind
             FROM edges e
             JOIN symbols ss ON ss.id = e.src_id
             WHERE e.dst_id IN ({placeholders}) AND e.kind IN ({kind_sql})"
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
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        for r in rows {
            let (src, line, kind, path, qn, src_kind) = r?;
            records.insert(path, depth, line, kind, qn, src_kind);
            if visited.insert(src) {
                next.push(src);
            }
        }
    }
    Ok(next)
}

type ImpactRecord = (String, usize, i64, String, String, String);

/// Exact distinct-path map with the earliest (depth, line) record per path.
/// SQL returns unordered rows; BFS depth increases monotonically, so replace
/// only when a same-depth row has a smaller site line. Output selection is a
/// bounded heap, avoiding a full ORDER BY over every inbound edge.
struct ImpactTop {
    limit: usize,
    seen: std::collections::HashMap<String, ImpactRecord>,
}

impl ImpactTop {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            seen: std::collections::HashMap::new(),
        }
    }

    fn len(&self) -> usize {
        self.seen.len()
    }

    fn insert(
        &mut self,
        path: String,
        depth: usize,
        line: i64,
        kind: String,
        qualname: String,
        src_kind: String,
    ) {
        // Depth and site line are the primary key. On an exact tie, compare
        // stable semantic fields so the representative does not depend on
        // SQL row order or rowid/file-id assignment.
        let replace = match self.seen.get(&path) {
            None => true,
            Some(old) => {
                (depth, line, &kind, &qualname, &src_kind) < (old.1, old.2, &old.3, &old.4, &old.5)
            }
        };
        if replace {
            self.seen
                .insert(path.clone(), (path, depth, line, kind, qualname, src_kind));
        }
    }

    fn finish(self) -> (usize, Vec<ImpactRecord>) {
        let total = self.seen.len();
        let mut heap: BinaryHeap<ImpactRecord> = BinaryHeap::new();
        for record in self.seen.into_values() {
            if heap.len() < self.limit {
                heap.push(record);
            } else if heap.peek().is_some_and(|max| record.0 < max.0) {
                heap.pop();
                heap.push(record);
            }
        }
        let mut records = heap.into_vec();
        records.sort_by(|a, b| a.0.cmp(&b.0));
        (total, records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impact_top_ties_are_deterministic() {
        let mut first = ImpactTop::new(10);
        first.insert(
            "p".into(),
            1,
            5,
            "imports".into(),
            "q".into(),
            "function".into(),
        );
        first.insert(
            "p".into(),
            1,
            5,
            "calls".into(),
            "q".into(),
            "function".into(),
        );
        let (_, records) = first.finish();
        assert_eq!(records[0].3, "calls");

        let mut second = ImpactTop::new(10);
        second.insert(
            "p".into(),
            1,
            5,
            "calls".into(),
            "q".into(),
            "function".into(),
        );
        second.insert(
            "p".into(),
            1,
            5,
            "imports".into(),
            "q".into(),
            "function".into(),
        );
        let (_, records) = second.finish();
        assert_eq!(records[0].3, "calls");
    }
}
