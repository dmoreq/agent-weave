//! rusqlite layer: schema lifecycle, meta manifest, build lock (flock),
//! build inserts under one caller-owned transaction (Amendment 11b; 500-row
//! prepared batches), and the full-atomic-replacement build policy.
//!
//! Build semantics (spec, single rule): `aw build` always produces a full
//! atomic replacement — extract into `.aw/index.db.new` (fresh schema), close,
//! then rename(2) onto `index.db`. A concurrent reader sees the old or the new
//! DB, never a missing one. There is no delete step; orphan sidecars are
//! cleaned at build start and by `doctor`.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rusqlite::{Connection, OpenFlags};

use crate::error::{is_sqlite_corruption, AwError, Result};
use crate::extract::{
    CarriedIndex, CarriedSymbol, Confidence, EdgeKind, EdgeSiteKind, NewSymbol, SymbolKind,
    SCHEMA_VERSION,
};

/// Compile-time grammar identity, derived from the Cargo.lock-pinned grammar
/// crate versions by build.rs (R6d A8: a hand-maintained string had already
/// drifted from the lock). The identity gates the fast path.
pub const GRAMMAR_VERSION: &str = env!("AW_GRAMMAR_VERSION");
/// Extraction revision: older indexes must be re-extracted when capture or
/// resolution rules change. This does not change the table shape.
pub const SIGNATURE_VERSION: &str = "3";
/// Per-symbol metric-rule version (Amendment 19); a mismatch disables the fast path.
pub const METRICS_VERSION: &str = "2";

/// Batch size for every `IN (...)` bind list. SQLite's variable limit is 999;
/// 500 keeps headroom for the other bound parameters in the same statement.
pub(crate) const SQL_CHUNK: usize = 500;

/// `?,?,...` placeholders for an `IN (...)` list of `count` bound values.
pub(crate) fn sql_placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(",")
}

/// Chunked IN-query scaffold: slices `values` into `SQL_CHUNK` batches
/// (SQLite's variable limit) and runs `body` per batch with the connection,
/// the batch's placeholder list, and the batch's value slice. `&Transaction`
/// call sites deref to `Connection`.
pub(crate) fn for_each_chunk<T: rusqlite::ToSql, F>(
    conn: &Connection,
    values: &[T],
    mut body: F,
) -> Result<()>
where
    F: FnMut(&Connection, &str, &[T]) -> Result<()>,
{
    for chunk in values.chunks(SQL_CHUNK) {
        body(conn, &sql_placeholders(chunk.len()), chunk)?;
    }
    Ok(())
}

/// Row mappers for the fixed-width integer tuples used by counter queries.
pub(crate) fn ints2(r: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, i64)> {
    Ok((r.get(0)?, r.get(1)?))
}

pub(crate) fn ints3(r: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, i64, i64)> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
}

pub(crate) fn ints4(r: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, i64, i64, i64)> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
}

pub fn aw_dir(root: &Path) -> PathBuf {
    root.join(".aw")
}

fn reject_symlink(path: &Path, label: &str) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(AwError::Internal(format!(
                "refusing symlinked {label}: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn unsafe_lock(path: &Path) -> AwError {
    AwError::Internal(format!("refusing unsafe build lock: {}", path.display()))
}

/// Create or validate `.aw/`, refusing a symlinked directory. A symlink can
/// redirect every later write outside the repository root.
pub fn ensure_aw_dir(root: &Path) -> Result<PathBuf> {
    let dir = aw_dir(root);
    reject_symlink(&dir, ".aw directory")?;
    match fs::symlink_metadata(&dir) {
        Ok(meta) if !meta.is_dir() => {
            return Err(AwError::Internal(format!(
                ".aw exists but is not a directory: {}",
                dir.display()
            )));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Creation race (two first-time writers, e.g. concurrent mem
            // writes — memory takes no build lock): the loser's create_dir
            // reports AlreadyExists and re-validates the winner's directory.
            if let Err(create_err) = fs::create_dir(&dir) {
                if create_err.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(create_err.into());
                }
            }
            reject_symlink(&dir, ".aw directory")?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(dir)
}

pub fn db_path(root: &Path) -> PathBuf {
    aw_dir(root).join("index.db")
}

/// Best-effort byte size of one file; `0` when the path is absent or
/// unreadable — size lines are presentation, never a hard failure.
pub(crate) fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

pub fn db_new_path(root: &Path) -> PathBuf {
    aw_dir(root).join("index.db.new")
}

fn unexpected_gitignore(marker: &Path) -> AwError {
    AwError::Internal(format!(
        "unexpected .aw/.gitignore content: {}",
        marker.display()
    ))
}

/// Write the tool-owned `.aw/.gitignore` (`*`) on first build: the whole
/// directory disappears from `git status` without touching the repo's own
/// .gitignore (allowed write path per invariant 3's constant list).
/// Fail-closed: a symlink, non-regular file (FIFO/device — rejected before
/// any open), over-cap file, or wrong content is an error, never kept silent.
pub fn ensure_aw_gitignore(root: &Path) -> Result<()> {
    let dir = ensure_aw_dir(root)?;
    let marker = dir.join(".gitignore");
    reject_symlink(&marker, ".aw/.gitignore")?;
    match fs::symlink_metadata(&marker) {
        Ok(_) => {
            // An existing marker must still be the tool's own: `read_bounded`
            // rejects non-regular files before any open and caps the read.
            let bytes = crate::walk::read_bounded(&marker)?;
            if bytes != b"*\n" {
                return Err(unexpected_gitignore(&marker));
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            publish_gitignore_marker(&dir, &marker)
        }
        Err(e) => Err(e.into()),
    }
}

/// Publish a missing marker atomically: write a private temp file, then
/// hard-link it into place. A concurrent creator either loses the link (and
/// validates the complete marker) or wins; no reader can observe a
/// half-written marker (the create-then-write race). Fail-closed: a symlink,
/// non-regular file, or wrong content is an error, never kept silent.
fn publish_gitignore_marker(dir: &Path, marker: &Path) -> Result<()> {
    let tmp = dir.join(format!(".gitignore.{}.tmp", std::process::id()));
    let write = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .and_then(|mut file| file.write_all(b"*\n"));
    if let Err(e) = write {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    let linked = fs::hard_link(&tmp, marker);
    match linked {
        Ok(()) => {
            let _ = fs::remove_file(&tmp);
            Ok(())
        }
        Err(link_err) if link_err.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&tmp);
            let bytes = crate::walk::read_bounded(marker)?;
            if bytes == b"*\n" {
                Ok(())
            } else {
                Err(unexpected_gitignore(marker))
            }
        }
        // Filesystems without hard links (rare for a local .aw):
        // fall back to the atomic rename, which replaces the target.
        Err(link_err) if link_err.kind() == std::io::ErrorKind::Unsupported => {
            let renamed = fs::rename(&tmp, marker);
            let _ = fs::remove_file(&tmp);
            renamed.map_err(AwError::from)
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e.into())
        }
    }
}

#[cfg(unix)]
fn link_count(meta: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.nlink()
}

#[cfg(not(unix))]
fn link_count(_: &fs::Metadata) -> u64 {
    1
}

/// Exclusive flock over `.aw/build.lock` for the whole build (CUJ6). This is
/// the only build lock: one POSIX flock identity for builds and (via the same
/// acquire) for `doctor --invalidate`.
pub struct BuildLock {
    file: File,
}

impl BuildLock {
    pub fn acquire(root: &Path) -> Result<BuildLock> {
        let dir = ensure_aw_dir(root)?;
        let lock_path = dir.join("build.lock");
        let mut file = open_lock_file(&lock_path)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                let pid = fs::read_to_string(&lock_path)
                    .ok()
                    .and_then(|s| {
                        s.lines().next().and_then(|l| {
                            l.split_whitespace()
                                .next()
                                .and_then(|p| p.parse::<u32>().ok())
                        })
                    })
                    .unwrap_or(0);
                return Err(AwError::Locked(pid));
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
        }
        // Orphan sidecar cleanup AFTER the lock is held: deleting index.db.new
        // while another build is writing it would destroy its in-progress work.
        let new = db_new_path(root);
        if new.exists() {
            let _ = fs::remove_file(&new);
        }
        file.seek(SeekFrom::Start(0))?;
        file.set_len(0)?;
        writeln!(file, "{} {}", std::process::id(), env!("CARGO_PKG_VERSION"))?;
        Ok(BuildLock { file })
    }
}

/// Open (or create) the lock file, rejecting symlinks and hard-linked files
/// before and after the open.
fn open_lock_file(lock_path: &Path) -> Result<fs::File> {
    reject_symlink(lock_path, "build lock")?;
    let file = match fs::symlink_metadata(lock_path) {
        Ok(meta) if !meta.is_file() || link_count(&meta) > 1 => {
            return Err(unsafe_lock(lock_path));
        }
        Ok(_) => fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock_path)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            match fs::OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .open(lock_path)
            {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(lock_path)?,
                Err(e) => return Err(e.into()),
            }
        }
        Err(e) => return Err(e.into()),
    };
    reject_symlink(lock_path, "build lock")?;
    let meta = file.metadata()?;
    if !meta.is_file() || link_count(&meta) > 1 {
        return Err(unsafe_lock(lock_path));
    }
    Ok(file)
}

impl Drop for BuildLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Open the index for querying with the full skew/corruption gate chain:
/// missing -> exit 1 (CUJ7), corrupt -> exit 3 (CUJ3), skew -> exit 5 (CUJ5).
/// Amendment 11a: no `PRAGMA quick_check` here — it is a full B-tree scan,
/// linear in DB size, on every open (CLI and per-MCP-tool-call). Corruption
/// still exits 3: SQLITE_NOTADB surfaces on the first statement against a
/// non-SQLite file, SQLITE_CORRUPT when a query hits a malformed page, and
/// both map to the contract message via `AwError::from(rusqlite::Error)`.
/// Deep integrity remains `aw doctor` (`PRAGMA integrity_check`).
pub fn open_for_query(root: &Path) -> Result<Connection> {
    reject_symlink(&aw_dir(root), ".aw directory")?;
    let path = db_path(root);
    reject_symlink(&path, "index.db")?;
    if !path.exists() {
        return Err(AwError::NoResults(format!(
            "no .aw/index.db under {} — run aw build",
            root.display()
        )));
    }
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    check_schema_version(&conn)?;
    Ok(conn)
}

