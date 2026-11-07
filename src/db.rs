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

use crate::error::{AwError, Result};
use crate::extract::{CarriedIndex, CarriedSymbol, NewSymbol, SCHEMA_VERSION};

/// Compile-time grammar identity, derived from the Cargo.lock-pinned grammar
/// crate versions by build.rs (R6d A8: a hand-maintained string had already
/// drifted from the lock). The identity gates the fast path.
pub const GRAMMAR_VERSION: &str = env!("AW_GRAMMAR_VERSION");
/// Extraction revision: older indexes must be re-extracted when capture or
/// resolution rules change. This does not change the table shape.
pub const SIGNATURE_VERSION: &str = "3";
/// Per-symbol metric-rule version (Amendment 19); a mismatch disables the fast path.
pub const METRICS_VERSION: &str = "2";

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

/// Create or validate `.aw/`, refusing a symlinked directory. A symlink can
/// redirect every later write outside the repository root.
pub fn ensure_aw_dir(root: &Path) -> Result<PathBuf> {
    let dir = aw_dir(root);
    match fs::symlink_metadata(&dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(AwError::Internal(format!(
                "refusing symlinked .aw directory: {}",
                dir.display()
            )));
        }
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
            if fs::symlink_metadata(&dir)?.file_type().is_symlink() {
                return Err(AwError::Internal(format!(
                    "refusing symlinked .aw directory: {}",
                    dir.display()
                )));
            }
        }
        Err(e) => return Err(e.into()),
    }
    Ok(dir)
}

pub fn db_path(root: &Path) -> PathBuf {
    aw_dir(root).join("index.db")
}

pub fn db_new_path(root: &Path) -> PathBuf {
    aw_dir(root).join("index.db.new")
}

