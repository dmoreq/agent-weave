//! Dual-FTS invariants: completeness marker, repair, epoch rebuild, candidate
//! filtering, and migration parity.

use super::test_support::remove_tree;
use super::*;
use crate::memory::{mem_fts_text, TimeWindow};
use rusqlite::OptionalExtension;

fn usable(conn: &Connection) -> bool {
    super::memory_fts::memory_fts_v2_usable(conn).unwrap()
}

fn meta(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM memory_meta WHERE key = ?1",
        rusqlite::params![key],
        |r| r.get(0),
    )
    .optional()
    .unwrap()
}

#[test]
fn v2_usable_requires_epoch_and_complete_marker() {
    let root = temp_root("mem-v2-usable");
    let mut conn = open_memory_store(&root).unwrap();
    crate::db::memory_write(
        &mut conn,
        &super::memory_tests::note("k", "body", "fact"),
        unix_now(),
    )
    .unwrap();
    assert!(usable(&conn));
    conn.execute(
        "UPDATE memory_meta SET value = '0' WHERE key = 'fts_v2_complete'",
        [],
    )
    .unwrap();
    assert!(!usable(&conn));
    conn.execute(
        "UPDATE memory_meta SET value = '1' WHERE key = 'fts_v2_complete'",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE memory_meta SET value = '1' WHERE key = 'fts_epoch'",
        [],
    )
    .unwrap();
    assert!(!usable(&conn));
    drop(conn);
    remove_tree(&root);
}

#[test]
fn repairs_old_binary_row() {
    let root = temp_root("mem-v2-repair");
    let mut conn = open_memory_store(&root).unwrap();
    let now = unix_now();
    crate::db::memory_write(
        &mut conn,
        &super::memory_tests::note("k", "Bộ nhớ dự án", "fact"),
        now,
    )
    .unwrap();
    let id: i64 = conn
        .query_row("SELECT id FROM memories WHERE key = 'k'", [], |r| r.get(0))
        .unwrap();
    conn.execute(
        "DELETE FROM memories_fts_v2 WHERE rowid = ?1",
        rusqlite::params![id],
    )
    .unwrap();
    conn.execute(
        "UPDATE memories SET fts_version = 0 WHERE id = ?1",
        rusqlite::params![id],
    )
    .unwrap();
    let n = super::memory_fts::ensure_memory_fts_v2(&conn, false).unwrap();
    assert_eq!(n, 1);
    let version: i64 = conn
        .query_row(
            "SELECT fts_version FROM memories WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(version, 2);
    assert!(usable(&conn));
    drop(conn);
    remove_tree(&root);
}

#[test]
fn epoch_change_forces_rebuild() {
    let root = temp_root("mem-v2-epoch");
    let mut conn = open_memory_store(&root).unwrap();
    crate::db::memory_write(
        &mut conn,
        &super::memory_tests::note("k", "Bộ nhớ dự án", "fact"),
        unix_now(),
    )
    .unwrap();
    conn.execute(
        "UPDATE memory_meta SET value = '1' WHERE key = 'fts_epoch'",
        [],
    )
    .unwrap();
    let n = super::memory_fts::ensure_memory_fts_v2(&conn, false).unwrap();
    assert_eq!(n, 1);
    assert_eq!(meta(&conn, "fts_epoch").as_deref(), Some("2"));
    assert_eq!(meta(&conn, "fts_v2_complete").as_deref(), Some("1"));
    let text: String = conn
        .query_row("SELECT text FROM memories_fts_v2", [], |r| r.get(0))
        .unwrap();
    assert_eq!(text, mem_fts_text("k", "", "Bộ nhớ dự án"));
    drop(conn);
    remove_tree(&root);
}

#[test]
fn superseded_rows_do_not_starve_the_candidate_cap() {
    let root = temp_root("mem-v2-cap");
    let conn = open_memory_store(&root).unwrap();
    for id in 1..=300i64 {
        conn.execute(
            "INSERT INTO memories(
                id, key, rev, kind, scope, session, body, tags, refs, content_hash,
                source, created_at, anchored_at, expires_at, superseded_by, superseded_at, fts_version
             ) VALUES (?1, ?2, 1, 'fact', 'durable', '', 'target stale', '', '', 0,
                       'cli', 100, 0, 0, ?1, 100, 2)",
            rusqlite::params![id, format!("stale-{id}")],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories_fts_v2(rowid, text) VALUES (?1, 'target stale')",
            rusqlite::params![id],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO memories(
            id, key, rev, kind, scope, session, body, tags, refs, content_hash,
            source, created_at, anchored_at, expires_at, superseded_by, superseded_at, fts_version
         ) VALUES (1000, 'live', 1, 'fact', 'durable', '', 'target current', '', '', 0,
                   'cli', 100, 0, 0, NULL, 0, 2)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memories_fts_v2(rowid, text) VALUES (1000, 'target current')",
        [],
    )
    .unwrap();
    let hits =
        crate::db::memory_fts_candidates(&conn, "target", &TimeWindow::default(), 1_000, true, 256)
            .unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].0, 1000);
    drop(conn);
    remove_tree(&root);
}

