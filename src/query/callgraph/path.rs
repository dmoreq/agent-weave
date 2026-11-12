//! Shortest call-path search: level-batched BFS over resolved call edges,
//! chain reconstruction, and rendering.

use super::*;

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
    let depth = depth.clamp(1, crate::query::MAX_DEPTH);
    let (from_id, from_qn) = resolve_endpoint(conn, from, "from")?;
    let (to_id, to_qn) = resolve_endpoint(conn, to, "to")?;
    if from_id == to_id {
        return Err(EgError::NoResults(format!(
            "'{from_qn}' and '{to_qn}' resolve to the same symbol"
        )));
    }
    let (chain_ids, hop_sites) = match path_bfs(conn, from_id, to_id, depth)? {
        PathSearch::Exhausted => {
            return Err(EgError::NoResults(format!(
                "no call path from '{from_qn}' to '{to_qn}' at any depth <= {depth} (resolved edges only)"
            )));
        }
        PathSearch::CapTruncated => {
            return Err(EgError::NoResults(format!(
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
            return Err(EgError::Internal("path hop file missing".to_string()));
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
            return Err(EgError::Internal("path chain qualname missing".to_string()));
        };
        chain_names.push(q.as_str());
    }
    out.push(format!(
        "eg: chain {} ({hops} hops)",
        chain_names.join(" -> ")
    ));
    Ok(LineQuery::new(out, String::new(), hops))
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
            return Err(EgError::Internal(
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