pub fn check_schema_version(conn: &Connection) -> Result<()> {
    let have: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if have != SCHEMA_VERSION {
        return Err(AwError::SchemaMismatch {
            have,
            want: SCHEMA_VERSION,
        });
    }
    Ok(())
}

/// Fresh DB at `.aw/index.db.new` with tables, FTS, and user_version stamped.
/// Secondary indexes are created later by `create_secondary_indexes` (after
/// the bulk insert) so each index is sorted once instead of maintained per row.
pub fn create_fresh(root: &Path) -> Result<Connection> {
    ensure_aw_gitignore(root)?;
    let path = db_new_path(root);
    let _ = fs::remove_file(&path);
    reject_symlink(&path, "index.db.new")?;
    let conn = Connection::open(&path)?;
    // Build-only pragmas (A2): `.new` is disposable until publish_new_index
    // renames it, so this connection drops the rollback journal (no double
    // writes) and per-commit fsyncs. publish_new_index fsyncs the finished file
    // before the atomic rename, so the published index is durable.
    conn.execute_batch(
        "PRAGMA journal_mode = OFF;
         PRAGMA synchronous = OFF;
         PRAGMA locking_mode = EXCLUSIVE;
         PRAGMA cache_size = -32768;",
    )?;
    conn.execute_batch(include_str!("../schema.sql"))?;
    // The version contract lives here alone: schema.sql declares no PRAGMA,
    // and this pragma_update stamps the compiled SCHEMA_VERSION.
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(conn)
}

/// Create the secondary indexes after every bulk insert. The final schema is
/// identical to the historical schema.sql layout; only the creation order moves.
pub fn create_secondary_indexes(conn: &Connection) -> Result<()> {
    conn.execute_batch(include_str!("../schema_indexes.sql"))?;
    Ok(())
}

/// Fsync the finished `.new` index and atomically publish it onto `index.db`.
/// The build connection must already be closed (SQLite flushes its pager on
/// close). A crash before the rename leaves only a disposable `.new`; the next
/// build's lock holder removes it, and the previous `index.db` stays readable.
pub fn publish_new_index(root: &Path) -> Result<()> {
    let new = db_new_path(root);
    File::open(&new)?.sync_all()?;
    fs::rename(&new, db_path(root))?;
    // Best-effort directory fsync: makes the rename durable. If it fails, a
    // crash can revert to the previous index, never expose a missing one.
    if let Ok(dir) = File::open(aw_dir(root)) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Fast path gate: a usable previous index under the SAME schema AND grammar
/// versions (any mismatch disables the fast path for the whole build).
pub fn probe_old(root: &Path) -> Result<Option<Connection>> {
    reject_symlink(&aw_dir(root), ".aw directory")?;
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
/// column, both indexes, and all four manifest versions.
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
    true
}

/// True when `sqlite_master` holds an object of `kind` named `name`.
fn object_exists(conn: &Connection, kind: &str, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = ?1 AND name = ?2",
        rusqlite::params![kind, name],
        |r| r.get::<_, i64>(0),
    )
    .unwrap_or(0)
        > 0
}

/// True when `table` has a column named `column`.
fn column_present(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let present: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
        rusqlite::params![table, column],
        |r| r.get(0),
    )?;
    Ok(present > 0)
}

/// (rel -> (mtime, size, content_hash)) of the previous index's files.
pub fn old_files(old: &Connection) -> Result<HashMap<String, (i64, i64, i64)>> {
    let mut stmt = old.prepare("SELECT path, mtime, size, content_hash FROM files")?;
    let rows = stmt
        .query_map(rusqlite::params![], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .map(|(p, m, s, h)| (p, (m, s, h)))
        .collect())
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
                 JOIN symbols s ON s.id = e.src_id JOIN files f ON f.id = s.file_id
                 JOIN symbols ds ON ds.id = e.dst_id JOIN files df ON df.id = ds.file_id
                 WHERE df.path IN ({placeholders})"
            );
            let mut stmt = old.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
                r.get::<_, String>(0)
            })?;
            for r in rows {
                let p = r?;
                if !dirty.contains(&p) {
                    new_paths.insert(p);
                }
            }
            Ok(())
        })?;
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
        let mut stmt = old.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
            r.get::<_, String>(0)
        })?;
        for r in rows {
            out.insert(r?);
        }
        Ok(())
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
    let segments: Vec<String> = new_qualnames
        .iter()
        .map(|q| glob_literal(q.rsplit('.').next().unwrap_or(q)))
        .collect();
    let mut out = HashSet::new();
    for_each_chunk(old, &segments, |old, _placeholders, chunk| {
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
        Ok(())
    })?;
    Ok(out)
}

/// Collect the `SELECT DISTINCT f.path` rows of one chunked gap query into
/// `out`, so the filter stays inside SQLite instead of materializing every
/// edge row into memory.
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

/// Insert unresolved edge sites for the fresh rows (schema v6, Amendment 19).
pub fn insert_unresolved_calls(
    conn: &Connection,
    rows: &[(i64, &str, EdgeSiteKind, usize)],
) -> Result<()> {
    if rows.iter().any(|(_, _, _, site_line)| *site_line == 0) {
        return Err(crate::error::AwError::Internal(
            "unresolved edge row has site_line 0".to_string(),
        ));
    }
    let mut stmt = conn.prepare(
        "INSERT INTO unresolved_calls(file_id, dst_name, kind, site_line) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (file_id, name, kind, site_line) in rows {
        stmt.execute(rusqlite::params![
            file_id,
            name,
            kind.as_str(),
            *site_line as i64
        ])?;
    }
    Ok(())
}

/// Symbols + inherits pairs of the carried set, for resolution tables.
pub fn load_carried(old: &Connection, rels: &HashSet<String>) -> Result<CarriedIndex> {
    if rels.is_empty() {
        return Ok(CarriedIndex {
            symbols: Vec::new(),
            inherits: Vec::new(),
        });
    }
    let mut rel_params: Vec<&str> = rels.iter().map(String::as_str).collect();
    rel_params.sort_unstable();
    Ok(CarriedIndex {
        symbols: load_carried_symbols(old, &rel_params)?,
        inherits: load_carried_inherits(old, &rel_params)?,
    })
}

