//! Reverse-dependency impact queries and their traversal bookkeeping.

use super::*;

use std::collections::BinaryHeap;

/// `--edges` accepted kinds plus `all`. `type_ref` rows are storage-only
/// (no surface reads them), so the CLI keeps rejecting that spelling.
const EDGE_CLI_KINDS: [EdgeKind; 3] = [EdgeKind::Calls, EdgeKind::Imports, EdgeKind::Inherits];

/// Validate a `--edges` spec into its kinds (sorted, deduplicated). Called at
/// the service boundary so `CommandRequest::Impact` carries validated kinds.
pub(crate) fn parse_edge_kinds(spec: &str) -> Result<Vec<EdgeKind>> {
    let mut out: Vec<EdgeKind> = Vec::new();
    let mut saw_all = false;
    for raw in spec.split(',') {
        let part = raw.trim();
        if part == "all" {
            saw_all = true;
            out.extend(EDGE_CLI_KINDS);
            continue;
        }
        match EdgeKind::parse(part) {
            Some(kind) if EDGE_CLI_KINDS.contains(&kind) => out.push(kind),
            _ => {
                let mut names: Vec<&str> =
                    EDGE_CLI_KINDS.iter().map(|kind| kind.as_str()).collect();
                names.push("all");
                let accepted = names.join(",");
                return Err(AwError::Usage(match part {
                    "" => format!("--edges requires {accepted}"),
                    other => format!("unknown --edges value '{other}' ({accepted})"),
                }));
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
    Ok(out)
}

/// Destination-side joins shared by every impact SQL statement that filters
/// on the destination file: edge -> destination symbol -> its file.
const IMPACT_DST_JOINS: &str = "JOIN symbols ds ON ds.id = e.dst_id
         JOIN files df ON df.id = ds.file_id";

/// Source-symbol join: edge -> source symbol. Statements that also read or
/// count source files append `IMPACT_SRC_FILE_JOIN`.
const IMPACT_SRC_JOIN: &str = "JOIN symbols ss ON ss.id = e.src_id";

/// Source-file join: source symbol -> its file.
const IMPACT_SRC_FILE_JOIN: &str = "JOIN files sf ON sf.id = ss.file_id";

fn impact_detail(
    ctx: &ImpactCtx<'_>,
    qualname: &str,
    src_kind: &str,
    kind: &str,
    depth: usize,
) -> String {
    let mut parts = Vec::new();
    if ctx.context
        && !qualname.is_empty()
        && matches!(
            SymbolKind::parse(src_kind),
            Some(SymbolKind::Function | SymbolKind::Method)
        )
    {
        parts.push(format!("[in {qualname}]"));
    }
    if ctx.explain {
        parts.push(format!("[{kind} depth={depth}]"));
    }
    parts.join(" ")
}

fn impact_empty_detail(kinds: &[EdgeKind]) -> String {
    let has_imports = kinds.contains(&EdgeKind::Imports);
    let has_inherits = kinds.contains(&EdgeKind::Inherits);
    match (has_imports, has_inherits) {
        (true, true) => "0 impacting files found (unresolved imports/bases are omitted)",
        (true, false) => "0 impacting files found (unresolved imports are omitted)",
        (false, true) => "0 impacting files found (unresolved bases are omitted)",
        (false, false) => "0 impacting files found",
    }
    .to_string()
}

fn impact_no_results(ctx: &ImpactCtx<'_>) -> AwError {
    AwError::NoResultsCaveated {
        caveat: ctx.caveat,
        detail: impact_empty_detail(ctx.kinds),
    }
}

/// Empty default (calls-only) detail: name the import/inherit files the
/// calls-only default skipped, so a file that only has importers points the
/// user at `--edges imports,inherits` instead of a bare zero.
fn importer_caveat_detail(conn: &Connection, rel: &str) -> Result<String> {
    let count_files = |kind: EdgeKind| -> Result<usize> {
        Ok(conn.query_row(
            &format!(
                "SELECT COUNT(DISTINCT sf.path) FROM edges e
                 {IMPACT_DST_JOINS}
                 {IMPACT_SRC_JOIN}
                 {IMPACT_SRC_FILE_JOIN}
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

/// Impact query options: call-site vs grouped, traversal depth, the validated
/// edge kinds, and output detail.
pub struct ImpactOpts {
    pub sites: bool,
    pub depth: usize,
    pub kinds: Vec<EdgeKind>,
    pub context: bool,
    pub explain: bool,
}

pub fn impact(
    qctx: &QueryCtx<'_>,
    file: &str,
    limit: usize,
    opts: ImpactOpts,
) -> Result<QueryOutput> {
    let ImpactOpts {
        sites,
        depth,
        kinds,
        context,
        explain,
    } = opts;
    let limit = clamp_limit(limit)?;
    let depth = depth.clamp(1, 10);
    let kind_sql = db::edge_kind_sql(&kinds);
    let rel = indexed_file(qctx.conn, file)?;
    let extended = depth > 1 || context || kinds.iter().any(|k| *k != EdgeKind::Calls);
    let caveat = if extended {
        if kinds.iter().any(|k| *k != EdgeKind::Calls) {
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
            return impact_direct_grouped(qctx, &ctx);
        }
        return impact_direct_sites(qctx, &ctx);
    }
    impact_deep(qctx, &ctx)
}

/// Shared, immutable arguments for the three impact phases. The phases stay
/// contiguous; this is not a generic query framework.
struct ImpactCtx<'a> {
    rel: &'a str,
    limit: usize,
    depth: usize,
    kind_sql: &'a str,
    caveat: &'static str,
    kinds: &'a [EdgeKind],
    context: bool,
    explain: bool,
    extended: bool,
}

fn impact_with_caveat(mut out: QueryOutput, caveat: &str, extended: bool) -> QueryOutput {
    if extended {
        prepend_header(&mut out, caveat);
    }
    out
}

/// Depth-1 grouped total: distinct source files with any matching edge.
fn impact_grouped_total(conn: &Connection, ctx: &ImpactCtx<'_>) -> Result<usize> {
    Ok(conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT sf.path) FROM edges e
             {IMPACT_DST_JOINS}
             {IMPACT_SRC_JOIN}
             {IMPACT_SRC_FILE_JOIN}
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
        "SELECT sf.path, MIN(e.site_line) AS line,
                COUNT(DISTINCT {}) AS n
         FROM edges e
         {IMPACT_DST_JOINS}
         {IMPACT_SRC_JOIN}
         {IMPACT_SRC_FILE_JOIN}
         WHERE df.path = ?1 AND e.kind IN ({})
         GROUP BY sf.path ORDER BY sf.path LIMIT {}",
        impact_site_key(),
        ctx.kind_sql,
        ctx.limit
    ))?;
    let rows = stmt.query_map([ctx.rel], |r| {
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
fn impact_direct_grouped(qctx: &QueryCtx<'_>, ctx: &ImpactCtx<'_>) -> Result<QueryOutput> {
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
    let out = collect_output(
        qctx,
        rows.into_iter().map(Ok),
        ctx.limit,
        "0 impacting files found",
        total_files,
    )?;
    Ok(impact_with_caveat(out, ctx.caveat, ctx.extended))
}

/// Depth-1 per-site rows: one row per call site, with optional context/explain.
fn impact_site_total(conn: &Connection, ctx: &ImpactCtx<'_>) -> Result<usize> {
    Ok(conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT {}) FROM edges e
             {IMPACT_DST_JOINS}
             {IMPACT_SRC_JOIN}
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
        "SELECT MIN(e.site_line), MIN(e.confidence), MIN(sf.path),
                MIN(ss.qualname), MIN(ss.kind), MIN(e.kind)
         FROM edges e
         {IMPACT_DST_JOINS}
         {IMPACT_SRC_JOIN}
         {IMPACT_SRC_FILE_JOIN}
         WHERE df.path = ?1 AND e.kind IN ({})
         GROUP BY {}
         ORDER BY 3, 1, 4 LIMIT {}",
        ctx.kind_sql,
        impact_site_key(),
        ctx.limit
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
            tag: confidence_tag(&conf),
            detail: impact_detail(ctx, &qn, &src_kind, &kind, 1),
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

fn impact_direct_sites(qctx: &QueryCtx<'_>, ctx: &ImpactCtx<'_>) -> Result<QueryOutput> {
    let conn = qctx.conn;
    let total = impact_site_total(conn, ctx)?;
    if total == 0 {
        if ctx.extended {
            return Err(impact_no_results(ctx));
        }
        return Err(AwError::NoResults("0 impacting files found".to_string()));
    }
    let rows = impact_site_rows(conn, ctx)?;
    let out = collect_output(
        qctx,
        rows.into_iter().map(Ok),
        ctx.limit,
        "0 impacting files found",
        total,
    )?;
    Ok(impact_with_caveat(out, ctx.caveat, ctx.extended))
}

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
    if ctx.context || ctx.explain {
        return direct_impact_level(conn, ctx, state, 1);
    }
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
fn impact_deep(qctx: &QueryCtx<'_>, ctx: &ImpactCtx<'_>) -> Result<QueryOutput> {
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
            detail: impact_detail(
                ctx,
                &record.qualname,
                &record.src_kind,
                &record.kind,
                record.depth,
            ),
        })
        .collect();
    let out = collect_output(
        qctx,
        pointers.into_iter().map(Ok),
        ctx.limit,
        "0 impacting files found",
        total,
    )?;
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
    ctx: &ImpactCtx<'_>,
    records: &mut ImpactTop,
) -> Result<()> {
    let kind_sql = ctx.kind_sql;
    let sql = format!(
        "SELECT MIN(e.site_line), MIN(e.kind), ss.file_path, MIN(ss.qualname), MIN(ss.kind)
         FROM edges e
         {IMPACT_DST_JOINS}
         {IMPACT_SRC_JOIN}
         WHERE df.path = ?1 AND e.kind IN ({kind_sql})
         GROUP BY ss.file_path"
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
const IMPACT_LEVEL_COLUMNS: &str =
    "e.src_id, e.site_line, e.kind, ss.file_path, ss.qualname, ss.kind";

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

fn direct_impact_level(
    conn: &Connection,
    ctx: &ImpactCtx<'_>,
    state: &mut TraversalState,
    depth: usize,
) -> Result<()> {
    let kind_sql = ctx.kind_sql;
    let sql = format!(
        "SELECT {IMPACT_LEVEL_COLUMNS}
         FROM edges e
         {IMPACT_DST_JOINS}
         {IMPACT_SRC_JOIN}
         WHERE df.path = ?1 AND e.kind IN ({kind_sql})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([ctx.rel], |r| impact_level_row(r, depth))?;
    // The file's own symbols are visited from the start: the BFS never
    // expands back into the impacted file.
    state.visited = file_symbol_ids(conn, ctx.rel)?.into_iter().collect();
    for r in rows {
        let (src, record) = r?;
        state.records.insert(record);
        if state.visited.insert(src) {
            state.frontier.push(src);
        }
    }
    Ok(())
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
             {IMPACT_SRC_JOIN}
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
/// cannot silently mis-bind the `ImpactTop` comparison and render inputs.
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
