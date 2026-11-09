//! Memory read paths: current-row filtering, FTS candidates, id/key
//! lookups, get/delete, export rows, and garbage collection.

use super::*;
use rusqlite::OptionalExtension;

/// Filter bundle shared by the memory read paths: the optional kind/scope
/// restrictions, the `now` clock, and whether the listing extends to
/// superseded revisions. One struct keeps the four call sites from re-ordering
/// or re-spelling the argument list.
pub struct MemFilter {
    pub kind: Option<crate::memory::MemKind>,
    pub scope: Option<crate::memory::MemScope>,
    pub now: i64,
    pub include_superseded: bool,
}

/// The one "current row" rule set, stated in Rust: not superseded (unless the
/// listing extends to superseded revisions), not expired, and matching the
/// optional kind/scope filters. `memory_current_rows` compiles the same rules
/// into the `--all` SQL WHERE clause (kind/scope empty-string filters mirror
/// the `None` arms); `service.rs::mem_ranked` applies this predicate to the
/// ranked fetch, so the two modes cannot drift apart.
pub(crate) fn memory_row_is_current(row: &MemRow, filter: &MemFilter) -> bool {
    if row.superseded_by.is_some() && !filter.include_superseded {
        return false;
    }
    if row.expires_at != 0 && row.expires_at <= filter.now {
        return false;
    }
    if let Some(k) = filter.kind {
        if row.kind != k.as_str() {
            return false;
        }
    }
    if let Some(s) = filter.scope {
        if row.scope != s.as_str() {
            return false;
        }
    }
    true
}

/// Current, non-expired rows (the resolver's candidate universe), newest
/// first. `include_superseded` extends the listing with superseded revisions
/// (the `--all --include-superseded` view); expired rows stay invisible in
/// every mode. Empty filter values match everything; all filters are bound
/// parameters, never string interpolation. The WHERE clause mirrors
/// `memory_row_is_current` (same rule set, SQL spelling; a unit test holds
/// the two row-by-row equal).
pub fn memory_current_rows(
    conn: &Connection,
    filter: &MemFilter,
    limit: Option<usize>,
) -> Result<Vec<MemRow>> {
    // include_superseded=0 keeps only current rows; =1 lists everything
    // (superseded rows included, tagged at render time). `limit` is the
    // export-side cap; -1 (`None`) means no SQL LIMIT, which the listing paths
    // use because they cap at render time.
    let superseded_clause = "AND (superseded_by IS NULL OR ?4 = 1)";
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {MEM_ROW_SQL} FROM memories
             WHERE (expires_at = 0 OR expires_at > ?1)
               {superseded_clause}
               AND (?2 = '' OR kind = ?2)
               AND (?3 = '' OR scope = ?3)
             ORDER BY created_at DESC, key ASC, rev DESC
             LIMIT ?5"
        ))
        .map_err(map_memory_err)?;
    let rows = stmt
        .query_map(
            rusqlite::params![
                filter.now,
                filter
                    .kind
                    .map(crate::memory::MemKind::as_str)
                    .unwrap_or(""),
                filter
                    .scope
                    .map(crate::memory::MemScope::as_str)
                    .unwrap_or(""),
                i64::from(filter.include_superseded),
                limit.map_or(-1, |n| n as i64),
            ],
            mem_row,
        )
        .map_err(map_memory_err)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(map_memory_err)
}

/// FTS candidates: (rowid, bm25 rank) best-first, capped. Smaller rank is the
/// better match; the presentation layer inverts via min-max normalization.
pub fn memory_fts_candidates(
    conn: &Connection,
    match_query: &str,
    cap: usize,
) -> Result<Vec<(i64, f64)>> {
    if match_query.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare(
            "SELECT rowid, bm25(memories_fts) FROM memories_fts
             WHERE memories_fts MATCH ?1 ORDER BY rank, rowid LIMIT ?2",
        )
        .map_err(map_memory_err)?;
    let rows = stmt
        .query_map(rusqlite::params![match_query, cap as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        })
        .map_err(map_memory_err)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(map_memory_err)
}

/// Rows by id (candidate fetch; at most CANDIDATE_CAP ids).
pub fn memory_rows_by_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<MemRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = sql_placeholders(ids.len());
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {MEM_ROW_SQL} FROM memories WHERE id IN ({placeholders})"
        ))
        .map_err(map_memory_err)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(ids.iter()), mem_row)
        .map_err(map_memory_err)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(map_memory_err)
}

/// Candidates from code-anchored `refs`: returns current unexpired rows whose
/// `refs` field matches any of the query `tokens` via substring matching.
pub fn memory_ref_candidate_rows(
    conn: &Connection,
    tokens: &[String],
    cap: usize,
) -> Result<Vec<MemRow>> {
    if tokens.is_empty() {
        return Ok(Vec::new());
    }
    let clauses: Vec<String> = (1..=tokens.len())
        .map(|i| format!("refs LIKE ?{i}"))
        .collect();
    let sql = format!(
        "SELECT {MEM_ROW_SQL} FROM memories
         WHERE superseded_by IS NULL AND refs != '' AND ({})
         ORDER BY id DESC LIMIT ?{}",
        clauses.join(" OR "),
        tokens.len() + 1
    );
    let mut params: Vec<rusqlite::types::Value> = tokens
        .iter()
        .map(|t| rusqlite::types::Value::Text(format!("%{t}%")))
        .collect();
    params.push(rusqlite::types::Value::Integer(cap as i64));
    let mut stmt = conn.prepare(&sql).map_err(map_memory_err)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), mem_row)
        .map_err(map_memory_err)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(map_memory_err)
}

