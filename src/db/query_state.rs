//! Incremental-build state queries: previous-index probing, schema
//! compatibility, carried-symbol loading, and gap detection for changed names.

use super::*;

/// Fast path gate: a usable previous index under the SAME schema AND grammar
/// versions (any mismatch disables the fast path for the whole build).
pub fn probe_old(root: &Path) -> Result<Option<Connection>> {
    reject_symlink(&eg_dir(root), ".engram directory")?;
    let old_path = db_path(root);
    reject_symlink(&old_path, "index.db")?;
    if !old_path.exists() {
        return Ok(None);
    }
    let old = match Connection::open_with_flags(&old_path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    if !schema_compatible(&old) {
        return Ok(None);
    }
    Ok(Some(old))
}

/// The read-only fast path needs the current version stamp, every required
/// column, the FTS table, and the grammar/signature/metrics manifest versions.
fn schema_compatible(old: &Connection) -> bool {
    let version: i32 = match old.query_row("PRAGMA user_version", [], |r| r.get(0)) {
        Ok(v) => v,
        Err(_) => return false,
    };
    if version != SCHEMA_VERSION {
        return false;
    }
    // Require every v10 column before enabling the fast path;
    // otherwise fall back to a full fresh build.
    let required_columns = [
        ("symbols", "file_path"),
        ("symbols", "is_test"),
        ("unresolved_calls", "site_line"),
        ("unresolved_calls", "kind"),
    ];
    for (table, column) in required_columns {
        if !column_present(old, table, column).unwrap_or(false) {
            return false;
        }
    }
    if !object_exists(old, "table", "symbols_fts") {
        return false;
    }
    if meta_get(old, meta_keys::GRAMMAR_VERSION).as_deref() != Some(GRAMMAR_VERSION) {
        return false;
    }
    if meta_get(old, meta_keys::SIGNATURE_VERSION).as_deref() != Some(SIGNATURE_VERSION) {
        return false;
    }
    if meta_get(old, meta_keys::METRICS_VERSION).as_deref() != Some(METRICS_VERSION) {
        return false;
    }
    true
}

/// Fallible `sqlite_master` probe: distinguishes "absent" from "probe failed".
pub(super) fn object_exists_result(
    conn: &Connection,
    kind: &str,
    name: &str,
) -> rusqlite::Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = ?1 AND name = ?2",
        rusqlite::params![kind, name],
        |r| r.get(0),
    )?;
    Ok(count > 0)
}

/// True when `sqlite_master` holds an object of `kind` named `name`.
fn object_exists(conn: &Connection, kind: &str, name: &str) -> bool {
    object_exists_result(conn, kind, name).unwrap_or(false)
}

/// True when `table` has a column named `column`.
pub(super) fn column_present(
    conn: &Connection,
    table: &str,
    column: &str,
) -> rusqlite::Result<bool> {
    let present: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
        rusqlite::params![table, column],
        |r| r.get(0),
    )?;
    Ok(present > 0)
}

/// Stat fingerprint of one previous-index file row: (mtime, size,
/// content_hash). Named so a column reorder in the SELECT cannot silently
/// swap a hash for an mtime on the fast path.
pub struct FileStat {
    pub mtime: i64,
    pub size: i64,
    pub content_hash: i64,
}