fn load_carried_symbols(old: &Connection, rel_params: &[&str]) -> Result<Vec<CarriedSymbol>> {
    let mut symbols = Vec::new();
    // Bound the read to the carried files instead of scanning every symbol.
    // Chunked placeholders match the inherits query below; the final sort by
    // symbol id restores the old index order across chunks (resolution tables
    // are order-sensitive for duplicate qualnames).
    let mut rows: Vec<(i64, String, String, String, String)> = Vec::new();
    for_each_chunk(old, rel_params, |old, placeholders, chunk| {
        let mut stmt = old.prepare(&format!(
            "SELECT s.id, s.kind, s.name, s.qualname, f.path
             FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE f.path IN ({placeholders})"
        ))?;
        let part = stmt.query_map(rusqlite::params_from_iter(chunk.iter().copied()), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        for row in part {
            rows.push(row?);
        }
        Ok(())
    })?;
    rows.sort_by_key(|row| row.0);
    for (id, kind, name, qualname, path) in rows {
        // A carried trait/interface is a valid impl target on the fast path;
        // dropping it made incremental Rust impls permanently unresolved
        // (R12 Critical). Unknown spellings stay dropped, as before.
        let Some(kind) = SymbolKind::parse(&kind) else {
            continue;
        };
        let (module, _) = crate::extract::module_path_for(&path);
        symbols.push(CarriedSymbol {
            row_id: id,
            kind,
            name,
            qualname,
            module,
        });
    }
    Ok(symbols)
}

fn load_carried_inherits(old: &Connection, rel_params: &[&str]) -> Result<Vec<(String, String)>> {
    let mut inherits = Vec::new();
    for_each_chunk(old, rel_params, |old, placeholders, chunk| {
        let mut stmt = old.prepare(&format!(
            "SELECT s.qualname, e.dst_name
             FROM edges e JOIN symbols s ON s.id = e.src_id
             JOIN files f ON f.id = s.file_id
             WHERE e.kind = '{}' AND e.dst_name IS NOT NULL
               AND f.path IN ({placeholders})",
            EdgeKind::Inherits.as_str()
        ))?;
        let pairs = stmt.query_map(rusqlite::params_from_iter(chunk.iter().copied()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for p in pairs {
            inherits.push(p?);
        }
        Ok(())
    })?;
    Ok(inherits)
}

/// Copy rows for files matching the fast-path keys. `unchanged` rows carry
/// (rel, old_mtime, size, current_mtime): the SELECT identifies the old row by
/// (path, old_mtime, size); hash-carried rows (Amendment 9) then get their
/// stored mtime refreshed to the fresh checkout's stat so the staleness
/// contract stays truthful. Attaches the old DB itself (SQLite ATTACH takes no
/// bound parameters).
/// Amendment 11b: the caller owns the build transaction — no BEGIN/COMMIT here.
pub fn carry_forward_unchanged(
    new: &Connection,
    root: &Path,
    unchanged: &[(String, i64, i64, i64)],
) -> Result<()> {
    let old_path = db_path(root);
    let quoted = old_path.to_string_lossy().replace('\'', "''");
    new.execute_batch(&format!("ATTACH DATABASE '{quoted}' AS olddb"))?;
    {
        let mut stmt = new.prepare(
            "INSERT INTO files(id, path, size, mtime, content_hash, parse_status, error_nodes, unresolved)
             SELECT id, path, size, mtime, content_hash, parse_status, error_nodes, unresolved
             FROM olddb.files WHERE path = ?1 AND mtime = ?2 AND size = ?3",
        )?;
        let mut refresh_mtime = new.prepare("UPDATE files SET mtime = ?1 WHERE path = ?2")?;
        for (rel, mtime, size, current_mtime) in unchanged {
            let inserted = stmt.execute(rusqlite::params![rel, mtime, size])?;
            if inserted > 0 && current_mtime != mtime {
                refresh_mtime.execute(rusqlite::params![current_mtime, rel])?;
            }
        }
    }
    new.execute_batch(
        "INSERT INTO symbols(id, kind, name, qualname, file_id, file_path, line_start, line_end, signature, is_test)
         SELECT id, kind, name, qualname, file_id, file_path, line_start, line_end, signature, is_test
         FROM olddb.symbols WHERE file_id IN (SELECT id FROM files);
         INSERT INTO edges(id, src_id, dst_id, dst_name, kind, confidence, site_line)
         SELECT id, src_id, dst_id, dst_name, kind, confidence, site_line
         FROM olddb.edges
         WHERE src_id IN (SELECT id FROM symbols)
           AND (dst_id IS NULL OR dst_id IN (SELECT id FROM symbols));
         INSERT INTO unresolved_calls(file_id, dst_name, kind, site_line)
         SELECT file_id, dst_name, kind, site_line FROM olddb.unresolved_calls
         WHERE file_id IN (SELECT id FROM files);",
    )?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct FilesRow {
    pub rel: String,
    pub size: i64,
    pub mtime: i64,
    pub content_hash: i64,
    pub parse_status: &'static str,
    pub error_nodes: usize,
    /// Per-file unresolved-callee count (filled after resolve()).
    pub unresolved: usize,
}

/// Amendment 11b: caller holds the build transaction; 500-row prepared
/// batches remain (statement reuse), the per-chunk BEGIN/COMMIT fsyncs do not.
pub fn insert_files(conn: &Connection, rows: &[FilesRow]) -> Result<Vec<i64>> {
    let mut ids = Vec::with_capacity(rows.len());
    let mut stmt = conn.prepare(
        "INSERT INTO files(path, size, mtime, content_hash, parse_status, error_nodes, unresolved)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for r in rows {
        stmt.execute(rusqlite::params![
            r.rel,
            r.size,
            r.mtime,
            r.content_hash,
            r.parse_status,
            r.error_nodes as i64,
            r.unresolved as i64
        ])?;
        ids.push(conn.last_insert_rowid());
    }
    Ok(ids)
}

pub fn insert_symbols(
    conn: &Connection,
    symbols: &[&NewSymbol],
    file_ids: &[i64],
    file_paths: &[String],
) -> Result<Vec<i64>> {
    let mut ids = Vec::with_capacity(symbols.len());
    let mut stmt = conn.prepare(
        "INSERT INTO symbols(kind, name, qualname, file_id, file_path, line_start, line_end, signature, is_test)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )?;
    for s in symbols {
        stmt.execute(rusqlite::params![
            s.kind.as_str(),
            s.name,
            s.qualname,
            file_ids[s.file_idx],
            file_paths[s.file_idx].as_str(),
            s.line_start as i64,
            s.line_end as i64,
            s.signature,
            i64::from(s.is_test)
        ])?;
        ids.push(conn.last_insert_rowid());
    }
    Ok(ids)
}

/// Fully resolved edge row: endpoint ids already mapped (fresh -> inserted id,
/// carried -> previous row id).
#[derive(Debug)]
pub struct EdgeRow<'a> {
    pub src_id: i64,
    pub dst_id: Option<i64>,
    pub dst_name: &'a str,
    pub kind: &'static str,
    pub confidence: Confidence,
    pub site_line: usize,
}

pub fn insert_edge_rows(conn: &Connection, rows: &[EdgeRow<'_>]) -> Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO edges(src_id, dst_id, dst_name, kind, confidence, site_line)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for e in rows {
        stmt.execute(rusqlite::params![
            e.src_id,
            e.dst_id,
            e.dst_name,
            e.kind,
            e.confidence.as_str(),
            e.site_line as i64
        ])?;
    }
    Ok(())
}

/// Populate the FTS5 symbol-search index from the fully-inserted symbols
/// table (Amendment 12b): one pass covering fresh AND carried rows, called
/// inside the build transaction after `insert_edge_rows`. Amendment 14
/// widens the indexed text with path and signature tokens — the two
/// haystacks grep already matches and a name-only index could not.
/// FTS text for one symbol (Amendment 12b/14/R7): qualname and name repeat so
/// bm25 ranks identity matches above signature-only matches now that signatures
/// carry full parameter lists. Shared by the SELECT and in-memory paths so the
/// indexed bytes cannot drift.
fn fts_text(qualname: &str, name: &str, path: &str, signature: &str) -> String {
    format!(
        "{q} {q} {n} {n} {p} {s}",
        q = crate::extract::split_ident(qualname),
        n = crate::extract::split_ident(name),
        p = crate::extract::split_ident(path),
        s = crate::extract::split_ident(signature)
    )
}

pub fn populate_symbols_fts(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.qualname, s.name, f.path, s.signature
         FROM symbols s JOIN files f ON f.id = s.file_id",
    )?;
    let rows: Vec<(i64, String, String, String, String)> = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut ins = conn.prepare("INSERT INTO symbols_fts(rowid, text) VALUES (?1, ?2)")?;
    for (id, qualname, name, path, signature) in rows {
        ins.execute(rusqlite::params![
            id,
            fts_text(&qualname, &name, &path, &signature)
        ])?;
    }
    Ok(())
}

/// Populate FTS from in-memory rows (force path): one row is
/// `(symbol id, qualname, name, file path, signature)`. Avoids the SELECT +
/// JOIN over every inserted symbol. The indexed bytes match `fts_text` exactly.
pub fn populate_symbols_fts_rows(
    conn: &Connection,
    rows: &[(i64, &str, &str, &str, &str)],
) -> Result<()> {
    let mut ins = conn.prepare("INSERT INTO symbols_fts(rowid, text) VALUES (?1, ?2)")?;
    for &(id, qualname, name, path, signature) in rows {
        ins.execute(rusqlite::params![
            id,
            fts_text(qualname, name, path, signature)
        ])?;
    }
    Ok(())
}

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// Read one meta-manifest value (built_at, source_rev, counters).
pub fn meta_get(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .ok()
}

/// Meta key names — the single vocabulary shared by the build-time writers
/// (`build.rs::write_meta_identity` and `build.rs::write_meta_counts`), the
/// carry-forward gate (`schema_compatible`), and the display readers
/// (`doctor`, `insights`), so a new key is registered under one name in all
/// definitions and they cannot drift apart. The `*_SRC_FILES` counters have
/// no `meta_counters` fallback: their readers (`query/impact.rs`) fall back
/// to an edge scan instead.
pub(crate) mod meta_keys {
    // Identity and provenance keys written by `write_meta_identity`.
    pub const AW_VERSION: &str = "aw_version";
    pub const GRAMMAR_VERSION: &str = "grammar_version";
    pub const SIGNATURE_VERSION: &str = "signature_version";
    pub const METRICS_VERSION: &str = "metrics_version";
    pub const BUILT_AT: &str = "built_at";
    pub const SOURCE_REV: &str = "source_rev";
    pub const CALLABLE_COUNT: &str = "callable_count";
    pub const FILE_COUNT: &str = "file_count";
    pub const SYMBOL_COUNT: &str = "symbol_count";
    pub const EDGE_COUNT: &str = "edge_count";
    pub const PARSE_ERROR_COUNT: &str = "parse_error_count";
    pub const EDGE_CALLS: &str = "edge_calls";
    pub const EDGE_IMPORTS: &str = "edge_imports";
    pub const EDGE_INHERITS: &str = "edge_inherits";
    pub const UNRESOLVED_CALLEE_COUNT: &str = "unresolved_callee_count";
    pub const EDGE_CALLS_SRC_FILES: &str = "edge_calls_src_files";
    pub const EDGE_IMPORTS_SRC_FILES: &str = "edge_imports_src_files";
    pub const EDGE_INHERITS_SRC_FILES: &str = "edge_inherits_src_files";
    pub const EDGE_ALL_SRC_FILES: &str = "edge_all_src_files";
}

/// Build-time meta counters with their fallback SQL (R7 L4): one (key, SQL)
/// pair per counter, shared by `aw stats` (insights) and the no-op build
/// line (build), so a new counter is registered once and both readers stay
/// in sync. The fallback COUNT keeps pre-counter indexes working. Kind
/// filters render through the enum so the vocabularies stay single-sourced.
fn meta_counters() -> Vec<(&'static str, String)> {
    let edge_count = |kind: EdgeKind| -> String {
        format!(
            "SELECT COUNT(*) FROM edges WHERE kind = '{}'",
            kind.as_str()
        )
    };
    vec![
        (
            meta_keys::FILE_COUNT,
            "SELECT COUNT(*) FROM files".to_string(),
        ),
        (
            meta_keys::SYMBOL_COUNT,
            "SELECT COUNT(*) FROM symbols".to_string(),
        ),
        (
            meta_keys::EDGE_COUNT,
            "SELECT COUNT(*) FROM edges".to_string(),
        ),
        (meta_keys::EDGE_CALLS, edge_count(EdgeKind::Calls)),
        (meta_keys::EDGE_IMPORTS, edge_count(EdgeKind::Imports)),
        (meta_keys::EDGE_INHERITS, edge_count(EdgeKind::Inherits)),
        (
            meta_keys::PARSE_ERROR_COUNT,
            "SELECT COALESCE(SUM(parse_status != 'ok'), 0) FROM files".to_string(),
        ),
        (
            meta_keys::UNRESOLVED_CALLEE_COUNT,
            "SELECT COALESCE(SUM(unresolved), 0) FROM files".to_string(),
        ),
    ]
}

/// Counter value for `key`: the build-time meta entry when present, else the
/// registered fallback COUNT (pre-counter index).
pub(crate) fn meta_count(conn: &Connection, key: &str) -> Result<i64> {
    if let Some(v) = meta_get(conn, key).and_then(|v| v.parse::<i64>().ok()) {
        return Ok(v);
    }
    let counters = meta_counters();
    let sql = counters
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, sql)| sql.as_str())
        .unwrap_or("SELECT 0");
    Ok(conn.query_row(sql, [], |r| r.get(0))?)
}

/// Probe the source repo's git revision with a `-dirty` suffix on a dirty tree.
pub fn source_rev(root: &Path) -> String {
    let head = std::process::Command::new("git")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    let Some(mut rev) = head else {
        return "unknown".to_string();
    };
    let dirty = std::process::Command::new("git")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| !o.stdout.is_empty());
    if dirty {
        rev.push_str("-dirty");
    }
    rev
}