#[test]
fn migration_adds_fts_version_and_pending_index() {
    let root = temp_root("mem-v2-migrate");
    let conn = open_memory_store(&root).unwrap();
    conn.execute_batch(
        "DROP INDEX IF EXISTS memories_pending;
         ALTER TABLE memories DROP COLUMN fts_version;
         DROP INDEX IF EXISTS memories_created;
         PRAGMA user_version = 1;",
    )
    .unwrap();
    drop(conn);
    let conn = open_memory_store(&root).unwrap();
    let has: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name = 'fts_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(has, 1);
    for index in ["memories_created", "memories_pending"] {
        let found: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                rusqlite::params![index],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(found, 1, "{index}");
    }
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, i64::from(MEMORY_SCHEMA_VERSION));
    drop(conn);
    remove_tree(&root);
}

#[test]
fn fresh_and_migrated_schema_match() {
    fn columns(conn: &Connection) -> Vec<(String, String)> {
        let mut stmt = conn.prepare("PRAGMA table_info(memories)").unwrap();
        let mut rows: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        rows.sort();
        rows
    }
    fn indexes(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'memories' ORDER BY name")
            .unwrap();
        let mut rows: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        rows.sort();
        rows
    }

    let fresh_root = temp_root("mem-v2-schema-fresh");
    let fresh = open_memory_store(&fresh_root).unwrap();
    let migrated_root = temp_root("mem-v2-schema-migrated");
    let conn = open_memory_store(&migrated_root).unwrap();
    conn.execute_batch(
        "DROP INDEX IF EXISTS memories_pending;
         ALTER TABLE memories DROP COLUMN fts_version;
         DROP INDEX IF EXISTS memories_created;
         PRAGMA user_version = 1;",
    )
    .unwrap();
    drop(conn);
    let migrated = open_memory_store(&migrated_root).unwrap();

    assert_eq!(columns(&fresh), columns(&migrated));
    assert_eq!(indexes(&fresh), indexes(&migrated));
    let v2_sql: String = fresh
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'memories_fts_v2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(v2_sql.contains("remove_diacritics 2"));
    assert_eq!(meta(&fresh, "fts_epoch").as_deref(), Some("2"));
    assert_eq!(meta(&fresh, "fts_v2_complete").as_deref(), Some("1"));
    drop(fresh);
    drop(migrated);
    remove_tree(&fresh_root);
    remove_tree(&migrated_root);
}

#[test]
fn candidate_sql_prepares_for_both_tables() {
    let root = temp_root("mem-v2-candidates");
    let mut conn = open_memory_store(&root).unwrap();
    crate::db::memory_write(
        &mut conn,
        &super::memory_tests::note("k", "resolver tiers", "fact"),
        unix_now(),
    )
    .unwrap();
    let window = TimeWindow::default();
    let now = unix_now();
    for v2 in [false, true] {
        let hits = crate::db::memory_fts_candidates(
            &conn,
            &crate::memory::mem_query_terms("resolver"),
            &window,
            now,
            v2,
            10,
        )
        .unwrap();
        assert_eq!(hits.len(), 1, "v2={v2}");
    }
    drop(conn);
    remove_tree(&root);
}