/// (rel -> stat fingerprint) of the previous index's files.
pub fn old_files(old: &Connection) -> Result<HashMap<String, FileStat>> {
    let mut stmt = old.prepare("SELECT path, mtime, size, content_hash FROM files")?;
    let rows = stmt
        .query_map(rusqlite::params![], |r| {
            Ok((
                r.get::<_, String>(0)?,
                FileStat {
                    mtime: r.get::<_, i64>(1)?,
                    size: r.get::<_, i64>(2)?,
                    content_hash: r.get::<_, i64>(3)?,
                },
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows.into_iter().collect())
}

/// Count parse-error files among the carried set (fast-path gate denominator).
/// One chunked IN query instead of one query per file.
pub fn old_error_count(old: &Connection, rels: &[String]) -> Result<usize> {
    let mut n = 0usize;
    for_each_chunk(old, rels, |old, placeholders, chunk| {
        let sql = format!(
            "SELECT COUNT(*) FROM files WHERE parse_status != 'ok' AND path IN ({placeholders})"
        );
        let c: i64 = old.query_row(&sql, rusqlite::params_from_iter(chunk.iter()), |r| r.get(0))?;
        n += c as usize;
        Ok(())
    })?;
    Ok(n)
}

/// Transitive dirty closure over the previous index (fast-path correctness):
/// every file with an edge (any kind, resolved destination) into a changed
/// file must be re-extracted, or its copied edge dies with the old destination
/// id — and any file calling into a re-extracted file needs the same treatment.
/// Returns changed + closure; the caller moves the carried members to `fresh`.
pub fn dirty_closure(old: &Connection, changed: &[String]) -> Result<HashSet<String>> {
    // Work-queue (R6f Blocker 4): each round queries only the NEWLY dirtied
    // paths, not the whole accumulated set. The old fixed-point re-queried
    // everything each round — O(N^2) on long dependency chains.
    let mut dirty: HashSet<String> = changed.iter().cloned().collect();
    let mut frontier: Vec<String> = changed.to_vec();
    while !frontier.is_empty() {
        frontier.sort();
        let mut new_paths: HashSet<String> = HashSet::new();
        for_each_chunk(old, &frontier, |old, placeholders, chunk| {
            let sql = format!(
                "SELECT DISTINCT f.path
                 FROM edges e
                 {EDGE_SRC_FILE_JOIN}
                 JOIN symbols ds ON ds.id = e.dst_id JOIN files df ON df.id = ds.file_id
                 WHERE df.path IN ({placeholders})"
            );
            collect_gap_paths(old, &sql, chunk, &mut new_paths)
        })?;
        // Work-queue guard: already-dirty paths never re-enter the frontier,
        // so the BFS terminates on dependency cycles too.
        new_paths.retain(|p| !dirty.contains(p));
        let next: Vec<String> = new_paths.into_iter().collect();
        dirty.extend(next.iter().cloned());
        frontier = next;
    }
    Ok(dirty)
}

/// Every symbol name and qualname in the old index (Amendment 15): the fast
/// path compares fresh extraction names against this set to detect names that
/// did not exist before the change.
pub fn symbol_names(conn: &Connection) -> Result<HashSet<String>> {
    let mut out = HashSet::new();
    let mut stmt = conn.prepare("SELECT qualname FROM symbols")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    for r in rows {
        out.insert(r?);
    }
    Ok(out)
}

/// Old symbol qualnames grouped by the files that declare them. Used to detect
/// a newly added duplicate candidate, not just a brand-new name.
pub fn symbol_qualname_files(conn: &Connection) -> Result<HashMap<String, HashSet<String>>> {
    let mut out: HashMap<String, HashSet<String>> = HashMap::new();
    let mut stmt =
        conn.prepare("SELECT s.qualname, f.path FROM symbols s JOIN files f ON f.id = s.file_id")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for r in rows {
        let (qualname, path) = r?;
        out.entry(qualname).or_default().insert(path);
    }
    Ok(out)
}

/// Qualnames declared by the given old-index files. Bounds candidate
/// invalidation to the files that actually changed.
pub fn qualnames_for_files(old: &Connection, files: &[String]) -> Result<HashSet<String>> {
    let mut out = HashSet::new();
    for_each_chunk(old, files, |old, placeholders, chunk| {
        let sql = format!(
            "SELECT DISTINCT s.qualname
             FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE f.path IN ({placeholders})"
        );
        collect_gap_paths(old, &sql, chunk, &mut out)
    })?;
    Ok(out)
}

/// Carried files whose edges or unresolved sites mention a qualname whose
/// candidate set changed (grew, shrank, or was renamed). Those sites may now
/// be ambiguous or resolve to a different candidate; re-extract them so the
/// incremental result matches a fresh build.
pub fn gap_files_for_candidate_changes(
    old: &Connection,
    changed_qualnames: &HashSet<String>,
) -> Result<HashSet<String>> {
    if changed_qualnames.is_empty() {
        return Ok(HashSet::new());
    }
    let names: Vec<String> = changed_qualnames.iter().cloned().collect();
    let mut out = HashSet::new();
    for_each_chunk(old, &names, |old, placeholders, chunk| {
        let edge_sql = format!(
            "SELECT DISTINCT f.path
             FROM edges e
             JOIN symbols s ON s.id = e.src_id
             JOIN files f ON f.id = s.file_id
             WHERE e.kind IN {kinds} AND e.dst_name IN ({placeholders})",
            kinds = edge_kinds_sql()
        );
        collect_gap_paths(old, &edge_sql, chunk, &mut out)?;
        let unresolved_sql = format!(
            "SELECT DISTINCT f.path
             FROM unresolved_calls u JOIN files f ON f.id = u.file_id
             WHERE u.dst_name IN ({placeholders})"
        );
        collect_gap_paths(old, &unresolved_sql, chunk, &mut out)?;
        Ok(())
    })?;
    Ok(out)
}

/// Escape the three GLOB metacharacters into bracket classes so a bound
/// segment matches itself literally (`_` and `%` are not special in GLOB).
fn glob_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if matches!(c, '*' | '?' | '[') {
            out.push('[');
            out.push(c);
            out.push(']');
        } else {
            out.push(c);
        }
    }
    out
}

/// Carried files whose resolution gaps mention any of `new_names`
/// (Amendment 15): a file with an unresolved call or an AMBIGUOUS calls edge
/// whose last name segment is a newly added symbol must be re-extracted, or
/// the fast path would keep a stale unresolved answer. Bounded by the gap
/// rows, not by the corpus.
pub fn gap_files_for_new_names(
    old: &Connection,
    new_qualnames: &HashSet<String>,
) -> Result<HashSet<String>> {
    if new_qualnames.is_empty() {
        return Ok(HashSet::new());
    }
    // Match by last segment: an unresolved `b.foo` heals when any newly
    // added qualname ends in `foo` (bare-name collisions in the old index
    // must not hide the new symbol — R6 review finding). `dst == seg` or
    // `dst` ends with `.seg` is exactly "the last segment is `seg`"; the two
    // GLOB arms share one numbered parameter per segment. GLOB is
    // case-sensitive, matching the Rust set membership it replaced.
    //
    // SQLite's default binary-tree depth limit (`SQLITE_MAX_EXPR_DEPTH`) is
    // 1000. Each segment generates 2 `OR` terms, so a batch of 100 segments
    // creates at most 200 `OR` terms, maintaining a 5x safety margin against
    // expression-tree-overflow errors.
    let mut segments: Vec<String> = new_qualnames
        .iter()
        .map(|q| glob_literal(q.rsplit('.').next().unwrap_or(q)))
        .collect();
    segments.sort_unstable();
    segments.dedup();
    const GLOB_OR_CHUNK: usize = 100;
    let mut out = HashSet::new();
    for chunk in segments.chunks(GLOB_OR_CHUNK) {
        // Parameter ?N is referenced twice: once for the exact match, once
        // for the `*.<segment>` suffix match.
        let last_segment = (1..=chunk.len())
            .map(|n| format!("dst_name GLOB ?{n} OR dst_name GLOB '*.' || ?{n}"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let edge_sql = format!(
            "SELECT DISTINCT f.path
             FROM edges e
             JOIN symbols s ON s.id = e.src_id
             JOIN files f ON f.id = s.file_id
             WHERE e.kind IN {kinds} AND e.confidence = '{ambiguous}'
               AND ({last_segment})",
            kinds = edge_kinds_sql(),
            ambiguous = Confidence::Ambiguous.as_str()
        );
        collect_gap_paths(old, &edge_sql, chunk, &mut out)?;
        let unresolved_sql = format!(
            "SELECT DISTINCT f.path
             FROM unresolved_calls u JOIN files f ON f.id = u.file_id
             WHERE {last_segment}"
        );
        collect_gap_paths(old, &unresolved_sql, chunk, &mut out)?;
    }
    Ok(out)
}

/// Collect the first-column rows of one chunked gap query (`f.path` or
/// `s.qualname`) into `out`, so the filter stays inside SQLite instead of
/// materializing every matched row into memory.
fn collect_gap_paths(
    conn: &Connection,
    sql: &str,
    chunk: &[String],
    out: &mut HashSet<String>,
) -> Result<()> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
        r.get::<_, String>(0)
    })?;
    for row in rows {
        out.insert(row?);
    }
    Ok(())
}