/// Nanosecond-precision mtime from already-captured metadata; `None` when
/// the platform cannot report it. The single copy of the fast-path/staleness
/// arithmetic: the walk stamps `WalkedFile.mtime` through this function and
/// every later comparison goes through `stat_mtime`, so both sides of a
/// comparison share one implementation and cannot drift.
pub fn metadata_mtime(metadata: &std::fs::Metadata) -> Option<i64> {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
}

/// Nanosecond-precision stat mtime as i64 (fast-path and staleness key).
/// Nanoseconds close the same-second, same-size edit hole that silently kept
/// a stale index.
pub fn stat_mtime(path: &Path) -> Option<i64> {
    fs::metadata(path).ok().and_then(|m| metadata_mtime(&m))
}

/// stat size as i64.
pub fn stat_size(path: &Path) -> Option<i64> {
    fs::metadata(path).ok().map(|m| m.len() as i64)
}

// ---------------------------------------------------------------------------
// Project memory store (`.aw/memory.db`) — companion design:
// docs/designs/2026-11-05_agent-weave-memory-retrieval.md. All memory.db
// lifecycle lives here so the write-confinement allowlist stays unchanged.
// Pragma regime (deliberately NOT the index build's OFF/OFF/EXCLUSIVE — the
// design's Storage note): default rollback journal, synchronous = FULL, and a
// busy_timeout, because memory is primary data with concurrent agent writers.
// ---------------------------------------------------------------------------

use rusqlite::OptionalExtension;

/// Compiled memory schema version; gates `.aw/memory.db` like SCHEMA_VERSION
/// gates the index, with a diverging policy: older-but-structurally-complete
/// stores open read-write (memory cannot be rebuilt), only newer-than-binary
/// or missing-required-structure exits 5.
pub const MEMORY_SCHEMA_VERSION: i32 = 1;
const MEMORY_BUSY_TIMEOUT_MS: u64 = 5000;
/// Recovery hint shared by every memory-store failure message (corruption
/// and both schema-gate shapes) so the wording cannot drift apart.
const MEMORY_RESTORE_HINT: &str = "restore from an aw mem export backup or remove .aw/memory.db";

pub fn memory_db_path(root: &Path) -> PathBuf {
    aw_dir(root).join("memory.db")
}

/// Required structure manifest, checked at open via pragma_table_info — the
/// probe_old pattern. "Structurally complete" is this list, not an adjective.
const MEMORY_REQUIRED: [(&str, &str); 15] = [
    ("memories", "key"),
    ("memories", "rev"),
    ("memories", "kind"),
    ("memories", "scope"),
    ("memories", "session"),
    ("memories", "body"),
    ("memories", "tags"),
    ("memories", "refs"),
    ("memories", "content_hash"),
    ("memories", "source"),
    ("memories", "created_at"),
    ("memories", "expires_at"),
    ("memories", "superseded_by"),
    ("memories", "superseded_at"),
    ("memories", "id"),
];

/// Map a rusqlite failure to the memory error contract: corruption -> exit 3,
/// busy after the timeout -> exit 4, everything else internal (exit 6).
fn map_memory_err(e: rusqlite::Error) -> AwError {
    if let rusqlite::Error::SqliteFailure(ffi, _) = &e {
        if is_sqlite_corruption(&e) {
            return AwError::CorruptMemory(format!(
                "memory.db failed integrity check — {MEMORY_RESTORE_HINT}"
            ));
        }
        // Both lock errors are the documented "retry" outcome (exit 4):
        // SQLITE_BUSY means another writer holds the lock, SQLITE_LOCKED
        // means the connection hit an internal lock conflict. Mapping only
        // BUSY leaked the second shape as an internal error (exit 6).
        if matches!(
            ffi.code,
            rusqlite::ffi::ErrorCode::DatabaseBusy | rusqlite::ffi::ErrorCode::DatabaseLocked
        ) {
            return AwError::MemoryLocked;
        }
    }
    AwError::Internal(format!("memory database error: {e}"))
}

fn memory_manifest_complete(conn: &Connection) -> Result<bool> {
    for (table, column) in MEMORY_REQUIRED {
        if !column_present(conn, table, column).map_err(map_memory_err)? {
            return Ok(false);
        }
    }
    let fts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'memories_fts'",
            [],
            |r| r.get(0),
        )
        .map_err(map_memory_err)?;
    Ok(fts > 0)
}

/// Two distinct exit-5 messages (Storage section): newer than the binary vs
/// older but failing the compiled manifest. Memory cannot be rebuilt, so both
/// point at the `aw mem export` backup, never at `aw build`.
fn memory_version_gate(conn: &Connection) -> Result<()> {
    let have: i32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(map_memory_err)?;
    if have > MEMORY_SCHEMA_VERSION {
        return Err(AwError::MemorySchema {
            detail: format!(
                "memory.db schema v{have} is newer than the binary expects (v{MEMORY_SCHEMA_VERSION}) — {MEMORY_RESTORE_HINT}"
            ),
        });
    }
    if !memory_manifest_complete(conn)? {
        return Err(AwError::MemorySchema {
            detail: format!(
                "memory.db schema v{have} is missing required structure for this binary — {MEMORY_RESTORE_HINT}"
            ),
        });
    }
    Ok(())
}

fn memory_pragmas(conn: &Connection) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_millis(MEMORY_BUSY_TIMEOUT_MS))
        .map_err(map_memory_err)?;
    // synchronous = FULL is the rollback-journal default; stated explicitly so
    // the durability contract survives a future default change.
    conn.execute_batch("PRAGMA synchronous = FULL;")
        .map_err(map_memory_err)?;
    Ok(())
}

fn resolve_default_home_aw_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| {
            AwError::Internal(
                "could not determine home directory (HOME/USERPROFILE unset)".to_string(),
            )
        })?;
    Ok(PathBuf::from(home).join(".aw"))
}

/// Directory for the cross-project global memory store.
/// Reads $AW_GLOBAL_DIR, falling back to $HOME/.aw or %USERPROFILE%/.aw.
/// Refuses symlinks.
pub fn global_aw_dir() -> Result<PathBuf> {
    let path = if let Ok(val) = std::env::var("AW_GLOBAL_DIR") {
        if !val.trim().is_empty() {
            PathBuf::from(val)
        } else {
            resolve_default_home_aw_dir()?
        }
    } else {
        resolve_default_home_aw_dir()?
    };
    reject_symlink(&path, "global .aw directory")?;
    if let Ok(meta) = fs::symlink_metadata(&path) {
        if !meta.is_dir() {
            return Err(AwError::Internal(format!(
                "global .aw exists but is not a directory: {}",
                path.display()
            )));
        }
    }
    Ok(path)
}

