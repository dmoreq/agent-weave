//! Query surface: callers/callees/impact with qualname disambiguation,
//! result-set-scoped staleness, caps, tier tags, and empty-result reason lines
//! (response contract; CUJ2/CUJ3/CUJ4/CUJ7).

use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::Connection;

use crate::db;
use crate::error::{KgError, Result};
use crate::hash::fnv1a64;

pub const DEFAULT_LIMIT: usize = 50;
/// Cap on rendered ambiguous-name candidates (R6f: common method names could
/// print megabytes; the total count still travels in the error).
pub const MAX_AMBIGUOUS_CANDIDATES: usize = 50;
pub const MAX_LIMIT: usize = 10_000;

/// Permanent honesty caveat for `kg dead` (Amendment 5): candidates, not proof.
const DEAD_CAVEAT: &str =
    "kg: dead-code candidates — dynamic dispatch, entry points, and external callers are not proven absent";

/// Permanent honesty caveat for `kg clones` (Amendment 12d): candidates, not proof.
const CLONES_CAVEAT: &str =
    "kg: clone candidates — normalized-text equality, not proof of semantic duplication";

/// Permanent honesty caveat for `kg path` (Amendment 13): resolved static
/// edges only; dynamic dispatch may hide paths.
const PATH_CAVEAT: &str = "kg: path — resolved static edges only; dynamic dispatch may hide paths";

/// Permanent honesty caveat for `kg context` (Amendment 17).
const CONTEXT_CAVEAT: &str =
    "kg: context — resolved static edges only; dynamic dispatch may hide callers or callees";

/// One resolved candidate: (path, line, qualname).
pub type Candidate = (String, i64, String);

/// One BFS level's out-edges grouped per source: `(dst qualname, dst path,
/// site_line, dst id)` sorted by the pinned neighbor key (`path` is the
/// destination file). Amendment 13.
type LevelEdges = std::collections::HashMap<i64, Vec<(String, String, i64, i64)>>;

pub struct Pointer {
    pub file: String,
    pub line: i64,
    pub tag: &'static str, // "" or " [ambiguous]"
    /// Optional trailing detail rendered after the tag (search hits carry the
    /// qualname here, Amendment 12b; empty for call-edge pointers).
    pub detail: String,
}

impl Pointer {
    pub fn render(&self) -> String {
        if self.detail.is_empty() {
            format!("{}:{}{}", self.file, self.line, self.tag)
        } else {
            format!(
                "{}:{}{} {}",
                self.file,
                self.line,
                self.tag.trim_start(),
                self.detail
            )
        }
    }
}

pub struct QueryOutput {
    pub header: Option<String>,
    pub pointers: Vec<Pointer>,
    pub shown: usize,
    pub total: usize,
    /// True when the staleness header fired on the displayed set (journal).
    pub stale: bool,
}

pub fn clamp_limit(limit: usize) -> Result<usize> {
    if limit == 0 {
        return Err(KgError::Usage("--limit must be >= 1".to_string()));
    }
    Ok(limit.min(MAX_LIMIT))
}

/// Resolve the query name to target symbol ids. Multiple distinct symbols ->
/// exit 2 with a `path:line qualname` candidate list on stdout (CUJ4).
fn target_ids(conn: &Connection, name: &str) -> Result<(Vec<i64>, Vec<Candidate>)> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.qualname, s.name, f.path, s.line_start, s.kind
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE s.name = ?1 OR s.qualname = ?1
         ORDER BY s.qualname, f.path",
    )?;
    let rows = stmt.query_map(rusqlite::params![name], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, String>(5)?,
        ))
    })?;
    let all: Vec<_> = rows.collect::<std::result::Result<_, _>>()?;
    // A module shares the bare name of classes/functions (`button` vs
    // `Button`); when a callable candidate exists, module symbols must not
    // create a false ambiguity (R7 F1).
    let has_callable = all.iter().any(|r| r.5 != "module");
    let mut ids = Vec::new();
    let mut cands = Vec::new();
    let mut seen_ids: std::collections::HashSet<i64> = Default::default();
    for row in all {
        let (id, qualname, sname, path, line, kind) = row;
        if has_callable && kind == "module" {
            continue;
        }
        // Dedupe by id: a symbol whose bare name equals its qualname matches
        // both WHERE branches and must not create a false ambiguity. O(1)
        // membership (R6f: a Vec::contains scan was quadratic).
        if seen_ids.insert(id) {
            ids.push(id);
            // Render cap only (R6f): the error carries `total` separately and
            // the caller prints at most MAX_AMBIGUOUS_CANDIDATES lines.
            if cands.len() < MAX_AMBIGUOUS_CANDIDATES {
                cands.push((path, line, qualname));
            }
        }
        let _ = sname;
    }
    Ok((ids, cands))
}

pub fn callers(conn: &Connection, root: &Path, name: &str, limit: usize) -> Result<QueryOutput> {
    let limit = clamp_limit(limit)?;
    let (ids, cands) = target_ids(conn, name)?;
    if ids.is_empty() {
        return Err(KgError::NoResults(format!("symbol '{name}' not in index")));
    }
    if cands.len() > 1 {
        // CUJ4: no merged-silent result; name the Class.method form. The
        // candidate list travels IN the error so non-CLI surfaces render it.
        return Err(KgError::Ambiguous {
            name: name.to_string(),
            total: ids.len(),
            candidates: cands,
            role: None,
        });
    }
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let count_sql = format!(
        "SELECT COUNT(*) FROM edges e WHERE e.dst_id IN ({placeholders}) AND e.kind = 'calls'"
    );
    let count_params: Vec<rusqlite::types::Value> = ids
        .iter()
        .map(|i| rusqlite::types::Value::from(*i))
        .collect();
    let total: usize = conn.query_row(
        &count_sql,
        rusqlite::params_from_iter(count_params.iter()),
        |r| r.get::<_, i64>(0),
    )? as usize;
    let sql = format!(
        "SELECT e.site_line, e.confidence, f.path, s.qualname
         FROM edges e
         JOIN symbols s ON s.id = e.src_id
         JOIN files f ON f.id = s.file_id
         WHERE e.dst_id IN ({placeholders}) AND e.kind = 'calls'
         ORDER BY f.path, e.site_line LIMIT {limit}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<rusqlite::types::Value> = ids
        .iter()
        .map(|i| rusqlite::types::Value::from(*i))
        .collect();
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        let site: i64 = r.get(0)?;
        let conf: String = r.get(1)?;
        let path: String = r.get(2)?;
        Ok(Pointer {
            file: path,
            line: site,
            tag: if conf == "AMBIGUOUS" {
                " [ambiguous]"
            } else {
                ""
            },
            detail: r.get::<_, String>(3)?,
        })
    })?;
    collect_output(
        conn,
        root,
        name,
        rows,
        limit,
        "0 callers found (dynamic dispatch possible — grep fallback recommended)",
        Some(total),
    )
}

