//! rusqlite layer: schema lifecycle, meta manifest, build lock (flock),
//! streamed chunked inserts, and the full-atomic-replacement build policy.
//!
//! Build semantics (spec, single rule): `kg build` always produces a full
//! atomic replacement — extract into `.kg/index.db.new` (fresh schema), close,
//! then rename(2) onto `index.db`. A concurrent reader sees the old or the new
//! DB, never a missing one. There is no delete step; orphan sidecars are
//! cleaned at build start and by `doctor`.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rusqlite::{Connection, OpenFlags};

use crate::error::{KgError, Result};
use crate::extract::{CarriedIndex, CarriedSymbol, NewSymbol, SCHEMA_VERSION};

/// Compile-time grammar identity, derived from the Cargo.lock-pinned grammar
/// crate version (tree-sitter versioning rule: runtime + grammars upgrade in
/// one commit, golden fixture replay must stay identical).
pub const GRAMMAR_VERSION: &str = "py-0.25.0+ts-0.23.2+js-0.25.0+rs-0.24.2";

pub fn kg_dir(root: &Path) -> PathBuf {
    root.join(".kg")
}

pub fn db_path(root: &Path) -> PathBuf {
    kg_dir(root).join("index.db")
}

pub fn db_new_path(root: &Path) -> PathBuf {
    kg_dir(root).join("index.db.new")
}

/// Write the tool-owned `.kg/.gitignore` (`*`) on first build: the whole
/// directory disappears from `git status` without touching the repo's own
/// .gitignore (allowed write path per invariant 3's constant list).
pub fn ensure_kg_gitignore(root: &Path) -> Result<()> {
    let dir = kg_dir(root);
    fs::create_dir_all(&dir)?;
    let marker = dir.join(".gitignore");
    if !marker.exists() {
        fs::write(&marker, "*\n")?;
    }
    Ok(())
}

///Exclusive flock over `.kg/build.lock` for the whole build (CUJ6).
pub struct BuildLock {
    file: File,
}

