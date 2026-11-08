//! Depth > 1 reverse BFS traversal and the bounded per-file record store.

use super::*;
use crate::db::EDGE_SRC_JOIN;

use std::collections::BinaryHeap;

/// BFS traversal bookkeeping shared by the impact phases: the settled
/// per-file records plus the current frontier and visited symbol set. One
/// struct replaces the parameter clump the BFS helpers passed around.
struct TraversalState {
    records: ImpactTop,
    frontier: Vec<i64>,
    visited: HashSet<i64>,
}

impl TraversalState {
    fn new(limit: usize) -> Self {
        Self {
            records: ImpactTop::new(limit),
            frontier: Vec::new(),
            visited: HashSet::new(),
        }
    }
}

/// Seed the BFS with depth-1 records; detailed mode also fills the frontier
/// and visited set.
fn seed_impact(conn: &Connection, ctx: &ImpactCtx<'_>, state: &mut TraversalState) -> Result<()> {
    direct_impact_records(conn, ctx, &mut state.records)?;
    Ok(())
}

/// Expand deeper levels until the cap, an empty frontier, or the known file count.
fn walk_levels(conn: &Connection, ctx: &ImpactCtx<'_>, state: &mut TraversalState) -> Result<bool> {
    let total_files = impact_source_files(conn, ctx)?;
    let mut truncated = false;
    for d in 2..=ctx.depth {
        if state.records.len() >= total_files {
            break;
        }
        if state.frontier.is_empty() && state.visited.is_empty() {
            let (frontier, visited) = direct_impact_frontier(conn, ctx)?;
            state.frontier = frontier;
            state.visited = visited;
        }
        let next = level_impact(conn, ctx, state, d)?;
        if next.is_empty() {
            break;
        }
        if d == ctx.depth {
            truncated = true;
        }
        state.frontier = next;
    }
    Ok(truncated)
}

/// Depth > 1: reverse BFS. R1 is the exact depth-1 edge set; deeper levels
/// expand from R1's source symbols, so deeper search is monotone.
pub(super) fn impact_deep(qctx: &QueryCtx<'_>, ctx: &ImpactCtx<'_>) -> Result<QueryOutput> {
    let conn = qctx.conn;
    let mut state = TraversalState::new(ctx.limit);
    seed_impact(conn, ctx, &mut state)?;
    let truncated = walk_levels(conn, ctx, &mut state)?;
    let (total, paths) = state.records.finish();
    if total == 0 {
        return Err(impact_no_results(ctx));
    }
    let pointers: Vec<Pointer> = paths
        .into_iter()
        .map(|record| Pointer {
            file: record.path,
            line: record.line,
            tag: "",
            detail: String::new(),
        })
        .collect();
    let out = collect_output(qctx, pointers.into_iter().map(Ok), ctx.limit, total)?;
    let mut out = impact_with_caveat(out, ctx.caveat, ctx.extended);
    if truncated {
        let header = out.header.get_or_insert_with(String::new);
        header.push_str(&format!(
            "\naw: depth cap {} reached; deeper dependents may exist",
            ctx.depth
        ));
    }
    Ok(out)
}

/// Exact upper bound on files that can be an edge source for the selected
/// kinds. New indexes store per-kind and all-kind counters; older indexes and
/// mixed subsets fall back to an edge scan. The calls-only case must include
/// module-level call sites: a module symbol can be a `calls` edge source.
fn impact_source_files(conn: &Connection, ctx: &ImpactCtx<'_>) -> Result<usize> {
    let kind_sql = ctx.kind_sql;
    let key = match ctx.kinds {
        [kind] => match *kind {
            EdgeKind::Calls => Some(db::meta_keys::EDGE_CALLS_SRC_FILES),
            EdgeKind::Imports => Some(db::meta_keys::EDGE_IMPORTS_SRC_FILES),
            EdgeKind::Inherits => Some(db::meta_keys::EDGE_INHERITS_SRC_FILES),
            _ => None,
        },
        _ if ctx.kinds.len() == 3
            && ctx.kinds.iter().all(|k| {
                matches!(*k, EdgeKind::Calls | EdgeKind::Imports | EdgeKind::Inherits)
            }) =>
        {
            Some(db::meta_keys::EDGE_ALL_SRC_FILES)
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
            "SELECT COUNT(DISTINCT s.file_path) FROM edges e
             {EDGE_SRC_JOIN}
             WHERE e.kind IN ({kind_sql})"
        ),
        [],
        |r| r.get::<_, i64>(0),
    )? as usize)
}

fn direct_impact_records(
    conn: &Connection,
    ctx: &ImpactCtx<'_>,
    records: &mut ImpactTop,
) -> Result<()> {
    let kind_sql = ctx.kind_sql;
    let sql = format!(
        "SELECT MIN(e.site_line), MIN(e.kind), s.file_path, MIN(s.qualname), MIN(s.kind)
         FROM edges e
         {IMPACT_DST_JOINS}
         {EDGE_SRC_JOIN}
         WHERE df.path = ?1 AND e.kind IN ({kind_sql})
         GROUP BY s.file_path"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([ctx.rel], |r| {
        Ok(ImpactRecord {
            path: r.get(2)?,
            depth: 1,
            line: r.get(0)?,
            kind: r.get(1)?,
            qualname: r.get(3)?,
            src_kind: r.get(4)?,
        })
    })?;
    for row in rows {
        records.insert(row?);
    }
    Ok(())
}