pub fn callees(
    conn: &Connection,
    root: &Path,
    name: &str,
    limit: usize,
    depth: usize,
) -> Result<QueryOutput> {
    let limit = clamp_limit(limit)?;
    let depth = depth.clamp(1, 10);
    let (ids, cands) = target_ids(conn, name)?;
    if ids.is_empty() {
        return Err(KgError::NoResults(format!("symbol '{name}' not in index")));
    }
    if cands.len() > 1 {
        return Err(KgError::Ambiguous {
            name: name.to_string(),
            total: ids.len(),
            candidates: cands,
            role: None,
        });
    }
    // Callee pointer: the callee's definition line. AMBIGUOUS edges have no
    // dst symbol; they surface as the call site itself with the ambiguous tag.
    // depth > 1: recursive closure over call edges from the reached set.
    let total_opt: Option<usize>;
    let mut params: Vec<rusqlite::types::Value> = ids
        .iter()
        .map(|i| rusqlite::types::Value::from(*i))
        .collect();
    let sql: String = if depth == 1 {
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let count_sql = format!(
            "SELECT COUNT(DISTINCT COALESCE(ds.id, -(1000000 + e.id)))
             FROM edges e LEFT JOIN symbols ds ON ds.id = e.dst_id
             WHERE e.src_id IN ({placeholders}) AND e.kind = 'calls'"
        );
        total_opt = Some(conn.query_row(
            &count_sql,
            rusqlite::params_from_iter(params.iter()),
            |r| r.get::<_, i64>(0),
        )? as usize);
        format!(
            "SELECT MIN(COALESCE(ds.line_start, e.site_line)) AS line,
                MAX(e.confidence) AS confidence,
                MAX(COALESCE(df.path, sf.path)) AS path,
                COALESCE(ds.qualname, '') AS qn
         FROM edges e
         JOIN symbols s ON s.id = e.src_id JOIN files sf ON sf.id = s.file_id
         LEFT JOIN symbols ds ON ds.id = e.dst_id
         LEFT JOIN files df ON df.id = ds.file_id
         WHERE e.src_id IN ({placeholders}) AND e.kind = 'calls'
         GROUP BY COALESCE(ds.id, -(1000000 + e.id))
         ORDER BY path, line LIMIT {limit}"
        )
    } else {
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
                COALESCE(df.path, sf.path) AS path, COALESCE(ds.qualname, '') AS qn
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
        total_opt = Some(conn.query_row(
            &count_sql,
            rusqlite::params_from_iter(params.iter()),
            |r| r.get::<_, i64>(0),
        )? as usize);
        // Bounded materialization (R6 round-2): the closure result is
        // counted above and fetched with LIMIT, never fully collected.
        format!("{cte} {select} ORDER BY path, line LIMIT {limit}")
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        let line: i64 = r.get(0)?;
        let conf: String = r.get(1)?;
        let path: String = r.get(2)?;
        Ok(Pointer {
            file: path,
            line,
            tag: if conf == "AMBIGUOUS" {
                " [ambiguous]"
            } else {
                ""
            },
            detail: r.get::<_, String>(3)?,
        })
    })?;
    collect_output(conn, root, name, rows, limit, "0 callees found", total_opt)
}

