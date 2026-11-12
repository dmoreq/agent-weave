//! Derived Unicode FTS index: completeness marker, tokenizer epoch, and the
//! chunked reconcile that repairs rows written by older binaries.

use super::*;
use rusqlite::OptionalExtension;

/// Tokenizer contract version stored in `memory_meta.fts_epoch`.
const MEM_FTS_EPOCH: &str = "2";
/// Rows reconciled per transaction during a v2 backfill.
const MEM_FTS_CHUNK: usize = 5_000;

fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM memory_meta WHERE key = ?1",
        rusqlite::params![key],
        |r| r.get(0),
    )
    .optional()
    .map_err(map_memory_err)
}

fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )
    .map_err(map_memory_err)?;
    Ok(())
}

fn v2_tokenizer_ok(conn: &Connection) -> Result<bool> {
    let sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memories_fts_v2'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_memory_err)?;
    Ok(sql.is_some_and(|s| s.contains("remove_diacritics 2")))
}

/// True only when v2 exists and a clean reconcile completed.
pub(crate) fn memory_fts_v2_usable(conn: &Connection) -> Result<bool> {
    if !super::query_state::object_exists_result(conn, "table", "memories_fts_v2")
        .map_err(map_memory_err)?
    {
        return Ok(false);
    }
    if !super::query_state::object_exists_result(conn, "table", "memory_meta")
        .map_err(map_memory_err)?
    {
        return Ok(false);
    }
    let epoch = meta_get(conn, "fts_epoch")?;
    let complete = meta_get(conn, "fts_v2_complete")?;
    Ok(epoch.as_deref() == Some(MEM_FTS_EPOCH) && complete.as_deref() == Some("1"))
}

fn fts_has_pending(conn: &Connection) -> Result<bool> {
    // The partial `memories_pending` index makes this probe O(1) on a healthy
    // store. Missing v2 rows after a crash are covered by the completeness
    // marker, which is cleared before the first repair chunk.
    let pending: i64 = conn
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM memories
                WHERE superseded_by IS NULL
                  AND (expires_at = 0 OR expires_at > ?1)
                  AND fts_version < 2
                LIMIT 1)",
            rusqlite::params![unix_now()],
            |r| r.get(0),
        )
        .map_err(map_memory_err)?;
    Ok(pending != 0)
}

fn prepare_fts_v2(conn: &Connection, force: bool) -> Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(map_memory_err)?;
    let step = (|| -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memory_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL)",
        )
        .map_err(map_memory_err)?;
        let epoch = meta_get(conn, "fts_epoch")?;
        let tokenizer_ok = v2_tokenizer_ok(conn)?;
        if force || epoch.as_deref() != Some(MEM_FTS_EPOCH) || !tokenizer_ok {
            meta_set(conn, "fts_v2_complete", "0")?;
            meta_set(conn, "fts_epoch", MEM_FTS_EPOCH)?;
            conn.execute_batch(
                "DROP TABLE IF EXISTS memories_fts_v2;
                 CREATE VIRTUAL TABLE memories_fts_v2 USING fts5(text, tokenize='unicode61 remove_diacritics 2');",
            )
            .map_err(map_memory_err)?;
            conn.execute("UPDATE memories SET fts_version = 0", [])
                .map_err(map_memory_err)?;
            let now = unix_now();
            for table in ["memories_fts", "memories_fts_v2"] {
                conn.execute(
                    &format!(
                        "DELETE FROM {table} WHERE rowid NOT IN (
                            SELECT id FROM memories
                            WHERE superseded_by IS NULL
                              AND (expires_at = 0 OR expires_at > ?1))"
                    ),
                    rusqlite::params![now],
                )
                .map_err(map_memory_err)?;
            }
        } else {
            meta_set(conn, "fts_v2_complete", "0")?;
            conn.execute_batch(
                "CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts_v2
                 USING fts5(text, tokenize='unicode61 remove_diacritics 2');",
            )
            .map_err(map_memory_err)?;
        }
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

/// Reconcile one batch. Returns the number of rows indexed; 0 means done.
fn reconcile_fts_chunk(conn: &Connection) -> Result<usize> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(map_memory_err)?;
    let step = (|| -> Result<usize> {
        meta_set(conn, "fts_v2_complete", "0")?;
        let rows: Vec<(i64, String, String, String)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, key, tags, body FROM memories
                     WHERE superseded_by IS NULL
                       AND (expires_at = 0 OR expires_at > ?1)
                       AND (fts_version < 2
                            OR id NOT IN (SELECT rowid FROM memories_fts)
                            OR id NOT IN (SELECT rowid FROM memories_fts_v2))
                     ORDER BY id LIMIT ?2",
                )
                .map_err(map_memory_err)?;
            let mapped = stmt
                .query_map(rusqlite::params![unix_now(), MEM_FTS_CHUNK as i64], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })
                .map_err(map_memory_err)?;
            mapped
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(map_memory_err)?
        };
        if rows.is_empty() {
            return Ok(0);
        }
        for (id, key, tags, body) in &rows {
            let legacy = crate::memory::mem_fts_text_legacy(key, tags, body);
            let unicode = crate::memory::mem_fts_text(key, tags, body);
            for (table, text) in [("memories_fts", &legacy), ("memories_fts_v2", &unicode)] {
                conn.execute(
                    &format!("DELETE FROM {table} WHERE rowid = ?1"),
                    rusqlite::params![id],
                )
                .map_err(map_memory_err)?;
                conn.execute(
                    &format!("INSERT INTO {table}(rowid, text) VALUES (?1, ?2)"),
                    rusqlite::params![id, text],
                )
                .map_err(map_memory_err)?;
            }
            conn.execute(
                "UPDATE memories SET fts_version = 2 WHERE id = ?1",
                rusqlite::params![id],
            )
            .map_err(map_memory_err)?;
        }
        Ok(rows.len())
    })();
    match step {
        Ok(n) => {
            conn.execute_batch("COMMIT").map_err(map_memory_err)?;
            Ok(n)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

fn finish_fts_v2(conn: &Connection) -> Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(map_memory_err)?;
    let step = meta_set(conn, "fts_v2_complete", "1");
    match step {
        Ok(()) => conn.execute_batch("COMMIT").map_err(map_memory_err),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// Reconcile the Unicode index against current memory rows. Returns the number
/// of rows indexed in this call. `force` drops and recreates the index.
pub(crate) fn ensure_memory_fts_v2(conn: &Connection, force: bool) -> Result<u64> {
    if !force && memory_fts_v2_usable(conn)? && !fts_has_pending(conn)? {
        return Ok(0);
    }
    prepare_fts_v2(conn, force)?;
    let mut total = 0u64;
    loop {
        let n = reconcile_fts_chunk(conn)?;
        total += n as u64;
        if n == 0 {
            finish_fts_v2(conn)?;
            break;
        }
    }
    if total > MEM_FTS_CHUNK as u64 {
        eprintln!("eg: memory: reindexed {total} rows so far");
    }
    Ok(total)
}
