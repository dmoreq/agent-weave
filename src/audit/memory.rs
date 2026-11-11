//! Counts-only memory snapshot for audit. Reads current, unexpired refs and
//! resolves them against read-only indexes; never materializes note bodies,
//! keys, tags, or paths.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::db;

use super::collect::Mode;

#[derive(Default)]
pub struct MemoryFacts {
    pub failure: Option<MemoryFailure>,
    pub stores_checked: usize,
    pub stores_truncated: bool,
    pub broken: usize,
    pub modified: usize,
    pub valid: usize,
    pub unverified: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MemoryFailure {
    Corrupt,
    Schema,
    Symlink,
    Tracked,
    TooLarge,
    Unreadable,
}

struct Store {
    repo: Option<PathBuf>,
    global: bool,
    path: PathBuf,
}

pub fn collect(mode: &Mode, repos: &[PathBuf], now: i64) -> MemoryFacts {
    let mut stores: Vec<Store> = Vec::new();
    match mode {
        Mode::Local(root) => stores.push(Store {
            repo: Some(root.clone()),
            global: false,
            path: db::memory_db_path(root),
        }),
        Mode::Global => {
            if let Ok(g) = db::global_eg_dir() {
                stores.push(Store {
                    repo: None,
                    global: true,
                    path: g.join("memory.db"),
                });
            }
            for repo in repos {
                stores.push(Store {
                    repo: Some(repo.clone()),
                    global: false,
                    path: db::memory_db_path(repo),
                });
            }
        }
    }
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut unique: Vec<Store> = Vec::new();
    for store in stores {
        // Keep symlinks so the integrity check can report them; canonicalize
        // only regular files for dedup.
        if symlink_store(&store.path) {
            unique.push(store);
            continue;
        }
        if let Ok(p) = std::fs::canonicalize(&store.path) {
            if seen.insert(p) {
                unique.push(store);
            }
        }
    }
    let truncated = unique.len() > 50;
    unique.truncate(50);
    let mut facts = MemoryFacts {
        stores_truncated: truncated,
        ..Default::default()
    };
    for store in &unique {
        if !regular_store(&store.path) {
            if symlink_store(&store.path) {
                facts.failure = Some(MemoryFailure::Symlink);
            }
            continue;
        }
        if let Some(repo) = &store.repo {
            if db::reject_tracked_eg_dir(repo).is_err() {
                facts.failure = Some(MemoryFailure::Tracked);
                continue;
            }
        }
        let size = db::file_size(&store.path);
        if size > 50 * 1024 * 1024 {
            facts.failure = Some(MemoryFailure::TooLarge);
            continue;
        }
        facts.stores_checked += 1;
        if let Err(kind) = inspect_store(store, size, now, &mut facts) {
            facts.failure = Some(kind);
        }
    }
    facts
}

fn inspect_store(
    store: &Store,
    size: u64,
    now: i64,
    facts: &mut MemoryFacts,
) -> Result<(), MemoryFailure> {
    let conn = open_store(store)?;
    let integrity: std::result::Result<String, _> = if size > 50 * 1024 * 1024 {
        conn.query_row("PRAGMA quick_check", [], |r| r.get(0))
    } else {
        conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))
    };
    if !matches!(integrity, Ok(ref v) if v == "ok") {
        return Err(MemoryFailure::Corrupt);
    }
    let index = store.repo.as_ref().and_then(|r| db::try_open_index(r).ok());
    // v1 stores predate anchored_at; read the creation time as the anchor
    // instead of failing the whole store on a missing column.
    let anchor_expr = if column_present(&conn, "memories", "anchored_at") {
        "anchored_at"
    } else {
        "0"
    };
    let sql = format!(
        "SELECT refs, {anchor_expr}, created_at FROM memories \
         WHERE superseded_by IS NULL AND (expires_at = 0 OR expires_at > ?1) LIMIT 500"
    );
    let mut stmt = conn.prepare(&sql).map_err(|_| MemoryFailure::Corrupt)?;
    let rows = stmt
        .query_map(rusqlite::params![now], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(|_| MemoryFailure::Corrupt)?;
    for row in rows {
        let (refs, anchored_at, created_at) = row.map_err(|_| MemoryFailure::Corrupt)?;
        let anchor = if anchored_at > 0 {
            anchored_at
        } else {
            created_at
        };
        for r in refs.split_whitespace().take(20) {
            let resolved =
                crate::dashboard::data::memories::resolve_single_ref(r, index.as_ref(), anchor);
            match resolved.get("status").and_then(|s| s.as_str()) {
                Some("broken") => facts.broken += 1,
                Some("modified") => facts.modified += 1,
                Some("valid") => facts.valid += 1,
                Some("unverified") => facts.unverified += 1,
                _ => {}
            }
        }
    }
    Ok(())
}

fn open_store(store: &Store) -> Result<Connection, MemoryFailure> {
    if store.global {
        return db::open_global_memory_query().map_err(map_open_error);
    }
    let repo = store.repo.as_ref().ok_or(MemoryFailure::Unreadable)?;
    db::open_memory_query(repo).map_err(map_open_error)
}

fn map_open_error(e: crate::error::EgError) -> MemoryFailure {
    match e {
        crate::error::EgError::SchemaMismatch { .. } => MemoryFailure::Schema,
        crate::error::EgError::Security(_) => MemoryFailure::Symlink,
        _ => MemoryFailure::Corrupt,
    }
}

fn regular_store(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
}

fn symlink_store(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

fn column_present(conn: &Connection, table: &str, column: &str) -> bool {
    let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) else {
        return false;
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(1)) else {
        return false;
    };
    let names: Vec<String> = rows.flatten().collect();
    names.iter().any(|name| name == column)
}