/// Source-side join `edges e -> symbols s -> files f`, the single rendering
/// shared by the import-scoping, stats, and dirty-closure SQL so the join
/// aliases have one home.
pub(crate) const EDGE_SRC_FILE_JOIN: &str =
    "JOIN symbols s ON s.id = e.src_id JOIN files f ON f.id = s.file_id";

/// Resolved-caller subquery for the outer symbol `s` — the single definition
/// of "number of callers", shared by search ranking and reuse calibration.
pub(crate) const CALLER_COUNT_SQL: &str =
    "(SELECT COUNT(DISTINCT e.src_id) FROM edges e JOIN symbols src ON src.id = e.src_id \
      WHERE e.dst_id = s.id AND e.kind = 'calls' AND e.src_id != s.id \
      AND src.is_test = s.is_test)";

/// Quoted edge-kind SQL literals joined with commas — the one renderer for
/// every `kind IN (...)` fragment built from the enum, so a new kind joins
/// the vocabulary in a single place.
pub(crate) fn edge_kind_sql(kinds: &[EdgeKind]) -> String {
    kinds
        .iter()
        .map(|k| format!("'{}'", k.as_str()))
        .collect::<Vec<_>>()
        .join(",")
}

/// Edge kinds counted as references by the dead-code and gap-heal queries,
/// rendered from the enum as one shared SQL fragment so a new edge kind joins
/// the reference set in one place (`db.rs` and `query/analysis.rs` both
/// interpolate it).
pub(crate) fn edge_kinds_sql() -> String {
    format!(
        "({})",
        edge_kind_sql(&[
            EdgeKind::Calls,
            EdgeKind::Imports,
            EdgeKind::Inherits,
            EdgeKind::TypeRef,
        ])
    )
}