fn ensure_global_aw_dir(path: &Path) -> Result<()> {
    reject_symlink(path, "global .aw directory")?;
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_dir() => Err(AwError::Internal(format!(
            "global .aw exists but is not a directory: {}",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Err(create_err) = fs::create_dir_all(path) {
                if create_err.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(create_err.into());
                }
            }
            reject_symlink(path, "global .aw directory")?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

pub fn global_memory_db_path() -> Result<PathBuf> {
    Ok(global_aw_dir()?.join("memory.db"))
}

fn global_memory_store_path() -> Result<PathBuf> {
    let path = global_memory_db_path()?;
    reject_symlink(&path, "global memory.db")?;
    Ok(path)
}

fn no_global_memory_error(path: &Path) -> AwError {
    AwError::NoMemory(format!(
        "no {} — write with aw mem write --global",
        path.display()
    ))
}

/// Open the memory store for writing, creating `.aw/memory.db` (schema v1)
/// when absent. An existing store must pass the version gate (read-forward:
/// older structurally complete stores open read-write).
pub fn open_memory_store(root: &Path) -> Result<Connection> {
    let path = memory_store_path(root)?;
    if !path.exists() {
        ensure_aw_gitignore(root)?;
    }
    open_memory_conn_path(&path, true, || no_memory_error(root))
}

/// Open an existing memory store for mutation WITHOUT creating one: a delete
/// against a missing store must exit 1, never conjure an empty database.
pub fn open_memory_mutation(root: &Path) -> Result<Connection> {
    let path = memory_store_path(root)?;
    open_memory_conn_path(&path, false, || no_memory_error(root))
}

pub fn open_global_memory_store() -> Result<Connection> {
    let dir = global_aw_dir()?;
    ensure_global_aw_dir(&dir)?;
    let path = global_memory_store_path()?;
    open_memory_conn_path(&path, true, || no_global_memory_error(&path))
}

pub fn open_global_memory_mutation() -> Result<Connection> {
    let path = global_memory_store_path()?;
    open_memory_conn_path(&path, false, || no_global_memory_error(&path))
}

fn memory_store_path(root: &Path) -> Result<PathBuf> {
    reject_symlink(&aw_dir(root), ".aw directory")?;
    let path = memory_db_path(root);
    reject_symlink(&path, "memory.db")?;
    Ok(path)
}

pub fn no_memory_error(root: &Path) -> AwError {
    AwError::NoMemory(format!(
        "no .aw/memory.db under {} — write with aw mem write",
        root.display()
    ))
}

fn open_memory_conn_path(
    path: &Path,
    create: bool,
    missing_err: impl FnOnce() -> AwError,
) -> Result<Connection> {
    let existed = path.exists();
    if !existed && !create {
        return Err(missing_err());
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .map_err(map_memory_err)?;
    memory_pragmas(&conn)?;
    if !memory_manifest_complete(&conn)? {
        create_memory_schema(&conn)?;
    }
    memory_version_gate(&conn)?;
    Ok(conn)
}

/// Create the memory schema under SQLite's write lock. Creation race (two
/// first-time writers, or a writer that sees the file another process just
/// created): serializing the manifest check, DDL, and version stamp prevents
/// the loser from seeing a half-created schema (exit 5) or a raw
/// "already exists" (exit 6).
fn create_memory_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(map_memory_err)?;
    if !memory_manifest_complete(conn)? {
        match conn.execute_batch(include_str!("../schema_memory.sql")) {
            Ok(()) => {
                conn.pragma_update(None, "user_version", MEMORY_SCHEMA_VERSION)
                    .map_err(map_memory_err)?;
                conn.execute_batch("COMMIT").map_err(map_memory_err)?;
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                // A partial legacy schema stays for the version gate to
                // diagnose as exit 5; anything else is a real error.
                if !is_already_exists(&e) {
                    return Err(map_memory_err(e));
                }
            }
        }
    } else {
        conn.execute_batch("COMMIT").map_err(map_memory_err)?;
    }
    Ok(())
}

fn open_memory_query_path(
    path: &Path,
    missing_err: impl FnOnce() -> AwError,
) -> Result<Connection> {
    if !path.exists() {
        return Err(missing_err());
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(map_memory_err)?;
    memory_pragmas(&conn)?;
    memory_version_gate(&conn)?;
    Ok(conn)
}

/// Open the memory store read-only for the read commands. Missing -> exit 1
/// (no-memory), corrupt -> exit 3, skew -> exit 5 — the shared gate chain.
pub fn open_memory_query(root: &Path) -> Result<Connection> {
    let path = memory_store_path(root)?;
    open_memory_query_path(&path, || no_memory_error(root))
}

pub fn open_global_memory_query() -> Result<Connection> {
    let path = global_memory_store_path()?;
    open_memory_query_path(&path, || no_global_memory_error(&path))
}

/// Unix seconds (writes stamp created_at/superseded_at).
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// One memory note row as returned to the presentation layer. `superseded_by`
/// is read only for the `--include-superseded` listing tag; `superseded_at`
/// stays internal (the gc retention clock).
#[derive(Debug, Clone)]
pub struct MemRow {
    pub id: i64,
    pub key: String,
    pub rev: i64,
    pub kind: String,
    pub scope: String,
    pub session: String,
    pub body: String,
    pub tags: String,
    pub refs: String,
    pub source: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub superseded_by: Option<i64>,
}

const MEM_ROW_SQL: &str =
    "id, key, rev, kind, scope, session, body, tags, refs, source, created_at, expires_at, superseded_by";

fn mem_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<MemRow> {
    Ok(MemRow {
        id: r.get(0)?,
        key: r.get(1)?,
        rev: r.get(2)?,
        kind: r.get(3)?,
        scope: r.get(4)?,
        session: r.get(5)?,
        body: r.get(6)?,
        tags: r.get(7)?,
        refs: r.get(8)?,
        source: r.get(9)?,
        created_at: r.get(10)?,
        expires_at: r.get(11)?,
        superseded_by: r.get(12)?,
    })
}

/// Validated note ready for the write transaction (built by the service layer
/// through `memory::normalize_note`). `expires_at` is 0 for durable notes and
/// the absolute unix deadline for session notes; `supersedes` names a
/// different key whose current note this write replaces.
pub struct MemNoteInput<'a> {
    pub key: &'a str,
    pub kind: &'a str,
    pub tags: &'a str,
    pub refs: &'a str,
    pub scope: &'a str,
    pub session: &'a str,
    pub body: &'a str,
    pub source: &'a str,
    pub expires_at: i64,
    pub supersedes: Option<&'a str>,
}

/// Write outcome: a new revision (with the superseded rev, when any) or an
/// idempotent duplicate (the existing current rev, unchanged).
#[derive(Debug)]
pub enum MemWriteOutcome {
    Written {
        rev: i64,
        superseded_rev: Option<i64>,
    },
    Duplicate {
        rev: i64,
    },
}

/// `execute_batch` reports statement failures as `SqlInputError`; `execute`
/// reports them as `SqliteFailure`. Both shapes must classify "already
/// exists" so a concurrent creator can fall through to the version gate.
fn is_already_exists(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(_, Some(msg)) | rusqlite::Error::SqlInputError { msg, .. }
            if msg.contains("already exists")
    )
}

fn is_unique_violation(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(ffi, _)
            if ffi.code == rusqlite::ffi::ErrorCode::ConstraintViolation
    )
}

/// Insert one note under BEGIN IMMEDIATE: the write lock is taken BEFORE the
/// current rev is read and held across read-compute-insert, so racing writers
/// serialize on busy_timeout (or exit 4) and never compute the same rev. The
/// FTS delta (delete superseded rowid, insert the new one) runs in the same
/// transaction. A retried write with identical content is a successful no-op.
pub fn memory_write(
    conn: &mut Connection,
    note: &MemNoteInput<'_>,
    now: i64,
) -> Result<MemWriteOutcome> {
    // One in-transaction retry on a UNIQUE(key, rev) failure (a defect by
    // construction — rev is computed inside the lock — but never exit 2/5).
    let mut last_error = None;
    for _ in 0..2 {
        match try_write_revision(conn, note, now)? {
            WriteAttempt::Written(outcome) => return Ok(outcome),
            WriteAttempt::Retry(e) => last_error = Some(e),
        }
    }
    match last_error {
        Some(e) => Err(map_memory_err(e)),
        None => Err(AwError::Internal(
            "memory write: no attempt completed".to_string(),
        )),
    }
}

enum WriteAttempt {
    Written(MemWriteOutcome),
    Retry(rusqlite::Error),
}

fn try_write_revision(
    conn: &mut Connection,
    note: &MemNoteInput<'_>,
    now: i64,
) -> Result<WriteAttempt> {
    let hash =
        crate::memory::mem_content_hash(note.kind, note.body, note.tags, note.refs, note.scope)
            as i64;
    let fts_text = crate::memory::mem_fts_text(note.key, note.tags, note.body);
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(map_memory_err)?;
    let current = current_revision(&tx, note.key)?;
    if let Some((_, rev, current_hash)) = current {
        if current_hash == hash {
            // Idempotent replay: nothing written, nothing to commit.
            drop(tx);
            return Ok(WriteAttempt::Written(MemWriteOutcome::Duplicate { rev }));
        }
    }
    let new_rev = current.as_ref().map_or(1, |(_, rev, _)| rev + 1);
    if let Err(e) = insert_revision(&tx, note, now, hash, new_rev) {
        if is_unique_violation(&e) {
            return Ok(WriteAttempt::Retry(e));
        }
        return Err(map_memory_err(e));
    }
    let new_id = tx.last_insert_rowid();
    if let Some((old_id, _, _)) = current {
        supersede_current(&tx, new_id, now, old_id)?;
    }
    if let Some(target) = note.supersedes {
        supersede_cross_key(&tx, new_id, now, target)?;
    }
    tx.execute(
        "INSERT INTO memories_fts(rowid, text) VALUES (?1, ?2)",
        rusqlite::params![new_id, fts_text],
    )
    .map_err(map_memory_err)?;
    tx.commit().map_err(map_memory_err)?;
    Ok(WriteAttempt::Written(MemWriteOutcome::Written {
        rev: new_rev,
        superseded_rev: current.map(|(_, rev, _)| rev),
    }))
}

fn insert_revision(
    tx: &rusqlite::Transaction<'_>,
    note: &MemNoteInput<'_>,
    now: i64,
    hash: i64,
    rev: i64,
) -> rusqlite::Result<usize> {
    tx.execute(
        "INSERT INTO memories(key, rev, kind, scope, session, body, tags, refs, content_hash, source, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            note.key,
            rev,
            note.kind,
            note.scope,
            note.session,
            note.body,
            note.tags,
            note.refs,
            hash,
            note.source,
            now,
            note.expires_at,
        ],
    )
}

