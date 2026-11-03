//! `aw doctor`: 3 AM diagnosability — integrity, versions, parse coverage,
//! stale ratio, orphan `.aw/` files, lock leftovers, DB size vs target (CUJ3).

use std::path::Path;

use rusqlite::Connection;

use crate::db;
use crate::error::Result;

pub fn invalidate(root: &Path) -> Result<()> {
    // Serialize with builds through the stable root-directory lock. Removing
    // `.aw/` while a build holds its lock would unlink the lock inode and let
    // a second builder enter a new critical section.
    let _root_lock = db::RootLock::acquire(root)?;
    let dir = db::aw_dir(root);
    if let Ok(meta) = std::fs::symlink_metadata(&dir) {
        if meta.file_type().is_symlink() {
            return Err(crate::error::AwError::Internal(format!(
                "refusing symlinked .aw directory: {}",
                dir.display()
            )));
        }
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
        println!("aw: removed {}", dir.display());
    } else {
        println!("aw: no {} present", dir.display());
    }
    Ok(())
}

pub fn report(root: &Path) -> Result<()> {
    let conn = db::open_for_query(root)?;
    println!("aw: doctor report for {}", root.display());

    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    println!("  integrity: {integrity}");

    let user_version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    println!("  schema version: {user_version}");
    for (key, label) in [
        ("aw_version", "aw version"),
        ("grammar_version", "grammar version"),
        ("built_at", "built at"),
        ("source_rev", "source rev"),
    ] {
        let v: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .ok();
        println!("  {label}: {}", v.unwrap_or_else(|| "<missing>".into()));
    }

    let (total, bad): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(parse_status != 'ok'), 0) FROM files",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let rate = if total > 0 {
        bad as f64 / total as f64
    } else {
        0.0
    };
    println!(
        "  parse coverage: {bad}/{total} files with errors ({:.1}%)",
        rate * 100.0
    );

    let stale = stale_ratio(&conn, root)?;
    println!(
        "  stale ratio: {:.1}% of indexed files changed on disk",
        stale * 100.0
    );

    report_orphans(root);

    let size = std::fs::metadata(db::db_path(root))
        .map(|m| m.len())
        .unwrap_or(0);
    println!("  db size: {size} bytes (target < 52428800)");

    // Payload journal health (Amendment 10): volume only — contents are
    // usage data, not index integrity.
    let (plines, pbytes) = crate::journal::payload_stats(root);
    if plines > 0 {
        println!("  payload journal: {plines} lines, {pbytes} bytes");
    } else {
        println!("  payload journal: absent (no pointer queries journaled yet)");
    }
    Ok(())
}

fn stale_ratio(conn: &Connection, root: &Path) -> Result<f64> {
    let mut stmt = conn.prepare("SELECT path FROM files")?;
    let paths = stmt
        .query_map(rusqlite::params![], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    if paths.is_empty() {
        return Ok(0.0);
    }
    let changed = paths
        .iter()
        .filter(|p| crate::query::file_changed(conn, root, p))
        .count();
    Ok(changed as f64 / paths.len() as f64)
}

fn report_orphans(root: &Path) {
    let dir = db::aw_dir(root);
    let new = db::db_new_path(root);
    if new.exists() {
        println!(
            "  orphan: {} (leftover build — rerun aw build or delete)",
            new.display()
        );
    }
    let lock = dir.join("build.lock");
    // A leftover lock file after a completed build is normal: the advisory
    // flock is released on drop. Only a lock another process still holds is
    // actionable, so probe it instead of testing path existence.
    if let Ok(file) = std::fs::File::open(&lock) {
        if matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)) {
            println!("  lock file: {} held by a running build", lock.display());
        }
    }
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with("-journal") || name.ends_with("-wal") || name.ends_with("-shm") {
            println!("  orphan sidecar: {}", entry.path().display());
        }
    }
}
