//! `aw doctor`: 3 AM diagnosability — integrity, versions, parse coverage,
//! stale ratio, orphan `.aw/` files, lock leftovers, DB size vs target (CUJ3).

use std::path::Path;

use rusqlite::Connection;

use crate::db;
use crate::error::Result;

pub fn invalidate(root: &Path) -> Result<Vec<String>> {
    // Serialize with builds through the same `.aw/build.lock` flock a build
    // holds, so an invalidate cannot start while a build is running.
    let _lock = db::BuildLock::acquire(root)?;
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
        // Companion-design disposition (memory retrieval design, Risks):
        // memory.db is primary data, not a rebuildable index, so the reset
        // keeps every memory.db* file (the store plus any crash-recovery
        // sidecar — deleting a live -journal would corrupt the store) and
        // removes the derived index, journals, locks, and orphans.
        let mut kept_memory = false;
        for entry in std::fs::read_dir(&dir)?.flatten() {
            if entry.file_name().to_string_lossy().starts_with("memory.db") {
                kept_memory = true;
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                std::fs::remove_dir_all(&path)?;
            } else {
                std::fs::remove_file(&path)?;
            }
        }
        return Ok(vec![if kept_memory {
            format!("aw: reset {} (kept primary memory.db)", dir.display())
        } else {
            // Nothing primary survived: the old whole-directory contract holds.
            match std::fs::remove_dir(&dir) {
                Ok(()) => format!("aw: removed {}", dir.display()),
                Err(_) => format!("aw: reset {}", dir.display()),
            }
        }]);
    }
    Ok(vec![format!("aw: no {} present", dir.display())])
}

pub fn report(root: &Path) -> Result<Vec<String>> {
    let conn = db::open_for_query(root)?;
    let mut out = vec![format!("aw: doctor report for {}", root.display())];

    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    out.push(format!("  integrity: {integrity}"));

    let user_version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    out.push(format!("  schema version: {user_version}"));
    for (key, label) in [
        ("aw_version", "aw version"),
        ("grammar_version", "grammar version"),
        ("built_at", "built at"),
        ("source_rev", "source rev"),
    ] {
        let v: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .ok();
        out.push(format!(
            "  {label}: {}",
            v.unwrap_or_else(|| "<missing>".into())
        ));
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
    out.push(format!(
        "  parse coverage: {bad}/{total} files with errors ({:.1}%)",
        rate * 100.0
    ));

    let stale = stale_ratio(&conn, root)?;
    out.push(format!(
        "  stale ratio: {:.1}% of indexed files changed on disk",
        stale * 100.0
    ));

    report_orphans(root, &mut out);

    let size = std::fs::metadata(db::db_path(root))
        .map(|m| m.len())
        .unwrap_or(0);
    out.push(format!("  db size: {size} bytes (target < 52428800)"));

    out.push(memory_report_line(root));

    Ok(out)
}

/// One read-only line about the project-memory store: presence, integrity,
/// current note count, size. Best-effort by contract — a missing, skewed, or
/// corrupt store is reported as a line, never an abort of the index report.
fn memory_report_line(root: &Path) -> String {
    let path = db::memory_db_path(root);
    if !path.exists() {
        return "  memory: absent (aw mem write creates it)".to_string();
    }
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    match db::open_memory_query(root) {
        Ok(conn) => match conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0)) {
            Ok(integrity) if integrity == "ok" => {
                let notes: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM memories WHERE superseded_by IS NULL",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                format!("  memory: integrity ok, {notes} notes, {size} bytes")
            }
            Ok(integrity) => format!("  memory: integrity_check: {integrity} ({size} bytes)"),
            Err(e) => format!("  memory: UNREADABLE ({e}) ({size} bytes)"),
        },
        Err(e) => format!("  memory: UNREADABLE ({e}) ({size} bytes)"),
    }
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

fn report_orphans(root: &Path, out: &mut Vec<String>) {
    let dir = db::aw_dir(root);
    let new = db::db_new_path(root);
    if new.exists() {
        out.push(format!(
            "  orphan: {} (leftover build — rerun aw build or delete)",
            new.display()
        ));
    }
    let lock = dir.join("build.lock");
    // A leftover lock file after a completed build is normal: the advisory
    // flock is released on drop. Only a lock another process still holds is
    // actionable, so probe it instead of testing path existence.
    if std::fs::symlink_metadata(&lock).is_ok_and(|meta| meta.is_file()) {
        if let Ok(file) = std::fs::File::open(&lock) {
            if matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)) {
                out.push(format!(
                    "  lock file: {} held by a running build",
                    lock.display()
                ));
            }
        }
    }
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with("-journal") || name.ends_with("-wal") || name.ends_with("-shm") {
            out.push(format!("  orphan sidecar: {}", entry.path().display()));
        }
    }
}
