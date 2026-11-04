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
pub const METRICS_VERSION: &str = "1";

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
            fs::create_dir(&dir)?;
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
pub fn ensure_aw_gitignore(root: &Path) -> Result<()> {
    let dir = ensure_aw_dir(root)?;
    let marker = dir.join(".gitignore");
    match fs::symlink_metadata(&marker) {
        Ok(meta) if meta.file_type().is_symlink() => Err(AwError::Internal(format!(
            "refusing symlinked .aw/.gitignore: {}",
            marker.display()
        ))),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&marker)?;
            file.write_all(b"*\n")?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

/// Stable build identity: an exclusive flock on the repository directory
/// itself. The directory inode survives `.aw/` removal, so maintenance and
/// builds cannot split into two lock identities across invalidation.
pub struct RootLock {
    file: File,
}

impl RootLock {
    pub fn acquire(root: &Path) -> Result<RootLock> {
        let file = File::open(root)?;
        match file.try_lock() {
            Ok(()) => Ok(RootLock { file }),
            Err(std::fs::TryLockError::WouldBlock) => Err(AwError::Locked(0)),
            Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
        }
    }
}

impl Drop for RootLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
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

///Exclusive flock over `.aw/build.lock` for the whole build (CUJ6).
pub struct BuildLock {
    _root_lock: RootLock,
    file: File,
}

impl BuildLock {
    pub fn acquire(root: &Path) -> Result<BuildLock> {
        let root_lock = RootLock::acquire(root)?;
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
        Ok(BuildLock {
            _root_lock: root_lock,
            file,
        })
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
    // Amendment 12 landed schema v4 in two waves (symbols_fts first,
    // clone_groups later) behind ONE version stamp, so a v4-stamped index
    // from the first wave lacks clone_groups and the carry-forward INSERT
    // would fail. Require the complete v4 table set before enabling the
    // fast path; a partial index falls back to a full fresh build.
    let has_clone_groups: i64 = match old.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'clone_groups'",
        [],
        |r| r.get(0),
    ) {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    if has_clone_groups == 0 {
        return Ok(None);
    }
    // Amendment 19: a v6-stamped index can still be partial (the v4 lesson).
    // Require every new column and the complexity index before enabling the
    // fast path; otherwise fall back to a full fresh build.
    let required_columns = [
        ("symbols", "cyclomatic"),
        ("symbols", "branches"),
        ("symbols", "nesting"),
        ("symbols", "effective_loc"),
        ("symbols", "exported"),
        ("symbols", "file_path"),
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
         WHERE e.kind IN ('calls', 'imports', 'inherits')",
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
               AND e.kind IN ('calls', 'imports', 'inherits')",
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
            "INSERT INTO files(id, path, size, mtime, content_hash, parse_status, error_nodes, unresolved, loc, cyclomatic, generated)
             SELECT id, path, size, mtime, content_hash, parse_status, error_nodes, unresolved, loc, cyclomatic, generated
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
        "INSERT INTO symbols(id, kind, name, qualname, file_id, file_path, line_start, line_end, signature, cyclomatic, branches, nesting, effective_loc, exported)
         SELECT id, kind, name, qualname, file_id, file_path, line_start, line_end, signature, cyclomatic, branches, nesting, effective_loc, exported
         FROM olddb.symbols WHERE file_id IN (SELECT id FROM files);
         INSERT INTO edges(id, src_id, dst_id, dst_name, kind, confidence, site_line)
         SELECT id, src_id, dst_id, dst_name, kind, confidence, site_line
         FROM olddb.edges
         WHERE src_id IN (SELECT id FROM symbols)
           AND (dst_id IS NULL OR dst_id IN (SELECT id FROM symbols));
         INSERT INTO clone_groups(shingle_hash, file_id, line_start, line_end)
         SELECT shingle_hash, file_id, line_start, line_end
         FROM olddb.clone_groups WHERE file_id IN (SELECT id FROM files);
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
    /// Hotspots metrics (Amendment 11d), computed on the build's parse tree.
    pub loc: i64,
    pub cyclomatic: i64,
    /// Generated-file tag (Amendment 19).
    pub generated: i64,
}

/// Amendment 11b: caller holds the build transaction; 500-row prepared
/// batches remain (statement reuse), the per-chunk BEGIN/COMMIT fsyncs do not.
pub fn insert_files(conn: &Connection, rows: &[FilesRow]) -> Result<Vec<i64>> {
    let mut ids = Vec::with_capacity(rows.len());
    let mut stmt = conn.prepare(
        "INSERT INTO files(path, size, mtime, content_hash, parse_status, error_nodes, unresolved, loc, cyclomatic, generated)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
                r.loc,
                r.cyclomatic,
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
        "INSERT INTO symbols(kind, name, qualname, file_id, file_path, line_start, line_end, signature, cyclomatic, branches, nesting, effective_loc, exported)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
                s.cyclomatic as i64,
                s.branches as i64,
                s.nesting as i64,
                s.effective_loc as i64,
                i64::from(s.exported)
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

pub fn insert_clone_groups(
    conn: &Connection,
    per_file: &[crate::clones::FileShingles],
) -> Result<()> {
    // Raw bind/execute loop (no per-row params! allocation): this is the
    // highest-row insert in the build and the per-row overhead showed up as
    // build-wall overhead in the G16 gate measurement (Amendment 12d).
    let mut stmt = conn.prepare(
        "INSERT INTO clone_groups(shingle_hash, file_id, line_start, line_end)
         VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (file_id, shingles) in per_file {
        for (line_start, line_end, hash) in shingles {
            stmt.raw_bind_parameter(1, hash)?;
            stmt.raw_bind_parameter(2, file_id)?;
            stmt.raw_bind_parameter(3, line_start)?;
            stmt.raw_bind_parameter(4, line_end)?;
            stmt.raw_execute()?;
        }
    }
    Ok(())
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

/// Read one meta-manifest value (payload journal records built_at/source_rev).
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