fn current_revision(tx: &rusqlite::Transaction<'_>, key: &str) -> Result<Option<(i64, i64, i64)>> {
    tx.query_row(
        "SELECT id, rev, content_hash FROM memories WHERE key = ?1 ORDER BY rev DESC LIMIT 1",
        rusqlite::params![key],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
    .map_err(map_memory_err)
}

/// Mark one revision superseded and drop its FTS row (the caller inserts the
/// new revision's FTS row). Shared by same-key and cross-key supersession so
/// both paths apply the same invariant.
fn supersede_current(
    tx: &rusqlite::Transaction<'_>,
    new_id: i64,
    now: i64,
    old_id: i64,
) -> Result<()> {
    tx.execute(
        "UPDATE memories SET superseded_by = ?1, superseded_at = ?2 WHERE id = ?3",
        rusqlite::params![new_id, now, old_id],
    )
    .map_err(map_memory_err)?;
    tx.execute(
        "DELETE FROM memories_fts WHERE rowid = ?1",
        rusqlite::params![old_id],
    )
    .map_err(map_memory_err)?;
    Ok(())
}

/// Cross-key supersession inside the caller's transaction: the target's
/// current revision becomes superseded and its FTS row leaves the index with
/// it. Same invariant as same-key supersession; without the FTS delete `mem
/// query` would return a note that `mem get` calls unknown. An unknown target
/// rolls the whole write back (exit 1). A replayed identical write never
/// reaches here, so the side effects do not re-apply.
fn supersede_cross_key(
    tx: &rusqlite::Transaction<'_>,
    new_id: i64,
    now: i64,
    target: &str,
) -> Result<()> {
    let target_id: Option<i64> = tx
        .query_row(
            "SELECT id FROM memories WHERE key = ?1 AND superseded_by IS NULL LIMIT 1",
            rusqlite::params![target],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_memory_err)?;
    let Some(target_id) = target_id else {
        return Err(AwError::NoResults(format!(
            "unknown memory key '{target}' — nothing superseded, write rolled back"
        )));
    };
    supersede_current(tx, new_id, now, target_id)
}

/// Filter bundle shared by the memory read paths: the optional kind/scope
/// restrictions, the `now` clock, and whether the listing extends to
/// superseded revisions. One struct keeps the four call sites from re-ordering
/// or re-spelling the argument list.
pub struct MemFilter<'a> {
    pub kind: Option<&'a str>,
    pub scope: Option<&'a str>,
    pub now: i64,
    pub include_superseded: bool,
}

/// The one "current row" rule set, stated in Rust: not superseded (unless the
/// listing extends to superseded revisions), not expired, and matching the
/// optional kind/scope filters. `memory_current_rows` compiles the same rules
/// into the `--all` SQL WHERE clause (kind/scope empty-string filters mirror
/// the `None` arms); `service.rs::mem_ranked` applies this predicate to the
/// ranked fetch, so the two modes cannot drift apart.
pub(crate) fn memory_row_is_current(row: &MemRow, filter: &MemFilter<'_>) -> bool {
    if row.superseded_by.is_some() && !filter.include_superseded {
        return false;
    }
    if row.expires_at != 0 && row.expires_at <= filter.now {
        return false;
    }
    if let Some(k) = filter.kind {
        if row.kind != k {
            return false;
        }
    }
    if let Some(s) = filter.scope {
        if row.scope != s {
            return false;
        }
    }
    true
}

/// Current, non-expired rows (the resolver's candidate universe), newest
/// first. `include_superseded` extends the listing with superseded revisions
/// (the `--all --include-superseded` view); expired rows stay invisible in
/// every mode. Empty filter values match everything; all filters are bound
/// parameters, never string interpolation. The WHERE clause mirrors
/// `memory_row_is_current` (same rule set, SQL spelling; a unit test holds
/// the two row-by-row equal).
pub fn memory_current_rows(conn: &Connection, filter: &MemFilter<'_>) -> Result<Vec<MemRow>> {
    // include_superseded=0 keeps only current rows; =1 lists everything
    // (superseded rows included, tagged at render time).
    let superseded_clause = "AND (superseded_by IS NULL OR ?4 = 1)";
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {MEM_ROW_SQL} FROM memories
             WHERE (expires_at = 0 OR expires_at > ?1)
               {superseded_clause}
               AND (?2 = '' OR kind = ?2)
               AND (?3 = '' OR scope = ?3)
             ORDER BY created_at DESC, key ASC, rev DESC"
        ))
        .map_err(map_memory_err)?;
    let rows = stmt
        .query_map(
            rusqlite::params![
                filter.now,
                filter.kind.unwrap_or(""),
                filter.scope.unwrap_or(""),
                i64::from(filter.include_superseded),
            ],
            mem_row,
        )
        .map_err(map_memory_err)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(map_memory_err)
}

/// FTS candidates: (rowid, bm25 rank) best-first, capped. Smaller rank is the
/// better match; the presentation layer inverts via min-max normalization.
pub fn memory_fts_candidates(
    conn: &Connection,
    match_query: &str,
    cap: usize,
) -> Result<Vec<(i64, f64)>> {
    if match_query.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare(
            "SELECT rowid, bm25(memories_fts) FROM memories_fts
             WHERE memories_fts MATCH ?1 ORDER BY rank, rowid LIMIT ?2",
        )
        .map_err(map_memory_err)?;
    let rows = stmt
        .query_map(rusqlite::params![match_query, cap as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        })
        .map_err(map_memory_err)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(map_memory_err)
}

/// Rows by id (candidate fetch; at most CANDIDATE_CAP ids).
pub fn memory_rows_by_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<MemRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = sql_placeholders(ids.len());
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {MEM_ROW_SQL} FROM memories WHERE id IN ({placeholders})"
        ))
        .map_err(map_memory_err)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(ids.iter()), mem_row)
        .map_err(map_memory_err)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(map_memory_err)
}

/// Current row for one exact key plus its total revision count.
pub fn memory_get(conn: &Connection, key: &str) -> Result<Option<(MemRow, i64)>> {
    let row = conn
        .query_row(
            &format!(
                "SELECT {MEM_ROW_SQL} FROM memories
                 WHERE key = ?1 AND superseded_by IS NULL LIMIT 1"
            ),
            rusqlite::params![key],
            mem_row,
        )
        .optional()
        .map_err(map_memory_err)?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.expires_at != 0 && row.expires_at <= unix_now() {
        return Ok(None);
    }
    let revs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE key = ?1",
            rusqlite::params![key],
            |r| r.get(0),
        )
        .map_err(map_memory_err)?;
    Ok(Some((row, revs)))
}

/// Hard-delete every revision of one key plus their FTS rows, one
/// transaction. Returns None for an unknown key (nothing deleted).
pub fn memory_delete(conn: &mut Connection, key: &str) -> Result<Option<i64>> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(map_memory_err)?;
    let ids: Vec<i64> = {
        let mut stmt = tx
            .prepare("SELECT id FROM memories WHERE key = ?1")
            .map_err(map_memory_err)?;
        let rows = stmt
            .query_map(rusqlite::params![key], |r| r.get::<_, i64>(0))
            .map_err(map_memory_err)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(map_memory_err)?
    };
    if ids.is_empty() {
        return Ok(None); // tx dropped -> rollback, nothing written
    }
    delete_memory_ids(&tx, &ids)?;
    let removed = ids.len() as i64;
    tx.commit().map_err(map_memory_err)?;
    Ok(Some(removed))
}

/// Export rows: current revs, newest first, optionally limited to the N most
/// recent by created_at (deterministic; `None` = all).
pub fn memory_export_rows(conn: &Connection, limit: Option<usize>) -> Result<Vec<MemRow>> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {MEM_ROW_SQL} FROM memories
             WHERE superseded_by IS NULL
             ORDER BY created_at DESC, key ASC, rev DESC
             LIMIT ?1"
        ))
        .map_err(map_memory_err)?;
    let rows = stmt
        .query_map(rusqlite::params![limit.map_or(-1, |n| n as i64)], mem_row)
        .map_err(map_memory_err)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(map_memory_err)
}

/// Durable (non-superseded) memory note count. Single source of the
/// tombstone semantics (`superseded_by IS NULL`), shared by `memory_gc` and
/// the `doctor` memory report line; `&Transaction` call sites deref to
/// `&Connection`.
pub fn durable_note_count(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE superseded_by IS NULL",
        [],
        |r| r.get(0),
    )
}

/// gc purge (Phase 2 retention rule), one `BEGIN IMMEDIATE` transaction:
/// expired session notes (read-time invisible already; this reclaims disk)
/// lose their rows AND their FTS rows; superseded revisions whose
/// `superseded_at` is older than the 30-day retention constant are purged
/// (they have no FTS rows — deleted at supersede time — so a rev superseded
/// yesterday but created 40 days ago survives, clocked by supersession, not
/// creation). Returns (expired removed, superseded removed, durable rows left).
pub fn memory_gc(conn: &mut Connection, now: i64) -> Result<(i64, i64, i64)> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(map_memory_err)?;
    let expired_ids: Vec<i64> = {
        let mut stmt = tx
            .prepare("SELECT id FROM memories WHERE expires_at > 0 AND expires_at <= ?1")
            .map_err(map_memory_err)?;
        let rows = stmt
            .query_map(rusqlite::params![now], |r| r.get::<_, i64>(0))
            .map_err(map_memory_err)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(map_memory_err)?
    };
    delete_memory_ids(&tx, &expired_ids)?;
    let retention_horizon = now - crate::memory::RETENTION_DAYS * 86_400;
    let superseded = tx
        .execute(
            "DELETE FROM memories
             WHERE superseded_by IS NOT NULL AND superseded_at > 0 AND superseded_at <= ?1",
            rusqlite::params![retention_horizon],
        )
        .map_err(map_memory_err)? as i64;
    let durable: i64 = durable_note_count(&tx).map_err(map_memory_err)?;
    tx.commit().map_err(map_memory_err)?;
    Ok((expired_ids.len() as i64, superseded, durable))
}

/// Chunked delete: a batch with more ids than SQLite's variable limit must
/// still delete (one placeholder per id would fail to prepare), so ids go
/// through `SQL_CHUNK` batches covering both the row and its FTS entry.
fn delete_memory_ids(tx: &rusqlite::Transaction<'_>, ids: &[i64]) -> Result<()> {
    for_each_chunk(tx, ids, |conn, placeholders, chunk| {
        conn.execute(
            &format!("DELETE FROM memories WHERE id IN ({placeholders})"),
            rusqlite::params_from_iter(chunk.iter()),
        )
        .map_err(map_memory_err)?;
        conn.execute(
            &format!("DELETE FROM memories_fts WHERE rowid IN ({placeholders})"),
            rusqlite::params_from_iter(chunk.iter()),
        )
        .map_err(map_memory_err)?;
        Ok(())
    })
}