fn direct_impact_frontier(
    conn: &Connection,
    ctx: &ImpactCtx<'_>,
) -> Result<(Vec<i64>, HashSet<i64>)> {
    let kind_sql = ctx.kind_sql;
    let mut visited: HashSet<i64> = file_symbol_ids(conn, ctx.rel)?.into_iter().collect();
    let sql = format!(
        "SELECT DISTINCT e.src_id FROM edges e
         {IMPACT_DST_JOINS}
         WHERE df.path = ?1 AND e.kind IN ({kind_sql})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut frontier = Vec::new();
    for id in stmt.query_map([ctx.rel], |r| r.get::<_, i64>(0))? {
        let id = id?;
        if visited.insert(id) {
            frontier.push(id);
        }
    }
    Ok((frontier, visited))
}

/// The six columns both BFS-level SELECTs return, in `impact_level_row`
/// order: the frontier source id, then the five `ImpactRecord` fields. One
/// source for the column list and its binding keeps the two statements from
/// drifting apart.
const IMPACT_LEVEL_COLUMNS: &str = "e.src_id, e.site_line, e.kind, s.file_path, s.qualname, s.kind";

/// Map one BFS-level row: the frontier source id plus its `ImpactRecord`.
/// Column order must match `IMPACT_LEVEL_COLUMNS`.
fn impact_level_row(r: &rusqlite::Row<'_>, depth: usize) -> rusqlite::Result<(i64, ImpactRecord)> {
    Ok((
        r.get::<_, i64>(0)?,
        ImpactRecord {
            path: r.get(3)?,
            depth,
            line: r.get(1)?,
            kind: r.get(2)?,
            qualname: r.get(4)?,
            src_kind: r.get(5)?,
        },
    ))
}

fn level_impact(
    conn: &Connection,
    ctx: &ImpactCtx<'_>,
    state: &mut TraversalState,
    depth: usize,
) -> Result<Vec<i64>> {
    let kind_sql = ctx.kind_sql;
    let TraversalState {
        frontier,
        visited,
        records,
    } = &mut *state;
    if frontier.is_empty() {
        return Ok(Vec::new());
    }
    let mut next = Vec::new();
    crate::db::for_each_chunk(conn, frontier, |conn, placeholders, chunk| {
        let sql = format!(
            "SELECT {IMPACT_LEVEL_COLUMNS}
             FROM edges e
             {EDGE_SRC_JOIN}
             WHERE e.dst_id IN ({placeholders}) AND e.kind IN ({kind_sql})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let params = id_params(chunk);
        let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
            impact_level_row(r, depth)
        })?;
        for r in rows {
            let (src, record) = r?;
            records.insert(record);
            if visited.insert(src) {
                next.push(src);
            }
        }
        Ok(())
    })?;
    Ok(next)
}

/// One settled impact record: the representative row kept for one file path.
/// Named fields replace the positional 6-tuple so a SELECT column reorder
/// cannot silently misbind the `ImpactTop` comparison and render inputs.
/// Field order mirrors the retired tuple, so the derived `Ord` keeps the
/// lexicographic order `BinaryHeap` relied on (paths are distinct in the heap).
#[derive(Eq, Ord, PartialEq, PartialOrd)]
struct ImpactRecord {
    path: String,
    depth: usize,
    line: i64,
    kind: String,
    qualname: String,
    src_kind: String,
}

/// Symbol ids defined by one indexed file (the impact BFS seed set).
fn file_symbol_ids(conn: &Connection, rel: &str) -> Result<Vec<i64>> {
    let mut stmt = conn
        .prepare("SELECT s.id FROM symbols s JOIN files f ON f.id = s.file_id WHERE f.path = ?1")?;
    let rows = stmt.query_map([rel], |r| r.get(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

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

    fn insert(&mut self, record: ImpactRecord) {
        // Depth and site line are the primary key. On an exact tie, compare
        // stable semantic fields so the representative does not depend on
        // SQL row order or rowid/file-id assignment.
        let replace = match self.seen.get(&record.path) {
            None => true,
            Some(old) => {
                (
                    record.depth,
                    record.line,
                    &record.kind,
                    &record.qualname,
                    &record.src_kind,
                ) < (old.depth, old.line, &old.kind, &old.qualname, &old.src_kind)
            }
        };
        if replace {
            self.seen.insert(record.path.clone(), record);
        }
    }

    fn finish(self) -> (usize, Vec<ImpactRecord>) {
        let total = self.seen.len();
        let mut heap: BinaryHeap<ImpactRecord> = BinaryHeap::new();
        for record in self.seen.into_values() {
            if heap.len() < self.limit {
                heap.push(record);
            } else if heap.peek().is_some_and(|max| record.path < max.path) {
                heap.pop();
                heap.push(record);
            }
        }
        let mut records = heap.into_vec();
        records.sort_by(|a, b| a.path.cmp(&b.path));
        (total, records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impact_top_ties_are_deterministic() {
        let mut first = ImpactTop::new(10);
        first.insert(ImpactRecord {
            path: "p".into(),
            depth: 1,
            line: 5,
            kind: "imports".into(),
            qualname: "q".into(),
            src_kind: "function".into(),
        });
        first.insert(ImpactRecord {
            path: "p".into(),
            depth: 1,
            line: 5,
            kind: "calls".into(),
            qualname: "q".into(),
            src_kind: "function".into(),
        });
        let (_, records) = first.finish();
        assert_eq!(records[0].kind, "calls");

        let mut second = ImpactTop::new(10);
        second.insert(ImpactRecord {
            path: "p".into(),
            depth: 1,
            line: 5,
            kind: "calls".into(),
            qualname: "q".into(),
            src_kind: "function".into(),
        });
        second.insert(ImpactRecord {
            path: "p".into(),
            depth: 1,
            line: 5,
            kind: "imports".into(),
            qualname: "q".into(),
            src_kind: "function".into(),
        });
        let (_, records) = second.finish();
        assert_eq!(records[0].kind, "calls");
    }
}