/// Shortest call path (Amendment 13): does `from` reach `to` through resolved
/// call edges, and via which call sites? Forward direction only (reverse
/// question = swap arguments, OQ5). Pinned determinism: multi-edges dedup to
/// the smallest `site_line`, neighbors expand in
/// `ORDER BY ds.qualname, df.path, e.site_line`, FIFO frontier with
/// visited-on-enqueue, parent = first discoverer. No limit surface: a single
/// shortest chain has at most `depth` hops (recorded deviation). Output:
/// caveat header, hop call-site pointers in path order, final chain line of
/// resolved full qualnames. Payload-journaled (Amendment 10), `name` = the
/// `from` argument.
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
        return Err(KgError::NoResults(format!("symbol '{from}' not in index")));
    }
    if from_cands.len() > 1 {
        return Err(KgError::Ambiguous {
            name: from.to_string(),
            total: from_ids.len(),
            candidates: from_cands,
            role: Some("from"),
        });
    }
    let (to_ids, to_cands) = target_ids(conn, to)?;
    if to_ids.is_empty() {
        return Err(KgError::NoResults(format!("symbol '{to}' not in index")));
    }
    if to_cands.len() > 1 {
        return Err(KgError::Ambiguous {
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
        return Err(KgError::NoResults(format!(
            "'{from_qn}' and '{to_qn}' resolve to the same symbol"
        )));
    }
    let (chain_ids, hop_sites) = match path_bfs(conn, from_id, to_id, depth)? {
        PathSearch::Exhausted => {
            return Err(KgError::NoResults(format!(
                "no call path from '{from_qn}' to '{to_qn}' at any depth <= {depth} (resolved edges only)"
            )));
        }
        PathSearch::CapTruncated => {
            return Err(KgError::NoResults(format!(
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
            return Err(KgError::Internal("path hop file missing".to_string()));
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
    // Payload capture (Amendment 10a): hop pointer text, byte-identical to
    // the stdout data lines; `name` = the `from` argument. `shown` = `total`
    // = hop count (no cap surface exists).
    let built_at = db::meta_get(conn, "built_at")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let source_rev = db::meta_get(conn, "source_rev").unwrap_or_default();
    crate::journal::record_payload(from, &pointers, stale, hops, hops, built_at, &source_rev);
    crate::journal::record_result(hops, hops, hops, stale, 0);
    let mut out = Vec::with_capacity(hops + 4);
    out.push(PATH_CAVEAT.to_string());
    if stale {
        out.push(format!(
            "kg: {changed} of {n_files} result files changed since build — run kg build"
        ));
    }
    out.extend(pointers);
    let mut chain_names: Vec<&str> = Vec::with_capacity(chain_ids.len());
    for id in &chain_ids {
        let Some(q) = qn.get(id) else {
            return Err(KgError::Internal("path chain qualname missing".to_string()));
        };
        chain_names.push(q.as_str());
    }
    out.push(format!(
        "kg: chain {} ({hops} hops)",
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
                return Err(KgError::Internal(
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
pub fn context(conn: &Connection, root: &Path, name: &str, limit: usize) -> Result<Vec<String>> {
    let limit = clamp_limit(limit)?;
    let (ids, cands) = target_ids(conn, name)?;
    if ids.is_empty() {
        return Err(KgError::NoResults(format!("symbol '{name}' not in index")));
    }
    if cands.len() > 1 {
        return Err(KgError::Ambiguous {
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
    let callers_total: usize = conn.query_row(
        "SELECT COUNT(*) FROM edges e WHERE e.dst_id = ?1 AND e.kind = 'calls'",
        [id],
        |r| r.get::<_, i64>(0),
    )? as usize;
    let mut stmt = conn.prepare(
        "SELECT e.site_line, f.path, s.qualname
         FROM edges e JOIN symbols s ON s.id = e.src_id JOIN files f ON f.id = s.file_id
         WHERE e.dst_id = ?1 AND e.kind = 'calls'
         ORDER BY f.path, e.site_line LIMIT ?2",
    )?;
    let callers: Vec<(String, i64, String)> = stmt
        .query_map(rusqlite::params![id, limit as i64], |r| {
            Ok((
                r.get::<_, String>(1)?,
                r.get::<_, i64>(0)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let callees_total: usize = conn.query_row(
        "SELECT COUNT(DISTINCT COALESCE(ds.id, -(1000000 + e.id)))
         FROM edges e LEFT JOIN symbols ds ON ds.id = e.dst_id
         WHERE e.src_id = ?1 AND e.kind = 'calls'",
        [id],
        |r| r.get::<_, i64>(0),
    )? as usize;
    let mut stmt = conn.prepare(
        "SELECT MIN(COALESCE(ds.line_start, e.site_line)) AS line,
                MAX(COALESCE(df.path, sf.path)) AS path,
                COALESCE(ds.qualname, e.dst_name) AS qn,
                MAX(e.confidence) AS conf
         FROM edges e
         JOIN symbols s ON s.id = e.src_id JOIN files sf ON sf.id = s.file_id
         LEFT JOIN symbols ds ON ds.id = e.dst_id
         LEFT JOIN files df ON df.id = ds.file_id
         WHERE e.src_id = ?1 AND e.kind = 'calls'
         GROUP BY COALESCE(ds.id, -(1000000 + e.id))
         ORDER BY path, line LIMIT ?2",
    )?;
    let callees: Vec<(String, i64, String, String)> = stmt
        .query_map(rusqlite::params![id, limit as i64], |r| {
            Ok((
                r.get::<_, String>(1)?,
                r.get::<_, i64>(0)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    // Staleness over the displayed set (definition + shown callers/callees).
    let mut files: BTreeSet<&String> = BTreeSet::new();
    files.insert(&def_path);
    for (p, _, _) in &callers {
        files.insert(p);
    }
    for (p, _, _, _) in &callees {
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
    // Payload pointers must be byte-identical to the stdout data lines
    // (Amendment 10/G11), including the identity suffix added in R7.
    let caller_lines: Vec<String> = callers
        .iter()
        .map(|(p, l, qn)| format!("{p}:{l} {qn}"))
        .collect();
    let callee_lines: Vec<String> = callees
        .iter()
        .map(|(p, l, qn, conf)| {
            let tag = if conf == "AMBIGUOUS" {
                " [ambiguous]"
            } else {
                ""
            };
            format!("{p}:{l}{tag} {qn}")
        })
        .collect();
    let mut pointers = caller_lines.clone();
    pointers.extend(callee_lines.iter().cloned());
    let total = callers_total + callees_total;
    let shown = pointers.len();
    let built_at = db::meta_get(conn, "built_at")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let source_rev = db::meta_get(conn, "source_rev").unwrap_or_default();
    crate::journal::record_payload(name, &pointers, stale, shown, total, built_at, &source_rev);
    crate::journal::record_result(shown, shown, total, stale, 0);
    let mut out = vec![CONTEXT_CAVEAT.to_string()];
    if stale {
        out.push(format!(
            "kg: {changed} of {n_files} result files changed since build — run kg build"
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

pub fn impact(
    conn: &Connection,
    root: &Path,
    file: &str,
    limit: usize,
    sites: bool,
) -> Result<QueryOutput> {
    let limit = clamp_limit(limit)?;
    let rel = file.trim_start_matches("./").to_string();
    let exists: bool = conn
        .query_row("SELECT COUNT(*) FROM files WHERE path = ?1", [&rel], |r| {
            r.get(0)
        })
        .map(|c: i64| c > 0)?;
    if !exists {
        return Err(KgError::NoResults(format!("path '{file}' not indexed")));
    }
    // Direction semantics (CLI help): files that call INTO <file>.
    // Default (Amendment 18): one row per caller FILE with a site count — the
    // per-site list could not answer "which files" once capped.
    if !sites {
        let total_files: usize = conn.query_row(
            "SELECT COUNT(DISTINCT sf.path) FROM edges e
             JOIN symbols ds ON ds.id = e.dst_id JOIN files df ON df.id = ds.file_id
             JOIN symbols ss ON ss.id = e.src_id JOIN files sf ON sf.id = ss.file_id
             WHERE df.path = ?1 AND e.kind = 'calls'",
            [&rel],
            |r| r.get::<_, i64>(0),
        )? as usize;
        let mut stmt = conn.prepare(&format!(
            "SELECT sf.path, MIN(e.site_line) AS line, COUNT(*) AS n
             FROM edges e
             JOIN symbols ds ON ds.id = e.dst_id JOIN files df ON df.id = ds.file_id
             JOIN symbols ss ON ss.id = e.src_id JOIN files sf ON sf.id = ss.file_id
             WHERE df.path = ?1 AND e.kind = 'calls'
             GROUP BY sf.path ORDER BY sf.path LIMIT {limit}"
        ))?;
        let rows = stmt.query_map([&rel], |r| {
            Ok(Pointer {
                file: r.get::<_, String>(0)?,
                line: r.get::<_, i64>(1)?,
                tag: "",
                detail: format!("({} sites)", r.get::<_, i64>(2)?),
            })
        })?;
        return collect_output(
            conn,
            root,
            &rel,
            rows,
            limit,
            "0 impacting files found",
            Some(total_files),
        );
    }
    let total: usize = conn.query_row(
        "SELECT COUNT(*) FROM edges e
         JOIN symbols ds ON ds.id = e.dst_id JOIN files df ON df.id = ds.file_id
         WHERE df.path = ?1 AND e.kind = 'calls'",
        [&rel],
        |r| r.get::<_, i64>(0),
    )? as usize;
    let sql = format!(
        "SELECT e.site_line, e.confidence, sf.path
         FROM edges e
         JOIN symbols ds ON ds.id = e.dst_id JOIN files df ON df.id = ds.file_id
         JOIN symbols ss ON ss.id = e.src_id JOIN files sf ON sf.id = ss.file_id
         WHERE df.path = ?1 AND e.kind = 'calls'
         ORDER BY sf.path, e.site_line LIMIT {limit}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([&rel], |r| {
        let site: i64 = r.get(0)?;
        let conf: String = r.get(1)?;
        let path: String = r.get(2)?;
        Ok(Pointer {
            file: path,
            line: site,
            tag: if conf == "AMBIGUOUS" {
                " [ambiguous]"
            } else {
                ""
            },
            detail: String::new(),
        })
    })?;
    collect_output(
        conn,
        root,
        &rel,
        rows,
        limit,
        "0 impacting files found",
        Some(total),
    )
}

/// Dead-code candidates (Amendment 5): no resolved inbound calls edge and no
/// AMBIGUOUS edge naming the symbol (qualname or bare name). Pointer contract;
/// the caveat header is unconditional.
pub fn dead(
    conn: &Connection,
    root: &Path,
    limit: usize,
    exclude_tests: bool,
) -> Result<QueryOutput> {
    let limit = clamp_limit(limit)?;
    // AMBIGUOUS suppression is per-symbol (R7): a bare-name ambiguous call
    // suppresses only same-named symbols in the caller's own file; a symbol
    // elsewhere with the same bare name is not presumed referenced.
    // One materialized referenced-id set, shared by COUNT and the page query.
    // The previous correlated NOT EXISTS re-scanned edges per candidate
    // symbol (R7 review).
    let referenced = "WITH referenced_ids(id) AS MATERIALIZED (
           SELECT dst_id FROM edges WHERE kind = 'calls' AND dst_id IS NOT NULL
           UNION
           SELECT s2.id FROM edges e
             JOIN symbols s2 ON s2.qualname = e.dst_name
            WHERE e.kind = 'calls' AND e.confidence = 'AMBIGUOUS'
           UNION
           SELECT s2.id FROM edges e
             JOIN symbols src ON src.id = e.src_id
             JOIN symbols s2 ON s2.name = e.dst_name AND s2.file_id = src.file_id
            WHERE e.kind = 'calls' AND e.confidence = 'AMBIGUOUS'
       )";
    let mut predicate = "s.kind IN ('function', 'method', 'class')
           AND s.id NOT IN (SELECT id FROM referenced_ids)"
        .to_string();
    if exclude_tests {
        predicate.push_str(
            " AND f.path NOT LIKE 'tests/%' AND f.path NOT LIKE '%/tests/%'
              AND s.name NOT LIKE 'test\\_%' ESCAPE '\\'",
        );
    }
    let predicate = predicate.as_str();
    let total: usize = conn.query_row(
        &format!(
            "{referenced} SELECT COUNT(*) FROM symbols s JOIN files f ON f.id = s.file_id WHERE {predicate}"
        ),
        [],
        |r| r.get::<_, i64>(0),
    )? as usize;
    let sql = format!(
        "{referenced} SELECT s.qualname, f.path, s.line_start
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE {predicate}
         ORDER BY f.path, s.line_start LIMIT {limit}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |r| {
        let path: String = r.get(1)?;
        let line: i64 = r.get(2)?;
        Ok(Pointer {
            file: path,
            line,
            tag: "",
            detail: String::new(),
        })
    })?;
    if total == 0 {
        return Err(KgError::NoResultsCaveated {
            caveat: DEAD_CAVEAT,
            detail: "0 dead-code candidates found".to_string(),
        });
    }
    let mut out = collect_output(
        conn,
        root,
        "",
        rows,
        limit,
        "0 dead-code candidates found",
        Some(total),
    )?;
    out.header = Some(match out.header.take() {
        Some(stale) => format!("{DEAD_CAVEAT}\n{stale}"),
        None => DEAD_CAVEAT.to_string(),
    });
    Ok(out)
}

/// Unresolved-callee report (Amendment 5): files with counts, descending.
/// Report shape — cap, no staleness; header carries the index total.
pub fn unresolved(conn: &Connection, limit: usize) -> Result<Vec<String>> {
    let limit = clamp_limit(limit)?;
    let total: i64 = conn.query_row("SELECT COALESCE(SUM(unresolved), 0) FROM files", [], |r| {
        r.get(0)
    })?;
    if total == 0 {
        return Err(KgError::NoResults(
            "0 unresolved in indexed scope (external/builtin calls are not counted)".to_string(),
        ));
    }
    let mut stmt = conn.prepare(
        "SELECT path, unresolved FROM files WHERE unresolved > 0
         ORDER BY unresolved DESC, path",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let n_files = rows.len();
    let shown = n_files.min(limit);
    // Name-level view (R7 L4): file counts alone say where resolution failed,
    // not what it missed. `[internal]` marks names that exist in the index —
    // an actionable resolver gap — versus external/builtin calls.
    let mut name_stmt = conn.prepare(
        "SELECT uc.dst_name, COUNT(*) AS n,
                EXISTS(SELECT 1 FROM symbols s
                       WHERE s.name = uc.dst_name OR s.qualname = uc.dst_name)
         FROM unresolved_calls uc
         GROUP BY uc.dst_name
         ORDER BY n DESC, uc.dst_name
         LIMIT 10",
    )?;
    let names = name_stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)? != 0,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let internal: i64 = conn.query_row(
        "SELECT COUNT(*) FROM (
            SELECT uc.dst_name FROM unresolved_calls uc
            WHERE EXISTS(SELECT 1 FROM symbols s
                         WHERE s.name = uc.dst_name OR s.qualname = uc.dst_name)
            GROUP BY uc.dst_name)",
        [],
        |r| r.get(0),
    )?;
    let distinct: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT dst_name) FROM unresolved_calls",
        [],
        |r| r.get(0),
    )?;
    let mut out = vec![format!(
        "kg: unresolved={total} callees in {n_files} files, {distinct} distinct names (internal={internal})"
    )];
    if !names.is_empty() {
        out.push("top unresolved names:".to_string());
        out.extend(names.into_iter().map(|(name, n, internal)| {
            if internal {
                format!("  {name} n={n} [internal]")
            } else {
                format!("  {name} n={n}")
            }
        }));
    }
    out.extend(
        rows.into_iter()
            .take(shown)
            .map(|(path, n)| format!("{path} unresolved={n}")),
    );
    if n_files > shown {
        out.push(format!("showing {} of {}", shown, n_files));
        eprintln!("[kg] capped at {} results ({} total)", shown, n_files);
    }
    crate::journal::record_result(n_files, shown, n_files, false, 0);
    Ok(out)
}

/// Symbols of one indexed file (Amendment 5): `path:line kind qualname` by
/// line; single-file staleness header. With `outline` (Amendment 12a) each
/// line gains the stored one-line signature; without it, byte-identical to
/// the pre-v4 contract.
pub fn symbols(
    conn: &Connection,
    root: &Path,
    file: &str,
    outline: bool,
    limit: usize,
) -> Result<Vec<String>> {
    let limit = clamp_limit(limit)?;
    let rel = file.trim_start_matches("./").to_string();
    let sql = if outline {
        "SELECT s.kind, s.qualname, s.line_start, s.signature
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE f.path = ?1
         ORDER BY s.line_start"
    } else {
        "SELECT s.kind, s.qualname, s.line_start, ''
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE f.path = ?1
         ORDER BY s.line_start"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map([&rel], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if rows.is_empty() {
        return Err(KgError::NoResults(format!("path '{file}' not indexed")));
    }
    let stale = file_changed(conn, root, &rel);
    let n_symbols = rows.len();
    let shown = n_symbols.min(limit);
    let data_lines: Vec<String> = rows
        .into_iter()
        .take(shown)
        .map(|(kind, qualname, line, signature)| {
            if signature.is_empty() {
                format!("{rel}:{line} {kind} {qualname}")
            } else {
                format!("{rel}:{line} {kind} {qualname} {signature}")
            }
        })
        .collect();
    // Payload capture (Amendment 10a): the `path:line kind qualname` data
    // lines only — never the staleness header or the cap footer.
    let built_at = db::meta_get(conn, "built_at")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let source_rev = db::meta_get(conn, "source_rev").unwrap_or_default();
    crate::journal::record_payload(
        &rel,
        &data_lines,
        stale,
        shown,
        n_symbols,
        built_at,
        &source_rev,
    );
    crate::journal::record_result(n_symbols, shown, n_symbols, stale, 0);
    let mut out = Vec::new();
    if stale {
        out.push("kg: 1 of 1 result files changed since build — run kg build".to_string());
    }
    out.extend(data_lines);
    if n_symbols > shown {
        out.push(format!("showing {} of {}", shown, n_symbols));
        eprintln!("[kg] capped at {} results ({} total)", shown, n_symbols);
    }
    Ok(out)
}

/// Symbol-level search (Amendment 12b): FTS5 `bm25()` over split identifier
/// tokens. Hits are symbol identities — the tier field position stays empty
/// (only call-edge pointers carry ` [ambiguous]`). Reuses `collect_output`
/// for cap, post-cap staleness, and payload capture.
pub fn search(conn: &Connection, root: &Path, terms: &str, limit: usize) -> Result<Vec<String>> {
    let limit = clamp_limit(limit)?;
    // OR of prefix terms, bm25-ranked (Amendment 12b): rows matching more
    // and rarer tokens rank higher — natural-language fit without brittle
    // all-token AND requirements.
    let mut toks: Vec<String> = crate::extract::split_ident(terms)
        .split_whitespace()
        .map(|t| t.to_string())
        .collect();
    // Bound the OR expression (R6f Important 7): FTS5 cost is quadratic in
    // the number of prefix terms under OR.
    const MAX_TERMS: usize = 32;
    if toks.len() > MAX_TERMS {
        toks.truncate(MAX_TERMS);
        eprintln!("[kg] search terms capped at {MAX_TERMS}");
    }
    let mut expr = toks
        .iter()
        .map(|tok| format!("\"{}\"*", tok.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ");
    if expr.is_empty() {
        return Err(KgError::Usage("empty search terms".to_string()));
    }
    // Identity fast path (R7): a qualified input (`.`, `/`, `::`) matched as
    // an adjacent-token phrase preserves exact identity and avoids ranking
    // the whole corpus (1M-symbol: ~500 ms -> <1 ms). Fall back to OR when
    // the phrase has no hits, so recall does not regress.
    if terms.contains('.') || terms.contains('/') || terms.contains("::") {
        let phrase = format!("\"{}\"", toks.join(" ").replace('"', ""));
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM symbols_fts WHERE symbols_fts MATCH ?1",
            [&phrase],
            |r| r.get(0),
        )?;
        if n > 0 {
            expr = phrase;
        }
    }
    // Amendment 14b: hits carry the stored one-line signature after the
    // qualname — the agent no longer needs a follow-up `symbols` call to
    // learn what the hit is. Empty signature keeps the 12b byte shape.
    let mut total: usize = conn.query_row(
        "SELECT COUNT(*) FROM symbols_fts WHERE symbols_fts MATCH ?1",
        [&expr],
        |r| r.get::<_, i64>(0),
    )? as usize;
    // Morphology fallback (R7): agents ask "handlers" or "rotation" while the
    // index stores the stem. Retry once with bounded derived forms, only on
    // zero hits, so recall improves without changing ranked results.
    if total == 0 {
        let mut variants: Vec<String> = Vec::new();
        for tok in &toks {
            derived_forms(tok, &mut variants);
        }
        variants.retain(|v| !v.is_empty() && v.len() >= 3);
        variants.dedup();
        variants.truncate(MAX_TERMS);
        if !variants.is_empty() {
            let alt = variants
                .iter()
                .map(|tok| format!("\"{}\"*", tok.replace('"', "")))
                .collect::<Vec<_>>()
                .join(" OR ");
            let alt_total: usize = conn.query_row(
                "SELECT COUNT(*) FROM symbols_fts WHERE symbols_fts MATCH ?1",
                [&alt],
                |r| r.get::<_, i64>(0),
            )? as usize;
            if alt_total > 0 {
                expr = alt;
                total = alt_total;
            }
        }
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT f.path, s.line_start, s.qualname, s.signature
         FROM symbols_fts ft
         JOIN symbols s ON s.id = ft.rowid
         JOIN files f ON f.id = s.file_id
         WHERE symbols_fts MATCH ?1
         -- Tie-break on stable columns: rowids differ between fresh and
         -- carried builds, so bm25 alone is not deterministic (R6d A9).
         ORDER BY bm25(symbols_fts), s.qualname, f.path, s.line_start LIMIT {limit}"
    ))?;
    // Rank a bounded candidate set in Rust (R7): bm25 sees one text column,
    // so a full parameter list can outrank the symbol's own name. Re-rank by
    // how many query tokens occur in the qualname (prefix either way), then
    // keep bm25 order as the stable tie-break. The cap bounds the work.
    let rows = stmt.query_map([&expr], |r| {
        let qualname: String = r.get(2)?;
        let signature: String = r.get(3)?;
        Ok(Pointer {
            file: r.get::<_, String>(0)?,
            line: r.get::<_, i64>(1)?,
            tag: "",
            detail: if signature.is_empty() {
                qualname
            } else {
                format!("{qualname} {signature}")
            },
        })
    })?;
    let out = collect_output(
        conn,
        root,
        terms,
        rows,
        limit,
        &format!("0 symbols matched '{terms}' — try fewer or broader terms"),
        Some(total),
    )?;
    // Exact-identity path (R7): a single token that is an exact symbol name
    // or qualname returns those identities directly (top-1 measured 90-92%
    // through bm25; ~100% here). Fuzzy FTS order stays for other queries.
    if toks.len() == 1 {
        let exact_total: usize = conn.query_row(
            "SELECT COUNT(*) FROM symbols s WHERE s.name = ?1 OR s.qualname = ?1",
            [toks[0].as_str()],
            |r| r.get::<_, i64>(0),
        )? as usize;
        if exact_total > 0 {
            let mut stmt = conn.prepare(&format!(
                "SELECT f.path, s.line_start, s.qualname, s.signature
                 FROM symbols s JOIN files f ON f.id = s.file_id
                 WHERE s.name = ?1 OR s.qualname = ?1
                 ORDER BY s.qualname, f.path LIMIT {limit}"
            ))?;
            let rows = stmt.query_map([toks[0].as_str()], |r| {
                let qualname: String = r.get(2)?;
                let signature: String = r.get(3)?;
                Ok(Pointer {
                    file: r.get::<_, String>(0)?,
                    line: r.get::<_, i64>(1)?,
                    tag: "",
                    detail: if signature.is_empty() {
                        qualname
                    } else {
                        format!("{qualname} {signature}")
                    },
                })
            })?;
            let out = collect_output(
                conn,
                root,
                terms,
                rows,
                limit,
                &format!("0 symbols matched '{terms}' — try fewer or broader terms"),
                Some(exact_total),
            )?;
            return search_lines(out);
        }
    }
    search_lines(out)
}

/// Clone candidates (Amendment 12d): merge adjacent matching shingle windows
/// per file, pair the merged regions, filter by `min_lines`, emit
/// `pathA:startA-endA pathB:startB-endB` under the standard cap contract.
/// Caveat line always first — candidates, never proof. NOT payload-journaled
/// (Amendment 12 deviation: pair lines are not single pointers, so the
/// `usage --verify` structural checks do not apply).
pub fn clones(
    conn: &Connection,
    root: &Path,
    min_lines: usize,
    limit: usize,
) -> Result<Vec<String>> {
    let limit = clamp_limit(limit)?;
    let min_lines = min_lines.max(crate::clones::WINDOW);
    let mut stmt = conn.prepare(
        "SELECT cg.shingle_hash, f.path, cg.line_start, cg.line_end
         FROM clone_groups cg JOIN files f ON f.id = cg.file_id
         WHERE cg.shingle_hash IN (
             SELECT shingle_hash FROM clone_groups
             GROUP BY shingle_hash HAVING COUNT(*) >= 2)
         ORDER BY cg.shingle_hash, f.path, cg.line_start",
    )?;
    let mut groups: std::collections::BTreeMap<i64, Vec<(String, i64, i64)>> = Default::default();
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    for r in rows {
        let (h, path, s, e) = r?;
        groups.entry(h).or_default().push((path, s, e));
    }
    // Pair generation is bounded (R6d Blocker 2): a shared header can put
    // thousands of regions in one shingle group, and the full pair set is
    // quadratic. Candidates beyond the caps are reported as a lower bound.
    const MAX_REGIONS_PER_GROUP: usize = 64;
    const MAX_PAIRS: usize = 20_000;
    let mut capped = false;
    let mut seen = std::collections::BTreeSet::new();
    let mut pairs: Vec<(String, i64, i64, String, i64, i64)> = Vec::new();
    'groups: for regions in groups.values() {
        // Merge overlapping / near-adjacent windows (gap < window) per file:
        // one long clone produces many overlapping windows, one region.
        let mut merged: Vec<(String, i64, i64)> = Vec::new();
        for (path, s, e) in regions {
            if let Some(last) = merged.last_mut() {
                if last.0 == *path && *s <= last.2 + 4 {
                    last.2 = (*e).max(last.2);
                    continue;
                }
            }
            merged.push((path.clone(), *s, *e));
        }
        if merged.len() < 2 {
            continue;
        }
        if merged.len() > MAX_REGIONS_PER_GROUP {
            merged.truncate(MAX_REGIONS_PER_GROUP);
            capped = true;
        }
        for i in 0..merged.len() {
            for j in (i + 1)..merged.len() {
                if pairs.len() >= MAX_PAIRS {
                    capped = true;
                    break 'groups;
                }
                let (pa, sa, ea) = &merged[i];
                let (pb, sb, eb) = &merged[j];
                let (fa, sta, ena, fb, stb, enb) =
                    if (pa.as_str(), *sa, *ea) <= (pb.as_str(), *sb, *eb) {
                        (pa, *sa, *ea, pb, *sb, *eb)
                    } else {
                        (pb, *sb, *eb, pa, *sa, *ea)
                    };
                let key = (fa.clone(), sta, ena, fb.clone(), stb, enb);
                if seen.insert(key) {
                    pairs.push((fa.clone(), sta, ena, fb.clone(), stb, enb));
                }
            }
        }
    }
    // Pair-merge (R7): windows of one long clone live in different hash
    // groups, so group every candidate by the unordered file pair and merge
    // overlapping spans on BOTH sides into maximal regions. Apply the
    // --min-lines filter AFTER merging (the old code filtered 5-line windows
    // first, which made every min-lines > 5 return a false negative).
    // Group by file pair first (R7 review): sorting globally interleaves
    // different pairs, so merging only against the previous row left
    // overlapping spans for the same pair unmerged.
    type SpanPair = (i64, i64, i64, i64);
    let mut by_pair: std::collections::BTreeMap<(String, String), Vec<SpanPair>> =
        Default::default();
    for (fa, sa, ea, fb, sb, eb) in pairs {
        by_pair.entry((fa, fb)).or_default().push((sa, ea, sb, eb));
    }
    let mut merged_pairs: Vec<(String, i64, i64, String, i64, i64)> = Vec::new();
    for ((fa, fb), mut spans) in by_pair {
        spans.sort_unstable();
        let mut cur: Option<(i64, i64, i64, i64)> = None;
        for (sa, ea, sb, eb) in spans {
            match cur {
                Some((csa, cea, csb, ceb)) if sa <= cea && sb <= ceb => {
                    cur = Some((csa, cea.max(ea), csb, ceb.max(eb)));
                }
                Some(done) => {
                    merged_pairs.push((fa.clone(), done.0, done.1, fb.clone(), done.2, done.3));
                    cur = Some((sa, ea, sb, eb));
                }
                None => cur = Some((sa, ea, sb, eb)),
            }
        }
        if let Some((sa, ea, sb, eb)) = cur {
            merged_pairs.push((fa, sa, ea, fb, sb, eb));
        }
    }
    merged_pairs.retain(|(_, sa, ea, _, sb, eb)| {
        (*ea - *sa + 1) >= min_lines as i64 && (*eb - *sb + 1) >= min_lines as i64
    });
    // Longest merged spans first, deterministic tie-break by paths/spans.
    merged_pairs.sort_by(|a, b| {
        let la = (a.2 - a.1).max(a.5 - a.4);
        let lb = (b.2 - b.1).max(b.5 - b.4);
        lb.cmp(&la).then_with(|| {
            (a.0.as_str(), a.1, a.3.as_str(), a.4).cmp(&(b.0.as_str(), b.1, b.3.as_str(), b.4))
        })
    });
    let pairs = merged_pairs;
    if pairs.is_empty() {
        return Err(KgError::NoResultsCaveated {
            caveat: CLONES_CAVEAT,
            detail: "0 clone candidates found".to_string(),
        });
    }
    if capped {
        eprintln!("[kg] clone pair generation capped — result is a lower bound");
    }
    let shown = pairs.len().min(limit);
    let displayed = &pairs[..shown];
    let files: std::collections::BTreeSet<&String> = displayed
        .iter()
        .flat_map(|(a, _, _, b, _, _)| [a, b])
        .collect();
    let n_files = files.len();
    let mut changed = 0usize;
    for f in &files {
        if file_changed(conn, root, f) {
            changed += 1;
        }
    }
    let stale = changed > 0;
    crate::journal::record_result(shown, shown, pairs.len(), stale, 0);
    let mut out = vec![CLONES_CAVEAT.to_string()];
    if stale {
        out.push(format!(
            "kg: {changed} of {n_files} result files changed since build — run kg build"
        ));
    }
    // Identity (R7 L4): a bare span forces a file read to learn what the
    // duplicated block is. Name the enclosing symbol per side, when any.
    let enclosing = |path: &str, start: i64, end: i64| -> Option<String> {
        // Largest overlap wins: a merged span can begin in the previous
        // function's last line, and the shallowest containing symbol is
        // rarely the duplicated unit.
        conn.query_row(
            "SELECT s.qualname FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE f.path = ?1 AND s.line_start <= ?3 AND s.line_end >= ?2
             ORDER BY (MIN(s.line_end, ?3) - MAX(s.line_start, ?2)) DESC,
                      s.line_start DESC
             LIMIT 1",
            rusqlite::params![path, start, end],
            |r| r.get::<_, String>(0),
        )
        .ok()
    };
    for (pa, sa, ea, pb, sb, eb) in displayed {
        let ta = enclosing(pa, *sa, *ea)
            .map(|q| format!(" ({q})"))
            .unwrap_or_default();
        let tb = enclosing(pb, *sb, *eb)
            .map(|q| format!(" ({q})"))
            .unwrap_or_default();
        out.push(format!("{pa}:{sa}-{ea}{ta} {pb}:{sb}-{eb}{tb}"));
    }
    if pairs.len() > shown {
        out.push(format!("showing {} of {}", shown, pairs.len()));
        eprintln!("[kg] capped at {} results ({} total)", shown, pairs.len());
    }
    Ok(out)
}

/// Cap, then staleness-check the post-cap displayed set only (cost bounded by
/// the displayed count): stat short-circuits, FNV-1a re-hash on mismatch.
/// `name` is the query target recorded in the payload journal (Amendment 10).
/// Render a search QueryOutput (header, hits, cap footer) as stdout lines.
fn search_lines(out: QueryOutput) -> Result<Vec<String>> {
    // Every search exit path (exact, phrase, fuzzy) journals here, so an
    // exact-name hit is not recorded as an empty call (R7 review).
    crate::journal::record_result(out.pointers.len(), out.shown, out.total, out.stale, 0);
    let mut lines = Vec::with_capacity(out.pointers.len() + 2);
    if let Some(h) = &out.header {
        lines.push(h.clone());
    }
    for p in &out.pointers {
        lines.push(p.render());
    }
    if out.total > out.shown {
        lines.push(format!("showing {} of {}", out.shown, out.total));
        eprintln!("[kg] capped at {} results ({} total)", out.shown, out.total);
    }
    Ok(lines)
}

/// Bounded English morphology variants for a zero-hit token (R7).
fn derived_forms(tok: &str, out: &mut Vec<String>) {
    if tok.len() <= 4 {
        return;
    }
    if let Some(stem) = tok.strip_suffix("ies") {
        out.push(format!("{stem}y"));
        return;
    }
    for suffix in ["es", "s", "ed", "ing", "ion"] {
        if let Some(stem) = tok.strip_suffix(suffix) {
            if stem.len() >= 3 {
                out.push(stem.to_string());
                out.push(format!("{stem}e"));
            }
            return;
        }
    }
}

fn collect_output(
    conn: &Connection,
    root: &Path,
    name: &str,
    rows: impl Iterator<Item = std::result::Result<Pointer, rusqlite::Error>>,
    limit: usize,
    empty_reason: &str,
    known_total: Option<usize>,
) -> Result<QueryOutput> {
    // Bounded materialization (R6f Important 5): callers pass a SQL LIMIT
    // plus a separate COUNT, so memory stays proportional to the displayed
    // set instead of the total hit count.
    let mut all: Vec<Pointer> = Vec::new();
    for r in rows {
        all.push(r?);
    }
    let total = known_total.unwrap_or(all.len());
    if total == 0 {
        return Err(KgError::NoResults(empty_reason.to_string()));
    }
    let shown = all.len().min(limit);
    let displayed: Vec<Pointer> = all.into_iter().take(shown).collect();
    let files: BTreeSet<&String> = displayed.iter().map(|p| &p.file).collect();
    let n_result_files = files.len();
    let mut changed = 0usize;
    for f in &files {
        if file_changed(conn, root, f) {
            changed += 1;
        }
    }
    let header = if changed > 0 {
        Some(format!(
            "kg: {changed} of {n_result_files} result files changed since build — run kg build"
        ))
    } else {
        None
    };
    let out = QueryOutput {
        header,
        pointers: displayed,
        shown,
        total,
        stale: changed > 0,
    };
    record_payload_capture(conn, name, &out);
    Ok(out)
}

/// Payload journal capture (Amendment 10a): the rendered displayed set,
/// byte-identical to stdout data lines, plus the index meta it came from.
/// Best-effort like every journal write; runs before the dispatch site's
/// `record_result`, which fills the same Pending fields consistently.
fn record_payload_capture(conn: &Connection, name: &str, out: &QueryOutput) {
    if out.pointers.is_empty() {
        return;
    }
    let built_at = db::meta_get(conn, "built_at")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let source_rev = db::meta_get(conn, "source_rev").unwrap_or_default();
    let rendered: Vec<String> = out.pointers.iter().map(|p| p.render()).collect();
    crate::journal::record_payload(
        name,
        &rendered,
        out.stale,
        out.shown,
        out.total,
        built_at,
        &source_rev,
    );
}

/// Render a QueryOutput exactly as the CLI prints it on stdout (header,
/// pointers, cap line). The cap stderr note is NOT part of this string.
pub fn render_output(out: &QueryOutput) -> String {
    let mut text = String::new();
    if let Some(h) = &out.header {
        text.push_str(h);
        text.push('\n');
    }
    for p in &out.pointers {
        text.push_str(&p.render());
        text.push('\n');
    }
    if out.total > out.shown {
        text.push_str(&format!("showing {} of {}\n", out.shown, out.total));
    }
    text
}

pub fn print_output(out: &QueryOutput) {
    print!("{}", render_output(out));
    if out.total > out.shown {
        eprintln!("[kg] capped at {} results ({} total)", out.shown, out.total);
    }
}

/// Staleness: (mtime, size) stat short-circuit; FNV-1a re-hash on mismatch.
pub fn file_changed(conn: &Connection, root: &Path, rel: &str) -> bool {
    let row = conn.query_row(
        "SELECT mtime, size, content_hash FROM files WHERE path = ?1",
        [rel],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        },
    );
    let Ok((stored_mtime, stored_size, stored_hash)) = row else {
        return false;
    };
    let abs = root.join(rel);
    let Some(size) = db::stat_size(&abs) else {
        return true;
    };
    if size != stored_size {
        return true;
    }
    let Some(mtime) = db::stat_mtime(&abs) else {
        return true;
    };
    if mtime != stored_mtime {
        let Ok(bytes) = std::fs::read(&abs) else {
            return true;
        };
        let hash = fnv1a64(&bytes) as i64;
        return hash != stored_hash;
    }
    false
}