/// Per-test temp project root (fresh directory, wiped from any previous run),
/// shared by the db and memory test modules.
#[cfg(test)]
fn temp_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aw-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_db_has_schema_v2() {
        let dir = temp_root("db-test");
        let conn = create_fresh(&dir).unwrap();
        let v: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        drop(conn);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_index_is_exit1_not_panic() {
        let dir = temp_root("db-missing");
        let err = open_for_query(&dir).unwrap_err();
        assert_eq!(err.exit_code(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupted_index_is_exit3() {
        let dir = temp_root("db-corrupt");
        fs::create_dir_all(dir.join(".aw")).unwrap();
        fs::write(db_path(&dir), b"this is not sqlite at all").unwrap();
        let err = open_for_query(&dir).unwrap_err();
        assert_eq!(err.exit_code(), 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn schema_skew_is_exit5() {
        let dir = temp_root("db-skew");
        {
            let conn = create_fresh(&dir).unwrap();
            conn.execute_batch("PRAGMA user_version = 99").unwrap();
        }
        // Build completes by renaming index.db.new onto index.db.
        fs::rename(db_new_path(&dir), db_path(&dir)).unwrap();
        let err = open_for_query(&dir).unwrap_err();
        assert_eq!(err.exit_code(), 5);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_carried_chunks_large_path_sets() {
        let dir = temp_root("db-carried");
        let conn = create_fresh(&dir).unwrap();
        conn.execute(
            "INSERT INTO files(id, path, size, mtime, content_hash, parse_status, error_nodes)
             VALUES (1, 'a.py', 1, 0, 0, 'ok', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO symbols(id, kind, name, qualname, file_id, line_start, line_end)
             VALUES (1, 'class', 'B', 'a.B', 1, 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO edges(id, src_id, dst_id, dst_name, kind, confidence, site_line)
             VALUES (1, 1, NULL, 'Base', 'inherits', 'EXTRACTED', 1)",
            [],
        )
        .unwrap();

        // More paths than the default SQLite variable limit; only a.py exists.
        let mut rels: HashSet<String> = (0..33_000).map(|i| format!("f{i}.py")).collect();
        rels.insert("a.py".to_string());
        let index = load_carried(&conn, &rels).unwrap();
        assert!(index.symbols.iter().any(|s| s.qualname == "a.B"));
        assert_eq!(
            index.inherits,
            vec![("a.B".to_string(), "Base".to_string())]
        );
        drop(conn);
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    fn note<'a>(key: &'a str, body: &'a str, kind: &'a str) -> MemNoteInput<'a> {
        MemNoteInput {
            key,
            kind,
            tags: "",
            refs: "",
            scope: "durable",
            session: "",
            body,
            source: "cli",
            expires_at: 0,
            supersedes: None,
        }
    }

    #[test]
    fn memories_fts_is_pure_function_of_memories() {
        let root = temp_root("mem-purity");
        let mut conn = open_memory_store(&root).unwrap();
        let now = unix_now();
        memory_write(&mut conn, &note("k1", "first body text", "fact"), now).unwrap();
        memory_write(&mut conn, &note("k1", "second body", "fact"), now).unwrap();
        memory_write(&mut conn, &note("k2", "other note", "gotcha"), now).unwrap();
        memory_write(&mut conn, &note("k2", "other note", "gotcha"), now).unwrap(); // duplicate no-op
        memory_delete(&mut conn, "k2").unwrap();

        // Rebuild the derived FTS from the authoritative table and compare.
        let current = memory_current_rows(
            &conn,
            &MemFilter {
                kind: None,
                scope: None,
                now,
                include_superseded: false,
            },
        )
        .unwrap();
        let mut fts: Vec<(i64, String)> = {
            let mut stmt = conn
                .prepare("SELECT rowid, text FROM memories_fts ORDER BY rowid")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
        };
        fts.sort();
        let mut expected: Vec<(i64, String)> = current
            .iter()
            .map(|row| {
                (
                    row.id,
                    crate::memory::mem_fts_text(&row.key, &row.tags, &row.body),
                )
            })
            .collect();
        expected.sort();
        assert_eq!(
            fts, expected,
            "memories_fts must be a pure function of memories"
        );
        drop(conn);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn memory_current_rows_matches_is_current_predicate() {
        // The `--all` SQL filter and the ranked resolver's Rust predicate must
        // stay row-by-row equal across every filter combination, or the two
        // query modes drift on expiry/supersession semantics.
        let root = temp_root("mem-current-predicate");
        let mut conn = open_memory_store(&root).unwrap();
        let now = unix_now();
        memory_write(&mut conn, &note("d1", "durable body", "fact"), now).unwrap();
        let mut expired = note("d2", "stale body", "fact");
        expired.expires_at = now - 10;
        memory_write(&mut conn, &expired, now).unwrap();
        let mut expiring_soon = note("d3", "later body", "gotcha");
        expiring_soon.expires_at = now + 3600;
        memory_write(&mut conn, &expiring_soon, now).unwrap();
        memory_write(&mut conn, &note("d4", "old rev", "decision"), now).unwrap();
        memory_write(&mut conn, &note("d4", "new rev", "decision"), now).unwrap();

        let all_rows: Vec<MemRow> = {
            let mut stmt = conn
                .prepare(&format!("SELECT {MEM_ROW_SQL} FROM memories"))
                .unwrap();
            stmt.query_map([], mem_row)
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(all_rows.iter().any(|r| r.superseded_by.is_some()));
        assert!(all_rows
            .iter()
            .any(|r| r.expires_at != 0 && r.expires_at <= now));
        for include_superseded in [false, true] {
            for kind in [None, Some("fact"), Some("decision")] {
                for scope in [None, Some("durable"), Some("project")] {
                    let filter = MemFilter {
                        kind,
                        scope,
                        now,
                        include_superseded,
                    };
                    let mut expected: Vec<i64> = all_rows
                        .iter()
                        .filter(|row| memory_row_is_current(row, &filter))
                        .map(|row| row.id)
                        .collect();
                    expected.sort();
                    let mut got: Vec<i64> = memory_current_rows(&conn, &filter)
                        .unwrap()
                        .iter()
                        .map(|row| row.id)
                        .collect();
                    got.sort();
                    assert_eq!(
                        got, expected,
                        "kind={kind:?} scope={scope:?} include_superseded={include_superseded}"
                    );
                }
            }
        }
        drop(conn);
        let _ = fs::remove_dir_all(&root);
    }

    /// Column names of `CREATE TABLE memories(...)` in schema_memory.sql:
    /// first token per line inside the parens, with trailing comments and
    /// constraint heads stripped.
    fn schema_memory_columns(sql: &str) -> Vec<String> {
        let body = sql
            .split("CREATE TABLE memories(")
            .nth(1)
            .expect("schema_memory.sql must declare memories");
        let body = body.split(");").next().unwrap_or(body);
        const CONSTRAINT_HEADS: [&str; 5] = ["primary", "unique", "check", "foreign", "constraint"];
        body.lines()
            .filter_map(|line| {
                let line = line.split("--").next().unwrap_or("").trim();
                let head = line.split_whitespace().next()?.trim_end_matches(',');
                (!head.is_empty()
                    && !CONSTRAINT_HEADS.contains(&head.to_ascii_lowercase().as_str()))
                .then(|| head.to_string())
            })
            .collect()
    }

    #[test]
    fn memory_required_manifest_matches_schema_file() {
        // MEMORY_REQUIRED is the hand-copied "required structure" manifest the
        // open gates check; this pins it to schema_memory.sql in both
        // directions, so a schema bump that forgets the manifest (or a
        // manifest edit that outlives a column) fails here instead of pushing
        // a valid store to exit 5 or opening a short store read-write.
        let sql =
            fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("schema_memory.sql"))
                .unwrap();
        let schema_columns = schema_memory_columns(&sql);
        let required: std::collections::HashSet<&str> = MEMORY_REQUIRED
            .iter()
            .map(|(table, column)| {
                assert_eq!(*table, "memories", "unexpected manifest table: {table}");
                *column
            })
            .collect();
        for column in &schema_columns {
            assert!(
                required.contains(&column.as_str()),
                "schema_memory.sql column '{column}' is missing from MEMORY_REQUIRED"
            );
        }
        for column in &required {
            assert!(
                schema_columns.iter().any(|c| c == column),
                "MEMORY_REQUIRED column '{column}' is missing from schema_memory.sql"
            );
        }
    }

    #[test]
    fn memory_write_dedup_supersedes_and_gates_versions() {
        let root = temp_root("mem-lifecycle");
        let mut conn = open_memory_store(&root).unwrap();
        let now = unix_now();
        let first = memory_write(&mut conn, &note("k", "body one", "decision"), now).unwrap();
        assert!(matches!(
            first,
            MemWriteOutcome::Written {
                rev: 1,
                superseded_rev: None
            }
        ));
        // Idempotent replay: identical content is a no-op naming the rev.
        let dup = memory_write(&mut conn, &note("k", "body one", "decision"), now).unwrap();
        assert!(matches!(dup, MemWriteOutcome::Duplicate { rev: 1 }));
        // Different content: new rev, previous marked superseded.
        let second = memory_write(&mut conn, &note("k", "body two", "decision"), now).unwrap();
        assert!(matches!(
            second,
            MemWriteOutcome::Written {
                rev: 2,
                superseded_rev: Some(1)
            }
        ));
        let (row, revs) = memory_get(&conn, "k").unwrap().unwrap();
        assert_eq!((row.rev, row.body.as_str(), revs), (2, "body two", 2));
        // Missing key reads as None; deleting an unknown key deletes nothing.
        assert!(memory_get(&conn, "missing").unwrap().is_none());
        assert!(memory_delete(&mut conn, "missing").unwrap().is_none());
        drop(conn);

        // Newer than the binary -> exit 5 with the export-backup message.
        {
            let conn = Connection::open(memory_db_path(&root)).unwrap();
            conn.execute_batch("PRAGMA user_version = 99").unwrap();
        }
        let err = open_memory_query(&root).unwrap_err();
        assert_eq!(err.exit_code(), 5);
        assert!(
            err.to_string()
                .contains("restore from an aw mem export backup"),
            "{err}"
        );
        // Older but missing required structure -> the other exit-5 message.
        {
            let conn = Connection::open(memory_db_path(&root)).unwrap();
            conn.execute_batch(
                "PRAGMA user_version = 0;
                 DROP TABLE memories_fts;",
            )
            .unwrap();
        }
        let err = open_memory_store(&root).unwrap_err();
        assert_eq!(err.exit_code(), 5);
        assert!(
            err.to_string().contains("missing required structure"),
            "{err}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn cross_key_supersede_removes_target_fts() {
        let root = temp_root("mem-cross-supersede");
        let mut conn = open_memory_store(&root).unwrap();
        let now = unix_now();
        memory_write(&mut conn, &note("k1", "alpha payload", "fact"), now).unwrap();
        let mut target = note("k2", "beta payload", "fact");
        target.supersedes = Some("k1");
        memory_write(&mut conn, &target, now).unwrap();
        // k1 is superseded: `mem get` hides it and its FTS row left the index.
        assert!(memory_get(&conn, "k1").unwrap().is_none());
        let hits = memory_fts_candidates(
            &conn,
            &crate::memory::mem_query_terms("alpha"),
            crate::memory::CANDIDATE_CAP,
        )
        .unwrap();
        assert!(hits.is_empty(), "superseded note still matched: {hits:?}");
        let orphan: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories_fts WHERE rowid NOT IN (SELECT id FROM memories)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(orphan, 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn memory_corrupt_file_is_exit3_and_missing_is_exit1() {
        let root = temp_root("mem-corrupt");
        fs::create_dir_all(aw_dir(&root)).unwrap();
        fs::write(memory_db_path(&root), b"definitely not a sqlite header").unwrap();
        let err = open_memory_query(&root).unwrap_err();
        assert_eq!(err.exit_code(), 3, "{err}");
        let empty = temp_root("mem-missing");
        let err = open_memory_query(&empty).unwrap_err();
        assert_eq!(err.exit_code(), 1, "{err}");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&empty);
    }

    #[test]
    fn mem_commands_work_without_an_index() {
        // Memory is independent of index.db: no build, no index, full write+read.
        let root = temp_root("mem-no-index");
        let mut conn = open_memory_store(&root).unwrap();
        let now = unix_now();
        memory_write(&mut conn, &note("standalone", "works alone", "fact"), now).unwrap();
        let candidates = memory_fts_candidates(
            &conn,
            &crate::memory::mem_query_terms("works alone"),
            crate::memory::CANDIDATE_CAP,
        )
        .unwrap();
        assert_eq!(candidates.len(), 1);
        assert!(!db_path(&root).exists(), "no index.db was created");
        drop(conn);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn gc_purges_expired_rows_and_their_fts_entries() {
        let root = temp_root("mem-gc-expiry");
        let mut conn = open_memory_store(&root).unwrap();
        let now = unix_now();
        // A session note already expired (injected clock) and a live one.
        let mut expired = note("expired-note", "transient scratch", "fact");
        expired.expires_at = now - 1;
        memory_write(&mut conn, &expired, now - 100).unwrap();
        memory_write(
            &mut conn,
            &note("live-note", "durable knowledge", "fact"),
            now,
        )
        .unwrap();
        // Boundary: a row whose deadline is exactly now is expired (<= now).
        let mut boundary = note("boundary-note", "edge case", "fact");
        boundary.expires_at = now;
        memory_write(&mut conn, &boundary, now).unwrap();

        let (removed_expired, removed_superseded, durable) = memory_gc(&mut conn, now).unwrap();
        assert_eq!((removed_expired, removed_superseded, durable), (2, 0, 1));
        // The expired rows and their FTS entries are gone.
        let fts_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts_count, 1, "only the live note keeps an FTS row");
        let current = memory_current_rows(
            &conn,
            &MemFilter {
                kind: None,
                scope: None,
                now: now + 10,
                include_superseded: false,
            },
        )
        .unwrap();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].key, "live-note");
        // A row one second inside the deadline survives gc.
        let mut alive = note("almost-expired", "still inside", "fact");
        alive.expires_at = now + 1;
        memory_write(&mut conn, &alive, now).unwrap();
        let (removed, _, _) = memory_gc(&mut conn, now).unwrap();
        assert_eq!(removed, 0, "expires_at > now is not expired");
        drop(conn);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn gc_retention_clock_is_superseded_at_not_created_at() {
        let root = temp_root("mem-gc-retention");
        let mut conn = open_memory_store(&root).unwrap();
        let now = unix_now();
        let day: i64 = 86_400;
        // Superseded yesterday but CREATED 40 days ago: must survive (the
        // retention clock is superseded_at).
        memory_write(
            &mut conn,
            &note("old-key", "v1 body", "fact"),
            now - 40 * day,
        )
        .unwrap();
        memory_write(&mut conn, &note("old-key", "v2 body", "fact"), now - day).unwrap();
        // Superseded 31 days ago (created 32 days ago): must be purged.
        memory_write(
            &mut conn,
            &note("ancient-key", "v1 body", "fact"),
            now - 32 * day,
        )
        .unwrap();
        memory_write(
            &mut conn,
            &note("ancient-key", "v2 body", "fact"),
            now - 31 * day,
        )
        .unwrap();

        let (_, removed_superseded, durable) = memory_gc(&mut conn, now).unwrap();
        assert_eq!(
            removed_superseded, 1,
            "only the 31-day-old supersession purges"
        );
        assert_eq!(durable, 2, "both current revs survive");
        let (row, revs) = memory_get(&conn, "old-key").unwrap().unwrap();
        assert_eq!(
            (row.rev, revs),
            (2, 2),
            "the young supersession survives audit"
        );
        drop(conn);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn supersedes_marks_the_target_and_rolls_back_on_unknown() {
        let root = temp_root("mem-gc-supersedes");
        let mut conn = open_memory_store(&root).unwrap();
        let now = unix_now();
        memory_write(
            &mut conn,
            &note("target", "old conclusion", "decision"),
            now,
        )
        .unwrap();
        let mut replacement = note("replacement", "new conclusion", "decision");
        replacement.supersedes = Some("target");
        let outcome = memory_write(&mut conn, &replacement, now).unwrap();
        assert!(matches!(
            outcome,
            MemWriteOutcome::Written {
                rev: 1,
                superseded_rev: None
            }
        ));
        // The target's current rev is superseded by the new note.
        let target_rev: Option<i64> = conn
            .query_row(
                "SELECT superseded_by FROM memories WHERE key = 'target' AND rev = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let new_id: i64 = conn
            .query_row(
                "SELECT id FROM memories WHERE key = 'replacement'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(target_rev, Some(new_id));
        // Unknown target: the write rolls back entirely.
        let mut doomed = note("doomed", "never lands", "fact");
        doomed.supersedes = Some("missing-key");
        let err = memory_write(&mut conn, &doomed, now).unwrap_err();
        assert_eq!(err.exit_code(), 1, "{err}");
        assert!(
            memory_get(&conn, "doomed").unwrap().is_none(),
            "the failed supersedes write must leave no row"
        );
        drop(conn);
        let _ = fs::remove_dir_all(&root);
    }

    static GLOBAL_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn global_memory_creates_dir_and_schema_safely() {
        let _guard = GLOBAL_ENV_LOCK.lock().unwrap();
        let base = temp_root("global-mem-schema");
        let tmp = base.join("sub-global-aw");
        let prev = std::env::var("AW_GLOBAL_DIR").ok();
        std::env::set_var("AW_GLOBAL_DIR", &tmp);

        let path = global_memory_db_path().unwrap();
        assert_eq!(path, tmp.join("memory.db"));
        assert!(!tmp.is_dir(), "path resolution must not create directory");

        let conn = open_global_memory_store().unwrap();
        assert!(tmp.is_dir(), "store creation must create directory");
        let v: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, MEMORY_SCHEMA_VERSION);
        drop(conn);

        if let Some(val) = prev {
            std::env::set_var("AW_GLOBAL_DIR", val);
        } else {
            std::env::remove_var("AW_GLOBAL_DIR");
        }
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn global_memory_write_and_query_roundtrip() {
        let _guard = GLOBAL_ENV_LOCK.lock().unwrap();
        let tmp = temp_root("global-mem-roundtrip");
        let prev = std::env::var("AW_GLOBAL_DIR").ok();
        std::env::set_var("AW_GLOBAL_DIR", &tmp);

        // Before creation, query returns NoMemory
        assert!(matches!(
            open_global_memory_query().unwrap_err(),
            AwError::NoMemory(_)
        ));

        let mut conn = open_global_memory_store().unwrap();
        let now = unix_now();
        let outcome = memory_write(
            &mut conn,
            &note("global-pref", "rust minimal", "decision"),
            now,
        )
        .unwrap();
        assert!(matches!(outcome, MemWriteOutcome::Written { rev: 1, .. }));
        drop(conn);

        // Read back with query
        let q_conn = open_global_memory_query().unwrap();
        let (row, revs) = memory_get(&q_conn, "global-pref").unwrap().unwrap();
        assert_eq!(row.key, "global-pref");
        assert_eq!(row.body, "rust minimal");
        assert_eq!(revs, 1);
        drop(q_conn);

        if let Some(val) = prev {
            std::env::set_var("AW_GLOBAL_DIR", val);
        } else {
            std::env::remove_var("AW_GLOBAL_DIR");
        }
        let _ = fs::remove_dir_all(&tmp);
    }
}