/// The subset of `keys` that carry a current, unexpired row — the existence
/// probe behind the merged-store shadowing check (one IN query replaces a
/// per-key `memory_get`, at most CANDIDATE_CAP keys). Matches `memory_get`'s
/// row visibility: current revision only, zero `expires_at` never expires.
pub fn memory_current_keys(conn: &Connection, keys: &[String]) -> Result<Vec<String>> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = sql_placeholders(keys.len());
    let mut values: Vec<rusqlite::types::Value> = keys
        .iter()
        .map(|k| rusqlite::types::Value::Text(k.clone()))
        .collect();
    values.push(rusqlite::types::Value::Integer(unix_now()));
    let now_param = values.len();
    let mut stmt = conn
        .prepare(&format!(
            "SELECT DISTINCT key FROM memories
             WHERE key IN ({placeholders})
               AND superseded_by IS NULL
               AND (expires_at = 0 OR expires_at > ?{now_param})"
        ))
        .map_err(map_memory_err)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(values.iter()), |r| {
            r.get::<_, String>(0)
        })
        .map_err(map_memory_err)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(map_memory_err)
}

/// Current row for one exact key plus its total revision count.
pub fn memory_get(conn: &Connection, key: &str) -> Result<Option<(MemRow, i64)>> {
    let row = conn
        .query_row(
            &format!(
                "SELECT {MEM_ROW_SQL} FROM memories
                 WHERE key = ?1 AND superseded_by IS NULL LIMIT 1"
            ),
            rusqlite::params![key],
            mem_row,
        )
        .optional()
        .map_err(map_memory_err)?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.expires_at != 0 && row.expires_at <= unix_now() {
        return Ok(None);
    }
    let revs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE key = ?1",
            rusqlite::params![key],
            |r| r.get(0),
        )
        .map_err(map_memory_err)?;
    Ok(Some((row, revs)))
}

/// Hard-delete every revision of one key plus their FTS rows, one
/// transaction. Returns None for an unknown key (nothing deleted).
pub fn memory_delete(conn: &mut Connection, key: &str) -> Result<Option<i64>> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(map_memory_err)?;
    let ids: Vec<i64> = {
        let mut stmt = tx
            .prepare("SELECT id FROM memories WHERE key = ?1")
            .map_err(map_memory_err)?;
        let rows = stmt
            .query_map(rusqlite::params![key], |r| r.get::<_, i64>(0))
            .map_err(map_memory_err)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(map_memory_err)?
    };
    if ids.is_empty() {
        return Ok(None); // tx dropped -> rollback, nothing written
    }
    delete_memory_ids(&tx, &ids)?;
    let removed = ids.len() as i64;
    tx.commit().map_err(map_memory_err)?;
    Ok(Some(removed))
}

/// Durable (non-superseded) memory note count. Single source of the
/// tombstone semantics (`superseded_by IS NULL`), shared by `memory_gc` and
/// the `doctor` memory report line; `&Transaction` call sites deref to
/// `&Connection`.
pub fn durable_note_count(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE superseded_by IS NULL",
        [],
        |r| r.get(0),
    )
}

/// gc purge (Phase 2 retention rule), one `BEGIN IMMEDIATE` transaction:
/// expired session notes (read-time invisible already; this reclaims disk)
/// lose their rows AND their FTS rows; superseded revisions whose
/// `superseded_at` is older than the 30-day retention constant are purged
/// (they have no FTS rows — deleted at supersede time — so a rev superseded
/// yesterday but created 40 days ago survives, clocked by supersession, not
/// creation). Returns (expired removed, superseded removed, durable rows left).
pub fn memory_gc(conn: &mut Connection, now: i64) -> Result<(i64, i64, i64)> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(map_memory_err)?;
    let expired_ids: Vec<i64> = {
        let mut stmt = tx
            .prepare("SELECT id FROM memories WHERE expires_at > 0 AND expires_at <= ?1")
            .map_err(map_memory_err)?;
        let rows = stmt
            .query_map(rusqlite::params![now], |r| r.get::<_, i64>(0))
            .map_err(map_memory_err)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(map_memory_err)?
    };
    delete_memory_ids(&tx, &expired_ids)?;
    let retention_horizon = now - crate::memory::RETENTION_DAYS * 86_400;
    let superseded = tx
        .execute(
            "DELETE FROM memories
             WHERE superseded_by IS NOT NULL AND superseded_at > 0 AND superseded_at <= ?1",
            rusqlite::params![retention_horizon],
        )
        .map_err(map_memory_err)? as i64;
    let durable: i64 = durable_note_count(&tx).map_err(map_memory_err)?;
    tx.commit().map_err(map_memory_err)?;
    Ok((expired_ids.len() as i64, superseded, durable))
}

/// Chunked delete: a batch with more ids than SQLite's variable limit must
/// still delete (one placeholder per id would fail to prepare), so ids go
/// through `SQL_CHUNK` batches covering both the row and its FTS entry.
fn delete_memory_ids(tx: &rusqlite::Transaction<'_>, ids: &[i64]) -> Result<()> {
    for_each_chunk(tx, ids, |conn, placeholders, chunk| {
        conn.execute(
            &format!("DELETE FROM memories WHERE id IN ({placeholders})"),
            rusqlite::params_from_iter(chunk.iter()),
        )
        .map_err(map_memory_err)?;
        conn.execute(
            &format!("DELETE FROM memories_fts WHERE rowid IN ({placeholders})"),
            rusqlite::params_from_iter(chunk.iter()),
        )
        .map_err(map_memory_err)?;
        Ok(())
    })
}
