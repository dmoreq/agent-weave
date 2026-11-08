//! Durable memory store paths and open gates: structure manifest checks,
//! version gating, corruption mapping, and local/global connection helpers.

use super::*;

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
pub(super) fn map_memory_err(e: rusqlite::Error) -> AwError {
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
    ensure_dir_validated(path, "global .aw directory", true)
}

pub fn global_memory_db_path() -> Result<PathBuf> {
    Ok(global_aw_dir()?.join("memory.db"))
}

pub fn global_usage_log_path() -> Result<PathBuf> {
    Ok(global_aw_dir()?.join("usage.log"))
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
        match conn.execute_batch(include_str!("../../schema_memory.sql")) {
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