/// Write the tool-owned `.aw/.gitignore` (`*`) on first build: the whole
/// directory disappears from `git status` without touching the repo's own
/// .gitignore (allowed write path per invariant 3's constant list).
/// Fail-closed: a symlink, non-regular file (FIFO/device — rejected before
/// any open), over-cap file, or wrong content is an error, never kept silent.
pub fn ensure_aw_gitignore(root: &Path) -> Result<()> {
    let dir = ensure_aw_dir(root)?;
    let marker = dir.join(".gitignore");
    match fs::symlink_metadata(&marker) {
        Ok(meta) if meta.file_type().is_symlink() => Err(AwError::Internal(format!(
            "refusing symlinked .aw/.gitignore: {}",
            marker.display()
        ))),
        Ok(_) => {
            // An existing marker must still be the tool's own: `read_bounded`
            // rejects non-regular files before any open and caps the read.
            let bytes = crate::walk::read_bounded(&marker)?;
            if bytes != b"*\n" {
                return Err(AwError::Internal(format!(
                    "unexpected .aw/.gitignore content: {}",
                    marker.display()
                )));
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let created = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&marker);
            match created {
                Ok(mut file) => {
                    file.write_all(b"*\n")?;
                    Ok(())
                }
                // Creation race: another writer made the marker first; it
                // must still validate as the tool's own (content check below
                // via the Ok arm of a fresh metadata probe).
                Err(create_err) if create_err.kind() == std::io::ErrorKind::AlreadyExists => {
                    let bytes = crate::walk::read_bounded(&marker)?;
                    if bytes == b"*\n" {
                        Ok(())
                    } else {
                        Err(AwError::Internal(format!(
                            "unexpected .aw/.gitignore content: {}",
                            marker.display()
                        )))
                    }
                }
                Err(e) => Err(e.into()),
            }
        }
        Err(e) => Err(e.into()),
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
        let file = match fs::symlink_metadata(&lock_path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(AwError::Internal(format!(
                    "refusing symlinked build lock: {}",
                    lock_path.display()
                )));
            }
            Ok(meta) if !meta.is_file() || link_count(&meta) > 1 => {
                return Err(AwError::Internal(format!(
                    "refusing unsafe build lock: {}",
                    lock_path.display()
                )));
            }
            Ok(_) => fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&lock_path)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match fs::OpenOptions::new()
                    .create_new(true)
                    .read(true)
                    .write(true)
                    .open(&lock_path)
                {
                    Ok(file) => file,
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        fs::OpenOptions::new()
                            .read(true)
                            .write(true)
                            .open(&lock_path)?
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            Err(e) => return Err(e.into()),
        };
        reject_symlink(&lock_path, "build lock")?;
        let meta = file.metadata()?;
        if !meta.is_file() || link_count(&meta) > 1 {
            return Err(AwError::Internal(format!(
                "refusing unsafe build lock: {}",
                lock_path.display()
            )));
        }
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
        let mut file = file;
        file.seek(SeekFrom::Start(0))?;
        file.set_len(0)?;
        writeln!(file, "{} {}", std::process::id(), env!("CARGO_PKG_VERSION"))?;
        Ok(BuildLock { file })
    }
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
    let version: i32 = match old.query_row("PRAGMA user_version", [], |r| r.get(0)) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    if version != SCHEMA_VERSION {
        return Ok(None);
    }
    // Amendment 19: a partial index (the v4 lesson) can lag its version
    // stamp. Require every v6 column and the complexity index before enabling
    // the fast path; otherwise fall back to a full fresh build.
    let required_columns = [
        ("symbols", "effective_loc"),
        ("symbols", "exported"),
        ("symbols", "file_path"),
        ("symbols", "is_test"),
        ("files", "generated"),
        ("unresolved_calls", "site_line"),
        ("unresolved_calls", "kind"),
    ];
    for (table, column) in required_columns {
        let present: i64 = old
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
                rusqlite::params![table, column],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if present == 0 {
            return Ok(None);
        }
    }
    let has_complexity_index: i64 = old
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'symbols_complexity_idx'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has_complexity_index == 0 {
        return Ok(None);
    }
    let has_symbols_fts: i64 = old
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'symbols_fts'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has_symbols_fts == 0 {
        return Ok(None);
    }
    let grammar: String = match old.query_row(
        "SELECT value FROM meta WHERE key = 'grammar_version'",
        [],
        |r| r.get(0),
    ) {
        Ok(g) => g,
        Err(_) => return Ok(None),
    };
    if grammar != GRAMMAR_VERSION {
        return Ok(None);
    }
    let sig_version = old
        .query_row(
            "SELECT value FROM meta WHERE key = 'signature_version'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap_or_default();
    if sig_version != SIGNATURE_VERSION {
        return Ok(None);
    }
    let metrics_version = old
        .query_row(
            "SELECT value FROM meta WHERE key = 'metrics_version'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap_or_default();
    if metrics_version != METRICS_VERSION {
        return Ok(None);
    }
    // Amendment 21 added the complexity manifest after v7 shipped. An index
    // without it must rebuild; the read-only no-op path cannot backfill.
    let has_callable_count: i64 = old
        .query_row(
            "SELECT COUNT(*) FROM meta WHERE key = 'callable_count'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has_callable_count == 0 {
        return Ok(None);
    }
    Ok(Some(old))
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
    for chunk in rels.chunks(500) {
        let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT COUNT(*) FROM files WHERE parse_status != 'ok' AND path IN ({placeholders})"
        );
        let c: i64 = old.query_row(&sql, rusqlite::params_from_iter(chunk.iter()), |r| r.get(0))?;
        n += c as usize;
    }
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
        for chunk in frontier.chunks(500) {
            let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
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
        }
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
    for chunk in files.chunks(500) {
        let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
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
    }
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
    let mut out = HashSet::new();
    let mut stmt = old.prepare(
        "SELECT DISTINCT f.path, e.dst_name
         FROM edges e
         JOIN symbols s ON s.id = e.src_id
         JOIN files f ON f.id = s.file_id
         WHERE e.kind IN ('calls', 'imports', 'inherits', 'type_ref')",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for r in rows {
        let (path, dst) = r?;
        if changed_qualnames.contains(&dst) {
            out.insert(path);
        }
    }
    let mut stmt = old.prepare(
        "SELECT DISTINCT f.path, u.dst_name
         FROM unresolved_calls u JOIN files f ON f.id = u.file_id",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for r in rows {
        let (path, dst) = r?;
        if changed_qualnames.contains(&dst) {
            out.insert(path);
        }
    }
    Ok(out)
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
    let mut out = HashSet::new();
    // Match by last segment: an unresolved `b.foo` heals when any newly
    // added qualname ends in `foo` (bare-name collisions in the old index
    // must not hide the new symbol — R6 review finding).
    let last_segments: HashSet<&str> = new_qualnames
        .iter()
        .map(|q| q.rsplit('.').next().unwrap_or(q))
        .collect();
    let matches = |dst: &str| -> bool {
        let last = dst.rsplit('.').next().unwrap_or(dst);
        last_segments.contains(last)
    };
    {
        let mut stmt = old.prepare(
            "SELECT f.path, u.dst_name FROM unresolved_calls u
             JOIN files f ON f.id = u.file_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for r in rows {
            let (path, dst) = r?;
            if matches(&dst) {
                out.insert(path);
            }
        }
    }
    {
        let mut stmt = old.prepare(
            "SELECT DISTINCT f.path, e.dst_name FROM edges e
             JOIN symbols s ON s.id = e.src_id JOIN files f ON f.id = s.file_id
             WHERE e.confidence = 'AMBIGUOUS'
               AND e.kind IN ('calls', 'imports', 'inherits', 'type_ref')",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for r in rows {
            let (path, dst) = r?;
            if matches(&dst) {
                out.insert(path);
            }
        }
    }
    Ok(out)
}

/// Insert unresolved edge sites for the fresh rows (schema v6, Amendment 19).
pub fn insert_unresolved_calls(
    conn: &Connection,
    rows: &[(i64, &str, &'static str, usize)],
) -> Result<()> {
    if rows.iter().any(|(_, _, _, site_line)| *site_line == 0) {
        return Err(crate::error::AwError::Internal(
            "unresolved edge row has site_line 0".to_string(),
        ));
    }
    let mut stmt = conn.prepare(
        "INSERT INTO unresolved_calls(file_id, dst_name, kind, site_line) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for chunk in rows.chunks(500) {
        for (file_id, name, kind, site_line) in chunk {
            stmt.execute(rusqlite::params![file_id, name, kind, *site_line as i64])?;
        }
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
    let mut symbols = Vec::new();
    let mut stmt = old.prepare(
        "SELECT s.id, s.kind, s.name, s.qualname, f.path
         FROM symbols s JOIN files f ON f.id = s.file_id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (id, kind, name, qualname, path) in rows {
        if !rels.contains(&path) {
            continue;
        }
        let mapped_kind = match kind.as_str() {
            "module" => "module",
            "class" => "class",
            "function" => "function",
            "method" => "method",
            // A carried trait/interface is a valid impl target on the fast
            // path; dropping it made incremental Rust impls permanently
            // unresolved (R12 Critical).
            "trait" => "trait",
            "interface" => "interface",
            _ => continue,
        };
        let (module, _) = crate::extract::module_path_for(&path);
        symbols.push(CarriedSymbol {
            row_id: id,
            kind: mapped_kind,
            name,
            qualname,
            module,
        });
    }
    let mut inherits = Vec::new();
    let mut rel_params: Vec<&str> = rels.iter().map(String::as_str).collect();
    rel_params.sort_unstable();
    for chunk in rel_params.chunks(500) {
        let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let mut stmt = old.prepare(&format!(
            "SELECT s.qualname, e.dst_name
             FROM edges e JOIN symbols s ON s.id = e.src_id
             JOIN files f ON f.id = s.file_id
             WHERE e.kind = 'inherits' AND e.dst_name IS NOT NULL
               AND f.path IN ({placeholders})"
        ))?;
        let pairs = stmt.query_map(rusqlite::params_from_iter(chunk.iter().copied()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for p in pairs {
            inherits.push(p?);
        }
    }
    Ok(CarriedIndex { symbols, inherits })
}

/// Copy rows for files matching the fast-path keys. `unchanged` rows carry
/// (rel, old_mtime, size, current_mtime): the SELECT identifies the old row by
/// (path, old_mtime, size); hash-carried rows (Amendment 9) then get their
/// stored mtime refreshed to the fresh checkout's stat so the staleness
/// contract stays truthful. Attaches the old DB itself (SQLite ATTACH takes no
/// bound parameters). Returns rel -> old file id for reuse.
/// Amendment 11b: the caller owns the build transaction — no BEGIN/COMMIT here.
pub fn carry_forward_unchanged(
    new: &Connection,
    root: &Path,
    unchanged: &[(String, i64, i64, i64)],
) -> Result<Vec<(String, i64)>> {
    let old_path = db_path(root);
    let quoted = old_path.to_string_lossy().replace('\'', "''");
    new.execute_batch(&format!("ATTACH DATABASE '{quoted}' AS olddb"))?;
    let mut reused = Vec::new();
    {
        let mut stmt = new.prepare(
            "INSERT INTO files(id, path, size, mtime, content_hash, parse_status, error_nodes, unresolved, generated)
             SELECT id, path, size, mtime, content_hash, parse_status, error_nodes, unresolved, generated
             FROM olddb.files WHERE path = ?1 AND mtime = ?2 AND size = ?3",
        )?;
        let mut refresh_mtime = new.prepare("UPDATE files SET mtime = ?1 WHERE path = ?2")?;
        for (rel, mtime, size, current_mtime) in unchanged {
            let inserted = stmt.execute(rusqlite::params![rel, mtime, size])?;
            if inserted > 0 {
                if current_mtime != mtime {
                    refresh_mtime.execute(rusqlite::params![current_mtime, rel])?;
                }
                let id: i64 = new.query_row(
                    "SELECT id FROM files WHERE path = ?1",
                    rusqlite::params![rel],
                    |r| r.get(0),
                )?;
                reused.push((rel.clone(), id));
            }
        }
    }
    new.execute_batch(
        "INSERT INTO symbols(id, kind, name, qualname, file_id, file_path, line_start, line_end, signature, effective_loc, exported, is_test)
         SELECT id, kind, name, qualname, file_id, file_path, line_start, line_end, signature, effective_loc, exported, is_test
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
    Ok(reused)
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
    /// Generated-file tag (Amendment 19).
    pub generated: i64,
}

/// Amendment 11b: caller holds the build transaction; 500-row prepared
/// batches remain (statement reuse), the per-chunk BEGIN/COMMIT fsyncs do not.
pub fn insert_files(conn: &Connection, rows: &[FilesRow]) -> Result<Vec<i64>> {
    let mut ids = Vec::with_capacity(rows.len());
    let mut stmt = conn.prepare(
        "INSERT INTO files(path, size, mtime, content_hash, parse_status, error_nodes, unresolved, generated)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    for chunk in rows.chunks(500) {
        for r in chunk {
            stmt.execute(rusqlite::params![
                r.rel,
                r.size,
                r.mtime,
                r.content_hash,
                r.parse_status,
                r.error_nodes as i64,
                r.unresolved as i64,
                r.generated
            ])?;
            ids.push(conn.last_insert_rowid());
        }
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
        "INSERT INTO symbols(kind, name, qualname, file_id, file_path, line_start, line_end, signature, effective_loc, exported, is_test)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )?;
    for chunk in symbols.chunks(500) {
        for s in chunk {
            stmt.execute(rusqlite::params![
                s.kind,
                s.name,
                s.qualname,
                file_ids[s.file_idx],
                file_paths[s.file_idx].as_str(),
                s.line_start as i64,
                s.line_end as i64,
                s.signature,
                s.effective_loc as i64,
                i64::from(s.exported),
                i64::from(s.is_test)
            ])?;
            ids.push(conn.last_insert_rowid());
        }
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
    pub confidence: &'static str,
    pub site_line: usize,
}

pub fn insert_edge_rows(conn: &Connection, rows: &[EdgeRow<'_>]) -> Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO edges(src_id, dst_id, dst_name, kind, confidence, site_line)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for chunk in rows.chunks(500) {
        for e in chunk {
            stmt.execute(rusqlite::params![
                e.src_id,
                e.dst_id,
                e.dst_name,
                e.kind,
                e.confidence,
                e.site_line as i64
            ])?;
        }
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

/// Nanosecond-precision stat mtime as i64 (fast-path and staleness key).
/// Nanoseconds close the same-second, same-size edit hole that silently kept
/// a stale index; both sides of every comparison use this function.
pub fn stat_mtime(path: &Path) -> Option<i64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_nanos() as i64)
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
        if matches!(
            ffi.code,
            rusqlite::ffi::ErrorCode::DatabaseCorrupt | rusqlite::ffi::ErrorCode::NotADatabase
        ) {
            return AwError::CorruptMemory(
                "memory.db failed integrity check — restore from an aw mem export backup or remove .aw/memory.db"
                    .to_string(),
            );
        }
        if ffi.code == rusqlite::ffi::ErrorCode::DatabaseBusy {
            return AwError::MemoryLocked;
        }
    }
    AwError::Internal(format!("memory database error: {e}"))
}

fn memory_manifest_complete(conn: &Connection) -> Result<bool> {
    for (table, column) in MEMORY_REQUIRED {
        let present: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
                rusqlite::params![table, column],
                |r| r.get(0),
            )
            .map_err(map_memory_err)?;
        if present == 0 {
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
                "memory.db schema v{have} is newer than the binary expects (v{MEMORY_SCHEMA_VERSION}) — restore from an aw mem export backup or remove .aw/memory.db"
            ),
        });
    }
    if !memory_manifest_complete(conn)? {
        return Err(AwError::MemorySchema {
            detail: format!(
                "memory.db schema v{have} is missing required structure for this binary — restore from an aw mem export backup or remove .aw/memory.db"
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

/// Open the memory store for writing, creating `.aw/memory.db` (schema v1)
/// when absent. An existing store must pass the version gate (read-forward:
/// older structurally complete stores open read-write).
pub fn open_memory_store(root: &Path) -> Result<Connection> {
    open_memory_conn(root, true)
}

/// Open an existing memory store for mutation WITHOUT creating one: a delete
/// against a missing store must exit 1, never conjure an empty database.
pub fn open_memory_mutation(root: &Path) -> Result<Connection> {
    open_memory_conn(root, false)
}

fn open_memory_conn(root: &Path, create: bool) -> Result<Connection> {
    reject_symlink(&aw_dir(root), ".aw directory")?;
    let path = memory_db_path(root);
    reject_symlink(&path, "memory.db")?;
    let existed = path.exists();
    if !existed && !create {
        return Err(AwError::NoMemory(format!(
            "no .aw/memory.db under {} — write with aw mem write",
            root.display()
        )));
    }
    if !existed {
        ensure_aw_gitignore(root)?;
    }
    let conn = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .map_err(map_memory_err)?;
    memory_pragmas(&conn)?;
    if !existed {
        // Creation race (two first-time writers): the loser's DDL fails with
        // "already exists" and falls through to the version gate below.
        if let Err(e) = conn.execute_batch(include_str!("../schema_memory.sql")) {
            let already = matches!(
                &e,
                rusqlite::Error::SqliteFailure(_, Some(msg)) if msg.contains("already exists")
            );
            if !already {
                return Err(map_memory_err(e));
            }
        }
        conn.pragma_update(None, "user_version", MEMORY_SCHEMA_VERSION)
            .map_err(map_memory_err)?;
    }
    memory_version_gate(&conn)?;
    Ok(conn)
}

/// Open the memory store read-only for the read commands. Missing -> exit 1
/// (no-memory), corrupt -> exit 3, skew -> exit 5 — the shared gate chain.
pub fn open_memory_query(root: &Path) -> Result<Connection> {
    reject_symlink(&aw_dir(root), ".aw directory")?;
    let path = memory_db_path(root);
    reject_symlink(&path, "memory.db")?;
    if !path.exists() {
        return Err(AwError::NoMemory(format!(
            "no .aw/memory.db under {} — write with aw mem write",
            root.display()
        )));
    }
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(map_memory_err)?;
    memory_pragmas(&conn)?;
    memory_version_gate(&conn)?;
    Ok(conn)
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
    let hash =
        crate::memory::mem_content_hash(note.kind, note.body, note.tags, note.refs, note.scope)
            as i64;
    let fts_text = crate::memory::mem_fts_text(note.key, note.tags, note.body);
    // One in-transaction retry on a UNIQUE(key, rev) failure (a defect by
    // construction — rev is computed inside the lock — but never exit 2/5).
    for attempt in 0..2 {
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(map_memory_err)?;
        let current: Option<(i64, i64, i64)> = tx
            .query_row(
                "SELECT id, rev, content_hash FROM memories WHERE key = ?1 ORDER BY rev DESC LIMIT 1",
                rusqlite::params![note.key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(map_memory_err)?;
        if let Some((_, rev, current_hash)) = current {
            if current_hash == hash {
                // Idempotent replay: nothing written, nothing to commit.
                drop(tx);
                return Ok(MemWriteOutcome::Duplicate { rev });
            }
        }
        let new_rev = current.as_ref().map_or(1, |(_, rev, _)| rev + 1);
        let inserted = tx.execute(
            "INSERT INTO memories(key, rev, kind, scope, session, body, tags, refs, content_hash, source, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                note.key,
                new_rev,
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
        );
        if let Err(e) = inserted {
            if is_unique_violation(&e) && attempt == 0 {
                drop(tx);
                continue;
            }
            return Err(map_memory_err(e));
        }
        let new_id = tx.last_insert_rowid();
        if let Some((old_id, _, _)) = current {
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
        }
        if let Some(target) = note.supersedes {
            // Cross-key supersession inside the same transaction: the target's
            // current rev becomes superseded by the new note. An unknown
            // target rolls the whole write back (exit 1). An identical
            // duplicate never reaches here — a replayed write is a no-op, so
            // supersession side effects do not re-apply.
            let updated = tx
                .execute(
                    "UPDATE memories SET superseded_by = ?1, superseded_at = ?2
                     WHERE id = (SELECT id FROM memories WHERE key = ?3 AND superseded_by IS NULL LIMIT 1)",
                    rusqlite::params![new_id, now, target],
                )
                .map_err(map_memory_err)?;
            if updated == 0 {
                return Err(AwError::NoResults(format!(
                    "unknown memory key '{target}' — nothing superseded, write rolled back"
                )));
            }
        }
        tx.execute(
            "INSERT INTO memories_fts(rowid, text) VALUES (?1, ?2)",
            rusqlite::params![new_id, fts_text],
        )
        .map_err(map_memory_err)?;
        tx.commit().map_err(map_memory_err)?;
        return Ok(MemWriteOutcome::Written {
            rev: new_rev,
            superseded_rev: current.map(|(_, rev, _)| rev),
        });
    }
    Err(AwError::Internal(
        "memory write: repeated UNIQUE(key, rev) violation".to_string(),
    ))
}

/// Current, non-expired rows (the resolver's candidate universe), newest
/// first. `include_superseded` extends the listing with superseded revisions
/// (the `--all --include-superseded` view); expired rows stay invisible in
/// every mode. Empty filter values match everything; all filters are bound
/// parameters, never string interpolation.
pub fn memory_current_rows(
    conn: &Connection,
    kind: Option<&str>,
    scope: Option<&str>,
    now: i64,
    include_superseded: bool,
) -> Result<Vec<MemRow>> {
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
                now,
                kind.unwrap_or(""),
                scope.unwrap_or(""),
                i64::from(include_superseded),
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
             WHERE memories_fts MATCH ?1 ORDER BY rank LIMIT ?2",
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
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
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
    tx.execute(
        "DELETE FROM memories WHERE key = ?1",
        rusqlite::params![key],
    )
    .map_err(map_memory_err)?;
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    tx.execute(
        &format!("DELETE FROM memories_fts WHERE rowid IN ({placeholders})"),
        rusqlite::params_from_iter(ids.iter()),
    )
    .map_err(map_memory_err)?;
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
    for chunk in expired_ids.chunks(500) {
        let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        tx.execute(
            &format!("DELETE FROM memories WHERE id IN ({placeholders})"),
            rusqlite::params_from_iter(chunk.iter()),
        )
        .map_err(map_memory_err)?;
        tx.execute(
            &format!("DELETE FROM memories_fts WHERE rowid IN ({placeholders})"),
            rusqlite::params_from_iter(chunk.iter()),
        )
        .map_err(map_memory_err)?;
    }
    let retention_horizon = now - crate::memory::RETENTION_DAYS * 86_400;
    let superseded = tx
        .execute(
            "DELETE FROM memories
             WHERE superseded_by IS NOT NULL AND superseded_at > 0 AND superseded_at <= ?1",
            rusqlite::params![retention_horizon],
        )
        .map_err(map_memory_err)? as i64;
    let durable: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE superseded_by IS NULL",
            [],
            |r| r.get(0),
        )
        .map_err(map_memory_err)?;
    tx.commit().map_err(map_memory_err)?;
    Ok((expired_ids.len() as i64, superseded, durable))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_db_has_schema_v2() {
        let dir = std::env::temp_dir().join(format!("aw-db-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
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
        let dir = std::env::temp_dir().join(format!("aw-db-missing-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let err = open_for_query(&dir).unwrap_err();
        assert_eq!(err.exit_code(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupted_index_is_exit3() {
        let dir = std::env::temp_dir().join(format!("aw-db-corrupt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".aw")).unwrap();
        fs::write(db_path(&dir), b"this is not sqlite at all").unwrap();
        let err = open_for_query(&dir).unwrap_err();
        assert_eq!(err.exit_code(), 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn schema_skew_is_exit5() {
        let dir = std::env::temp_dir().join(format!("aw-db-skew-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
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
        let dir = std::env::temp_dir().join(format!("aw-db-carried-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
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

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aw-mem-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

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
        let root = temp_root("purity");
        let mut conn = open_memory_store(&root).unwrap();
        let now = unix_now();
        memory_write(&mut conn, &note("k1", "first body text", "fact"), now).unwrap();
        memory_write(&mut conn, &note("k1", "second body", "fact"), now).unwrap();
        memory_write(&mut conn, &note("k2", "other note", "gotcha"), now).unwrap();
        memory_write(&mut conn, &note("k2", "other note", "gotcha"), now).unwrap(); // duplicate no-op
        memory_delete(&mut conn, "k2").unwrap();

        // Rebuild the derived FTS from the authoritative table and compare.
        let current = memory_current_rows(&conn, None, None, now, false).unwrap();
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
    fn memory_write_dedup_supersedes_and_gates_versions() {
        let root = temp_root("lifecycle");
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
    fn memory_corrupt_file_is_exit3_and_missing_is_exit1() {
        let root = temp_root("corrupt");
        fs::create_dir_all(aw_dir(&root)).unwrap();
        fs::write(memory_db_path(&root), b"definitely not a sqlite header").unwrap();
        let err = open_memory_query(&root).unwrap_err();
        assert_eq!(err.exit_code(), 3, "{err}");
        let empty = temp_root("missing");
        let err = open_memory_query(&empty).unwrap_err();
        assert_eq!(err.exit_code(), 1, "{err}");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&empty);
    }

    #[test]
    fn mem_commands_work_without_an_index() {
        // Memory is independent of index.db: no build, no index, full write+read.
        let root = temp_root("no-index");
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
        let root = temp_root("gc-expiry");
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
        let current = memory_current_rows(&conn, None, None, now + 10, false).unwrap();
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
        let root = temp_root("gc-retention");
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
        let root = temp_root("gc-supersedes");
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
}
