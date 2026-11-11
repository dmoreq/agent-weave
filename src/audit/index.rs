//! Per-repository index facts for audit. Non-aborting: one bad index becomes a
//! check result, never an exit that hides the rest of the report.

use std::path::{Path, PathBuf};

use crate::db;

pub struct RepoFacts {
    pub path: PathBuf,
    pub alias: String,
    pub state: IndexState,
}

pub enum IndexState {
    Ok(IndexFacts),
    Empty(IndexFacts),
    Missing,
    Symlink,
    Tracked,
    Corrupt,
    Schema,
    TooLarge,
    BuildInProgress,
}

pub struct IndexFacts {
    pub files: i64,
    pub symbols: i64,
    pub parse_errors: i64,
    pub stale_ratio: f64,
    pub db_bytes: u64,
    pub built_at: Option<i64>,
}

pub fn collect(paths: &[PathBuf], _now: i64) -> Vec<RepoFacts> {
    paths
        .iter()
        .map(|path| RepoFacts {
            path: path.clone(),
            alias: super::repo_alias(path),
            state: snapshot(path),
        })
        .collect()
}

fn snapshot(root: &Path) -> IndexState {
    if db::reject_tracked_eg_dir(root).is_err() {
        return IndexState::Tracked;
    }
    let conn = match db::try_open_index(root) {
        Ok(conn) => conn,
        Err(db::IndexOpenError::Missing) => return IndexState::Missing,
        Err(db::IndexOpenError::Symlink) => return IndexState::Symlink,
        Err(db::IndexOpenError::Schema { .. }) => return IndexState::Schema,
        Err(db::IndexOpenError::Corrupt) => return IndexState::Corrupt,
    };
    if build_lock_held(root) {
        return IndexState::BuildInProgress;
    }
    let db_bytes = db::file_size(&db::db_path(root));
    let integrity: std::result::Result<String, _> = if db_bytes > 50 * 1024 * 1024 {
        conn.query_row("PRAGMA quick_check", [], |r| r.get(0))
    } else {
        conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))
    };
    match integrity {
        Ok(v) if v == "ok" => {}
        _ => return IndexState::Corrupt,
    }
    let counts: std::result::Result<(i64, i64, i64), _> = conn.query_row(
        "SELECT COUNT(*), (SELECT COUNT(*) FROM symbols), COALESCE(SUM(parse_status != 'ok'), 0) FROM files",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    );
    let Ok((files, symbols, parse_errors)) = counts else {
        return IndexState::Corrupt;
    };
    let built_at = db::meta_get(&conn, db::meta_keys::BUILT_AT).and_then(|v| v.parse::<i64>().ok());
    let mut facts = IndexFacts {
        files,
        symbols,
        parse_errors,
        stale_ratio: 0.0,
        db_bytes,
        built_at,
    };
    if files == 0 {
        return IndexState::Empty(facts);
    }
    let mut stmt = match conn.prepare("SELECT path FROM files") {
        Ok(stmt) => stmt,
        Err(_) => return IndexState::Corrupt,
    };
    let rows = match stmt.query_map([], |r| r.get::<_, String>(0)) {
        Ok(rows) => rows,
        Err(_) => return IndexState::Corrupt,
    };
    let mut paths: Vec<String> = Vec::new();
    for row in rows {
        match row {
            Ok(p) => paths.push(p),
            Err(_) => return IndexState::Corrupt,
        }
    }
    if paths.len() > 10_000 {
        return IndexState::TooLarge;
    }
    let ctx = crate::query::QueryCtx { conn: &conn, root };
    let changed = paths
        .iter()
        .filter(|p| crate::query::file_changed(&ctx, p))
        .count();
    facts.stale_ratio = changed as f64 / paths.len() as f64;
    IndexState::Ok(facts)
}

/// A build lock held by another process makes the index a moving target.
/// Probe it instead of acquiring it: audit is read-only.
fn build_lock_held(root: &Path) -> bool {
    let lock = db::eg_dir(root).join("build.lock");
    if !std::fs::symlink_metadata(&lock).is_ok_and(|m| m.is_file()) {
        return false;
    }
    match std::fs::File::open(&lock) {
        Ok(file) => matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
        Err(_) => false,
    }
}
