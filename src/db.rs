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
pub(crate) fn ints3(r: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, i64, i64)> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
}

pub(crate) fn ints4(r: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, i64, i64, i64)> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
}

pub fn aw_dir(root: &Path) -> PathBuf {
    root.join(".aw")
}

pub(crate) fn reject_symlink(path: &Path, label: &str) -> Result<()> {
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

/// Create or validate a directory, refusing a symlinked path. A symlink can
/// redirect every later write outside the intended root. `recursive` creates
/// missing parents (the global store may nest under `$HOME/.aw`); the
/// repo-local `.aw` must not. Creation race (two first-time writers, e.g.
/// concurrent mem writes — memory takes no build lock): the loser's create
/// reports AlreadyExists and re-validates the winner's directory. The single
/// implementation shared by the local and global `.aw` sides.
fn ensure_dir_validated(path: &Path, label: &str, recursive: bool) -> Result<()> {
    reject_symlink(path, label)?;
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_dir() => Err(AwError::Internal(format!(
            "{label} exists but is not a directory: {}",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let created = if recursive {
                fs::create_dir_all(path)
            } else {
                fs::create_dir(path)
            };
            if let Err(create_err) = created {
                if create_err.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(create_err.into());
                }
            }
            reject_symlink(path, label)?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

/// Create or validate `.aw/`, refusing a symlinked directory.
pub fn ensure_aw_dir(root: &Path) -> Result<PathBuf> {
    let dir = aw_dir(root);
    ensure_dir_validated(&dir, ".aw directory", false)?;
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

#[cfg(test)]
pub(crate) mod test_support {
    //! Filesystem mutation helpers for db tests: every fs write performed by
    //! the db test modules is defined here, lexically inside `db.rs`, keeping
    //! the write-confinement invariant intact.

    use std::fs;
    use std::path::{Path, PathBuf};

    /// Fresh throwaway test root (`aw-{name}-{pid}`), wiped from any previous
    /// run; shared by journal, doctor, and walk tests via `crate::db::temp_root`.
    pub(crate) fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aw-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub(crate) fn write_file(path: impl AsRef<Path>, bytes: &[u8]) -> std::io::Result<()> {
        fs::write(path, bytes)
    }

    pub(crate) fn rename_file(from: impl AsRef<Path>, to: impl AsRef<Path>) -> std::io::Result<()> {
        fs::rename(from, to)
    }

    pub(crate) fn make_dir_all(path: impl AsRef<Path>) -> std::io::Result<()> {
        fs::create_dir_all(path)
    }

    pub(crate) fn remove_tree(path: impl AsRef<Path>) {
        let _ = fs::remove_dir_all(path);
    }
}

#[cfg(test)]
pub(crate) use test_support::temp_root;

mod inserts;
#[cfg(test)]
mod memory_gc_tests;
mod memory_query;
mod memory_store;
#[cfg(test)]
mod memory_tests;
mod memory_write;
mod meta;
mod query_state;
#[cfg(test)]
mod tests;

pub(crate) use inserts::*;
pub(crate) use memory_query::*;
pub(crate) use memory_store::*;
pub(crate) use memory_write::*;
pub(crate) use meta::*;
pub(crate) use query_state::*;
