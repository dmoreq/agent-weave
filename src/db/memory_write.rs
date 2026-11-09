//! Memory rows and revisioned writes: content dedup, revision chains,
//! supersession (same-key and cross-key), and the unix clock.

use super::*;
use rusqlite::OptionalExtension;

/// Unix seconds (writes stamp created_at/superseded_at).
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// One memory note row as returned to the presentation layer. `superseded_by`
/// is read only for the `--include-superseded` listing tag; `superseded_at`
/// stays internal (the gc retention clock).
#[derive(Debug, Clone)]
pub struct MemRow {
    pub id: i64,
    pub key: String,
    pub rev: i64,
    pub kind: String,
    pub scope: String,
    pub session: String,
    pub body: String,
    pub tags: String,
    pub refs: String,
    pub source: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub superseded_by: Option<i64>,
}

pub(super) const MEM_ROW_SQL: &str =
    "id, key, rev, kind, scope, session, body, tags, refs, source, created_at, expires_at, superseded_by";

pub(super) fn mem_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<MemRow> {
    Ok(MemRow {
        id: r.get(0)?,
        key: r.get(1)?,
        rev: r.get(2)?,
        kind: r.get(3)?,
        scope: r.get(4)?,
        session: r.get(5)?,
        body: r.get(6)?,
        tags: r.get(7)?,
        refs: r.get(8)?,
        source: r.get(9)?,
        created_at: r.get(10)?,
        expires_at: r.get(11)?,
        superseded_by: r.get(12)?,
    })
}

/// Validated note ready for the write transaction (built by the service layer
/// through `memory::normalize_note`). `expires_at` is 0 for durable notes and
/// the absolute unix deadline for session notes; `supersedes` names a
/// different key whose current note this write replaces.
pub struct MemNoteInput<'a> {
    pub key: &'a str,
    pub kind: &'a str,
    pub tags: &'a str,
    pub refs: &'a str,
    pub scope: &'a str,
    pub session: &'a str,
    pub body: &'a str,
    pub source: &'a str,
    pub expires_at: i64,
    pub supersedes: Option<&'a str>,
    pub touch: bool,
}

/// Write outcome: a new revision, an idempotent duplicate, or a refreshed timestamp.
#[derive(Debug)]
pub enum MemWriteOutcome {
    Written {
        rev: i64,
        superseded_rev: Option<i64>,
    },
    Duplicate {
        rev: i64,
    },
    Touched {
        rev: i64,
    },
}

/// `execute_batch` reports statement failures as `SqlInputError`; `execute`
/// reports them as `SqliteFailure`. Both shapes must classify "already
/// exists" so a concurrent creator can fall through to the version gate.
pub(super) fn is_already_exists(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(_, Some(msg)) | rusqlite::Error::SqlInputError { msg, .. }
            if msg.contains("already exists")
    )
}

fn is_unique_violation(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(ffi, _)
            if ffi.code == rusqlite::ffi::ErrorCode::ConstraintViolation
    )
}

/// Insert one note under BEGIN IMMEDIATE: the write lock is taken BEFORE the
/// current rev is read and held across read-compute-insert, so racing writers
/// serialize on busy_timeout (or exit 4) and never compute the same rev. The
/// FTS delta (delete superseded rowid, insert the new one) runs in the same
/// transaction. A retried write with identical content is a successful no-op.
pub fn memory_write(
    conn: &mut Connection,
    note: &MemNoteInput<'_>,
    now: i64,
) -> Result<MemWriteOutcome> {
    // One in-transaction retry on a UNIQUE(key, rev) failure (a defect by
    // construction — rev is computed inside the lock — but never exit 2/5).
    let mut last_error = None;
    for _ in 0..2 {
        match try_write_revision(conn, note, now)? {
            WriteAttempt::Written(outcome) => return Ok(outcome),
            WriteAttempt::Retry(e) => last_error = Some(e),
        }
    }
    match last_error {
        Some(e) => Err(map_memory_err(e)),
        None => Err(AwError::Internal(
            "memory write: no attempt completed".to_string(),
        )),
    }
}

enum WriteAttempt {
    Written(MemWriteOutcome),
    Retry(rusqlite::Error),
}

/// The current memory revision row for a key: row id, revision number, and
/// the revision's content hash. Named so the three i64 columns cannot be
/// swapped positionally in `try_write_revision`.
struct Revision {
    id: i64,
    rev: i64,
    content_hash: i64,
}