impl BuildLock {
    pub fn acquire(root: &Path) -> Result<BuildLock> {
        let dir = kg_dir(root);
        fs::create_dir_all(&dir)?;
        let lock_path = dir.join("build.lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .read(true)
            .open(&lock_path)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                let pid = fs::read_to_string(&lock_path)
                    .ok()
                    .and_then(|s| {
                        s.lines().next().and_then(|l| {
                            l.split_whitespace().next().and_then(|p| p.parse::<u32>().ok())
                        })
                    })
                    .unwrap_or(0);
                return Err(KgError::Locked(pid));
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
/// both map to the contract message via `KgError::from(rusqlite::Error)`.
/// Deep integrity remains `kg doctor` (`PRAGMA integrity_check`).
pub fn open_for_query(root: &Path) -> Result<Connection> {
    let path = db_path(root);
    if !path.exists() {
        return Err(KgError::NoResults(format!(
            "no .kg/index.db under {} — run kg build",
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
        return Err(KgError::SchemaMismatch { have, want: SCHEMA_VERSION });
    }
    Ok(())
}

/// Fresh DB at `.kg/index.db.new` with schema + user_version stamped.
pub fn create_fresh(root: &Path) -> Result<Connection> {
    ensure_kg_gitignore(root)?;
    let path = db_new_path(root);
    let _ = fs::remove_file(&path);
    let conn = Connection::open(&path)?;
    conn.execute_batch(include_str!("../schema.sql"))?;
    // Belt and braces: the schema batch declares user_version = 1; enforce it
    // via the pragma API so the contract never depends on batch semantics.
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(conn)
}

/// Fast path gate: a usable previous index under the SAME schema AND grammar
/// versions (any mismatch disables the fast path for the whole build).
pub fn probe_old(root: &Path) -> Result<Option<Connection>> {
    let old_path = db_path(root);
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
    Ok(rows.into_iter().map(|(p, m, s, h)| (p, (m, s, h))).collect())
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
        let c: i64 =
            old.query_row(&sql, rusqlite::params_from_iter(chunk.iter()), |r| r.get(0))?;
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
    let mut dirty: HashSet<String> = changed.iter().cloned().collect();
    loop {
        let mut frontier: Vec<String> = dirty.iter().cloned().collect();
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
            let rows =
                stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| r.get::<_, String>(0))?;
            for r in rows {
                let p = r?;
                if !dirty.contains(&p) {
                    new_paths.insert(p);
                }
            }
        }
        if new_paths.is_empty() {
            break;
        }
        dirty.extend(new_paths);
    }
    Ok(dirty)
}

/// Symbols + inherits pairs of the carried set, for resolution tables.
pub fn load_carried(old: &Connection, rels: &HashSet<String>) -> Result<CarriedIndex> {
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
    let mut stmt = old.prepare(
        "SELECT s.qualname, e.dst_name
         FROM edges e JOIN symbols s ON s.id = e.src_id
         WHERE e.kind = 'inherits' AND e.dst_name IS NOT NULL",
    )?;
    let pairs = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (sub, base) in pairs {
        inherits.push((sub, base));
    }
    Ok(CarriedIndex { symbols, inherits })
}

/// Copy rows for files matching the fast-path keys. `unchanged` rows carry
/// (rel, old_mtime, size, current_mtime): the SELECT identifies the old row by
/// (path, old_mtime, size); hash-carried rows (Amendment 9) then get their
/// stored mtime refreshed to the fresh checkout's stat so the staleness
/// contract stays truthful. Attaches the old DB itself (SQLite ATTACH takes no
/// bound parameters). Returns rel -> old file id for reuse.
pub fn carry_forward_unchanged(
    new: &Connection,
    root: &Path,
    unchanged: &[(String, i64, i64, i64)],
) -> Result<Vec<(String, i64)>> {
    let old_path = db_path(root);
    let quoted = old_path.to_string_lossy().replace('\'', "''");
    new.execute_batch(&format!("ATTACH DATABASE '{quoted}' AS olddb"))?;
    new.execute_batch("BEGIN")?;
    let mut reused = Vec::new();
    {
        let mut stmt = new.prepare(
            "INSERT INTO files(id, path, size, mtime, content_hash, parse_status, error_nodes, unresolved)
             SELECT id, path, size, mtime, content_hash, parse_status, error_nodes, unresolved
             FROM olddb.files WHERE path = ?1 AND mtime = ?2 AND size = ?3",
        )?;
        let mut refresh_mtime =
            new.prepare("UPDATE files SET mtime = ?1 WHERE path = ?2")?;
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
        "INSERT INTO symbols(id, kind, name, qualname, file_id, line_start, line_end)
         SELECT id, kind, name, qualname, file_id, line_start, line_end
         FROM olddb.symbols WHERE file_id IN (SELECT id FROM files);
         INSERT INTO edges(id, src_id, dst_id, dst_name, kind, confidence, site_line)
         SELECT id, src_id, dst_id, dst_name, kind, confidence, site_line
         FROM olddb.edges
         WHERE src_id IN (SELECT id FROM symbols)
           AND (dst_id IS NULL OR dst_id IN (SELECT id FROM symbols));",
    )?;
    new.execute_batch("COMMIT")?;
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
}

pub fn insert_files(conn: &Connection, rows: &[FilesRow]) -> Result<Vec<i64>> {
    let mut ids = Vec::with_capacity(rows.len());
    for chunk in rows.chunks(500) {
        conn.execute_batch("BEGIN")?;
        {
            let mut stmt = conn.prepare(
                "INSERT INTO files(path, size, mtime, content_hash, parse_status, error_nodes, unresolved)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for r in chunk {
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
        }
        conn.execute_batch("COMMIT")?;
    }
    Ok(ids)
}

pub fn insert_symbols(conn: &Connection, symbols: &[NewSymbol], file_ids: &[i64]) -> Result<Vec<i64>> {
    let mut ids = Vec::with_capacity(symbols.len());
    for chunk in symbols.chunks(500) {
        conn.execute_batch("BEGIN")?;
        {
            let mut stmt = conn.prepare(
                "INSERT INTO symbols(kind, name, qualname, file_id, line_start, line_end)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for s in chunk {
                stmt.execute(rusqlite::params![
                    s.kind,
                    s.name,
                    s.qualname,
                    file_ids[s.file_idx],
                    s.line_start as i64,
                    s.line_end as i64
                ])?;
                ids.push(conn.last_insert_rowid());
            }
        }
        conn.execute_batch("COMMIT")?;
    }
    Ok(ids)
}

/// Fully resolved edge row: endpoint ids already mapped (fresh -> inserted id,
/// carried -> previous row id).
#[derive(Debug)]
pub struct EdgeRow {
    pub src_id: i64,
    pub dst_id: Option<i64>,
    pub dst_name: String,
    pub kind: &'static str,
    pub confidence: &'static str,
    pub site_line: usize,
}

pub fn insert_edge_rows(conn: &Connection, rows: &[EdgeRow]) -> Result<()> {
    for chunk in rows.chunks(500) {
        conn.execute_batch("BEGIN")?;
        {
            let mut stmt = conn.prepare(
                "INSERT INTO edges(src_id, dst_id, dst_name, kind, confidence, site_line)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
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
        conn.execute_batch("COMMIT")?;
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
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    let Some(mut rev) = head else { return "unknown".to_string() };
    let dirty = std::process::Command::new("git")
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
    fs::metadata(path).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_nanos() as i64)
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
        let dir = std::env::temp_dir().join(format!("kg-db-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let conn = create_fresh(&dir).unwrap();
        let v: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        drop(conn);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_index_is_exit1_not_panic() {
        let dir = std::env::temp_dir().join(format!("kg-db-missing-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let err = open_for_query(&dir).unwrap_err();
        assert_eq!(err.exit_code(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupted_index_is_exit3() {
        let dir = std::env::temp_dir().join(format!("kg-db-corrupt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".kg")).unwrap();
        fs::write(db_path(&dir), b"this is not sqlite at all").unwrap();
        let err = open_for_query(&dir).unwrap_err();
        assert_eq!(err.exit_code(), 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn schema_skew_is_exit5() {
        let dir = std::env::temp_dir().join(format!("kg-db-skew-{}", std::process::id()));
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
}
