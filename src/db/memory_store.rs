//! Durable memory store paths and open gates: structure manifest checks,
//! version gating, corruption mapping, and local/global connection helpers.

use super::memory_fts::ensure_memory_fts_v2;
use super::*;

/// Compiled memory schema version; gates `.engram/memory.db` like SCHEMA_VERSION
/// gates the index, with a diverging policy: older-but-structurally-complete
/// stores open read-write (memory cannot be rebuilt), only newer-than-binary
/// or missing-required-structure exits 5.
pub const MEMORY_SCHEMA_VERSION: i32 = 2;
const MEMORY_BUSY_TIMEOUT_MS: u64 = 5000;
/// Recovery hint shared by every memory-store failure message (corruption
/// and both schema-gate shapes) so the wording cannot drift apart.
const MEMORY_RESTORE_HINT: &str =
    "restore from an eg mem export backup or remove .engram/memory.db";

pub fn memory_db_path(root: &Path) -> PathBuf {
    eg_dir(root).join("memory.db")
}

/// Required structure manifest, checked at open via pragma_table_info — the
/// probe_old pattern. "Structurally complete" is this list, not an adjective.
pub(super) const MEMORY_REQUIRED: [(&str, &str); 15] = [
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
pub(super) fn map_memory_err(e: rusqlite::Error) -> EgError {
    if let rusqlite::Error::SqliteFailure(ffi, _) = &e {
        if is_sqlite_corruption(&e) {
            return EgError::CorruptMemory(format!(
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
            return EgError::MemoryLocked;
        }
    }
    EgError::Internal(format!("memory database error: {e}"))
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
/// point at the `eg mem export` backup, never at `eg build`.
fn memory_version_gate(conn: &Connection) -> Result<()> {
    let have: i32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(map_memory_err)?;
    if have > MEMORY_SCHEMA_VERSION {
        return Err(EgError::MemorySchema {
            detail: format!(
                "memory.db schema v{have} is newer than the binary expects (v{MEMORY_SCHEMA_VERSION}) — {MEMORY_RESTORE_HINT}"
            ),
        });
    }
    if !memory_manifest_complete(conn)? {
        return Err(EgError::MemorySchema {
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

fn resolve_default_home_eg_dir() -> Result<PathBuf> {
    Ok(PathBuf::from(home_dir()?).join(".engram"))
}

/// Directory for the cross-project global memory store.
/// Reads `$ENGRAM_GLOBAL_DIR`, else `$HOME/.engram` or `%USERPROFILE%/.engram`.
/// Refuses symlinks.
pub fn global_eg_dir() -> Result<PathBuf> {
    let path = match env_non_empty(&["ENGRAM_GLOBAL_DIR"]) {
        Some(val) => PathBuf::from(val),
        None => resolve_default_home_eg_dir()?,
    };
    reject_symlink(&path, "global .engram directory")?;
    if let Ok(meta) = fs::symlink_metadata(&path) {
        if !meta.is_dir() {
            return Err(EgError::Internal(format!(
                "global .engram exists but is not a directory: {}",
                path.display()
            )));
        }
    }
    Ok(path)
}

/// First non-empty value among `keys`, in priority order.
pub(crate) fn env_non_empty(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match std::env::var(key) {
        Ok(val) if !val.trim().is_empty() => Some(val),
        _ => None,
    })
}

/// Whether the global confinement directory was overridden by
/// `ENGRAM_GLOBAL_DIR`.
pub fn global_dir_overridden() -> bool {
    env_non_empty(&["ENGRAM_GLOBAL_DIR"]).is_some()
}

fn home_dir() -> Result<String> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| {
            EgError::Internal(
                "could not determine home directory (HOME/USERPROFILE unset)".to_string(),
            )
        })
}

fn ensure_global_eg_dir(path: &Path) -> Result<()> {
    crate::db::ensure_global_dir(path)
}

pub fn global_memory_db_path() -> Result<PathBuf> {
    Ok(global_eg_dir()?.join("memory.db"))
}

pub fn global_usage_log_path() -> Result<PathBuf> {
    Ok(global_eg_dir()?.join("usage.log"))
}

fn global_memory_store_path() -> Result<PathBuf> {
    let path = global_memory_db_path()?;
    reject_symlink(&path, "global memory.db")?;
    Ok(path)
}

fn no_global_memory_error(path: &Path) -> EgError {
    EgError::NoMemory(format!(
        "no {} — write with eg mem write --global",
        path.display()
    ))
}

/// Open the memory store for writing, creating `.engram/memory.db` (schema v2)
/// when absent. An existing store must pass the version gate (read-forward:
/// older structurally complete stores open read-write).
pub fn open_memory_store(root: &Path) -> Result<Connection> {
    let path = memory_store_path(root)?;
    if !path.exists() {
        ensure_eg_gitignore(root)?;
    }
    open_memory_conn_path(&path, true, || no_memory_error(root))
}
/// Open an existing memory store for mutation WITHOUT creating one: a delete
/// against a missing store must exit 1, never conjure an empty database.
/// Open one discovered store for mutation, addressed by its own db path.
/// Touch uses this after resolving a `store_id`; there is no fallback.
pub fn open_memory_mutation_at(db_path: &Path) -> Result<Connection> {
    // Same write-path guards the CLI enforces: a symlinked store or a
    // git-tracked .engram is refused, so the dashboard cannot write outside
    // the directory it resolved. `db_path` is `<root>/.engram/memory.db`;
    // derive the repository root before probing (the old parent() call probed
    // `<root>/.engram/.engram` and always passed).
    reject_symlink(db_path, "memory store")?;
    if let Some(eg) = db_path.parent() {
        if eg
            .file_name()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(".engram"))
        {
            if let Some(root) = eg.parent() {
                reject_tracked_eg_dir(root)?;
            }
        }
    }
    open_memory_conn_path(db_path, false, || {
        EgError::NoResults(format!("no memory store at {}", db_path.display()))
    })
}

pub fn open_memory_mutation(root: &Path) -> Result<Connection> {
    let path = memory_store_path(root)?;
    open_memory_conn_path(&path, false, || no_memory_error(root))
}

pub fn open_global_memory_store() -> Result<Connection> {
    let dir = global_eg_dir()?;
    ensure_global_eg_dir(&dir)?;
    let path = global_memory_store_path()?;
    open_memory_conn_path(&path, true, || no_global_memory_error(&path))
}

pub fn open_global_memory_mutation() -> Result<Connection> {
    let path = global_memory_store_path()?;
    open_memory_conn_path(&path, false, || no_global_memory_error(&path))
}

fn memory_store_path(root: &Path) -> Result<PathBuf> {
    reject_symlink(&eg_dir(root), ".engram directory")?;
    let path = memory_db_path(root);
    reject_symlink(&path, "memory.db")?;
    // Only an existing store can carry planted notes, and the provenance probe
    // costs a `git` spawn: an absent store is created fresh by the write path.
    if path.exists() {
        crate::db::reject_tracked_eg_dir(root)?;
    }
    Ok(path)
}

pub fn no_memory_error(root: &Path) -> EgError {
    EgError::NoMemory(format!(
        "no .engram/memory.db under {} — write with eg mem write",
        root.display()
    ))
}

fn open_memory_conn_path(
    path: &Path,
    create: bool,
    missing_err: impl FnOnce() -> EgError,
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
    // Additive v1 -> v2: `anchored_at` records the last verify without
    // destroying the creation date. Only the write path migrates; readers
    // fall back to `created_at` when the column is absent.
    migrate_memory_schema(&conn)?;
    // Derived v2 index: reconcile pending/missing rows on every write open.
    ensure_memory_fts_v2(&conn, false)?;
    // Owner-only mode: notes are private primary data.
    let _ = crate::db::harden_file_mode(path);
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
        match conn.execute_batch(include_str!("../../schema/memory.sql")) {
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

/// Additive schema migration: v1 stores gain `anchored_at`, `fts_version`, the
/// meta table, and the time/pending indexes. Each step is independent; a
/// duplicate-column race is tolerated, every other error rolls back.
fn migrate_memory_schema(conn: &Connection) -> Result<()> {
    // Read-only fast path: a fully migrated store must not pay a write
    // transaction on every `mem write` open.
    let ready = column_present(conn, "memories", "anchored_at").map_err(map_memory_err)?
        && column_present(conn, "memories", "fts_version").map_err(map_memory_err)?
        && super::query_state::object_exists_result(conn, "table", "memory_meta")
            .map_err(map_memory_err)?
        && super::query_state::object_exists_result(conn, "index", "memories_created")
            .map_err(map_memory_err)?
        && super::query_state::object_exists_result(conn, "index", "memories_pending")
            .map_err(map_memory_err)?;
    if ready {
        return Ok(());
    }
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(map_memory_err)?;
    let step = (|| -> Result<()> {
        add_column_if_missing(
            conn,
            "memories",
            "anchored_at",
            "ALTER TABLE memories ADD COLUMN anchored_at INTEGER NOT NULL DEFAULT 0",
        )?;
        add_column_if_missing(
            conn,
            "memories",
            "fts_version",
            "ALTER TABLE memories ADD COLUMN fts_version INTEGER NOT NULL DEFAULT 0",
        )?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memory_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS memories_created ON memories(created_at);
             CREATE INDEX IF NOT EXISTS memories_pending ON memories(id) WHERE fts_version < 2;",
        )
        .map_err(map_memory_err)?;
        conn.pragma_update(None, "user_version", MEMORY_SCHEMA_VERSION)
            .map_err(map_memory_err)?;
        Ok(())
    })();
    match step {
        Ok(()) => conn.execute_batch("COMMIT").map_err(map_memory_err),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

fn add_column_if_missing(conn: &Connection, table: &str, column: &str, ddl: &str) -> Result<()> {
    if column_present(conn, table, column).map_err(map_memory_err)? {
        return Ok(());
    }
    match conn.execute_batch(ddl) {
        Ok(()) => Ok(()),
        Err(e) if is_already_exists(&e) => Ok(()),
        Err(e) => Err(map_memory_err(e)),
    }
}

fn open_memory_query_path(
    path: &Path,
    missing_err: impl FnOnce() -> EgError,
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