fn try_write_revision(
    conn: &mut Connection,
    note: &MemNoteInput<'_>,
    now: i64,
) -> Result<WriteAttempt> {
    let hash =
        crate::memory::mem_content_hash(note.kind, note.body, note.tags, note.refs, note.scope)
            as i64;
    let fts_text = crate::memory::mem_fts_text(note.key, note.tags, note.body);
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(map_memory_err)?;
    let current = current_revision(&tx, note.key)?;
    if let Some(row) = &current {
        if row.content_hash == hash {
            if note.touch {
                tx.execute(
                    "UPDATE memories SET created_at = ?1 WHERE id = ?2",
                    rusqlite::params![now, row.id],
                )
                .map_err(map_memory_err)?;
                tx.commit().map_err(map_memory_err)?;
                return Ok(WriteAttempt::Written(MemWriteOutcome::Touched {
                    rev: row.rev,
                }));
            }
            // Idempotent replay: nothing written, nothing to commit.
            drop(tx);
            return Ok(WriteAttempt::Written(MemWriteOutcome::Duplicate {
                rev: row.rev,
            }));
        }
    }
    let new_rev = current.as_ref().map_or(1, |row| row.rev + 1);
    if let Err(e) = insert_revision(&tx, note, now, hash, new_rev) {
        if is_unique_violation(&e) {
            return Ok(WriteAttempt::Retry(e));
        }
        return Err(map_memory_err(e));
    }
    let new_id = tx.last_insert_rowid();
    if let Some(row) = &current {
        supersede_current(&tx, new_id, now, row.id)?;
    }
    if let Some(target) = note.supersedes {
        supersede_cross_key(&tx, new_id, now, target)?;
    }
    tx.execute(
        "INSERT INTO memories_fts(rowid, text) VALUES (?1, ?2)",
        rusqlite::params![new_id, fts_text],
    )
    .map_err(map_memory_err)?;
    tx.commit().map_err(map_memory_err)?;
    Ok(WriteAttempt::Written(MemWriteOutcome::Written {
        rev: new_rev,
        superseded_rev: current.as_ref().map(|row| row.rev),
    }))
}

fn insert_revision(
    tx: &rusqlite::Transaction<'_>,
    note: &MemNoteInput<'_>,
    now: i64,
    hash: i64,
    rev: i64,
) -> rusqlite::Result<usize> {
    tx.execute(
        "INSERT INTO memories(key, rev, kind, scope, session, body, tags, refs, content_hash, source, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            note.key,
            rev,
            note.kind,
            note.scope,
            note.session,
            note.body,
            note.tags,
            note.refs,
            hash,
            note.source,
            now,
            note.expires_at,
        ],
    )
}

fn current_revision(tx: &rusqlite::Transaction<'_>, key: &str) -> Result<Option<Revision>> {
    tx.query_row(
        "SELECT id, rev, content_hash FROM memories WHERE key = ?1 ORDER BY rev DESC LIMIT 1",
        rusqlite::params![key],
        |r| {
            Ok(Revision {
                id: r.get(0)?,
                rev: r.get(1)?,
                content_hash: r.get(2)?,
            })
        },
    )
    .optional()
    .map_err(map_memory_err)
}

/// Mark one revision superseded and drop its FTS row (the caller inserts the
/// new revision's FTS row). Shared by same-key and cross-key supersession so
/// both paths apply the same invariant.
fn supersede_current(
    tx: &rusqlite::Transaction<'_>,
    new_id: i64,
    now: i64,
    old_id: i64,
) -> Result<()> {
    tx.execute(
        "UPDATE memories SET superseded_by = ?1, superseded_at = ?2 WHERE id = ?3",
        rusqlite::params![new_id, now, old_id],
    )
    .map_err(map_memory_err)?;
    tx.execute(
        "DELETE FROM memories_fts WHERE rowid = ?1",
        rusqlite::params![old_id],
    )
    .map_err(map_memory_err)?;
    Ok(())
}

/// Cross-key supersession inside the caller's transaction: the target's
/// current revision becomes superseded and its FTS row leaves the index with
/// it. Same invariant as same-key supersession; without the FTS delete `mem
/// query` would return a note that `mem get` calls unknown. An unknown target
/// rolls the whole write back (exit 1). A replayed identical write never
/// reaches here, so the side effects do not re-apply.
fn supersede_cross_key(
    tx: &rusqlite::Transaction<'_>,
    new_id: i64,
    now: i64,
    target: &str,
) -> Result<()> {
    let target_id: Option<i64> = tx
        .query_row(
            "SELECT id FROM memories WHERE key = ?1 AND superseded_by IS NULL LIMIT 1",
            rusqlite::params![target],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_memory_err)?;
    let Some(target_id) = target_id else {
        return Err(AwError::NoResults(format!(
            "unknown memory key '{target}' — nothing superseded, write rolled back"
        )));
    };
    supersede_current(tx, new_id, now, target_id)
}

/// Refresh `created_at = now` on the current active revision of `key`.
/// Returns `Ok(Some(rev))` if active revision was updated, or `Ok(None)` if key does not exist.
pub fn memory_touch(conn: &mut Connection, key: &str, now: i64) -> Result<Option<i64>> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(map_memory_err)?;
    let current = current_revision(&tx, key)?;
    let Some(rev_info) = current else {
        return Ok(None);
    };
    tx.execute(
        "UPDATE memories SET created_at = ?1 WHERE id = ?2",
        rusqlite::params![now, rev_info.id],
    )
    .map_err(map_memory_err)?;
    tx.commit().map_err(map_memory_err)?;
    Ok(Some(rev_info.rev))
}
