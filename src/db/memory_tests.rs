//! `db` unit tests (split by area; fs mutations go through `test_support`).

use std::fs;

use super::test_support::remove_tree;
use super::*;

pub(super) fn note<'a>(key: &'a str, body: &'a str, kind: &'a str) -> MemNoteInput<'a> {
    MemNoteInput {
        key,
        kind,
        tags: "",
        refs: "",
        scope: "durable",
        session: "",
        body,
        source: "cli",
        expires_at: 0,
        supersedes: None,
        touch: false,
    }
}

#[test]
fn memories_fts_is_pure_function_of_memories() {
    let root = temp_root("mem-purity");
    let mut conn = open_memory_store(&root).unwrap();
    let now = unix_now();
    memory_write(&mut conn, &note("k1", "first body text", "fact"), now).unwrap();
    memory_write(&mut conn, &note("k1", "second body", "fact"), now).unwrap();
    memory_write(&mut conn, &note("k2", "other note", "gotcha"), now).unwrap();
    memory_write(&mut conn, &note("k2", "other note", "gotcha"), now).unwrap(); // duplicate no-op
    memory_delete(&mut conn, "k2").unwrap();

    // Rebuild the derived FTS from the authoritative table and compare.
    let current = memory_current_rows(
        &conn,
        &MemFilter {
            kind: None,
            scope: None,
            now,
            include_superseded: false,
            window: crate::memory::TimeWindow::default(),
        },
        None,
    )
    .unwrap();
    let mut legacy: Vec<(i64, String)> = {
        let mut stmt = conn
            .prepare("SELECT rowid, text FROM memories_fts ORDER BY rowid")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    };
    legacy.sort();
    let mut v2: Vec<(i64, String)> = {
        let mut stmt = conn
            .prepare("SELECT rowid, text FROM memories_fts_v2 ORDER BY rowid")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    };
    v2.sort();
    let mut expected_legacy: Vec<(i64, String)> = current
        .iter()
        .map(|row| {
            (
                row.id,
                crate::memory::mem_fts_text_legacy(&row.key, &row.tags, &row.body),
            )
        })
        .collect();
    expected_legacy.sort();
    let mut expected_v2: Vec<(i64, String)> = current
        .iter()
        .map(|row| {
            (
                row.id,
                crate::memory::mem_fts_text(&row.key, &row.tags, &row.body),
            )
        })
        .collect();
    expected_v2.sort();
    assert_eq!(
        legacy, expected_legacy,
        "memories_fts must be a pure function of memories"
    );
    assert_eq!(
        v2, expected_v2,
        "memories_fts_v2 must be a pure function of memories"
    );
    assert_eq!(
        legacy.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        v2.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        "both FTS tables mirror the same current rows"
    );
    drop(conn);
    remove_tree(&root);
}

/// The predicate-matrix fixture: one store holding a durable note, an
/// expired note, a future-expiry note, and a superseded revision pair.
fn current_rows_fixture() -> (PathBuf, Connection, i64, Vec<MemRow>) {
    let root = temp_root("mem-current-predicate");
    let mut conn = open_memory_store(&root).unwrap();
    let now = unix_now();
    memory_write(&mut conn, &note("d1", "durable body", "fact"), now).unwrap();
    let mut expired = note("d2", "stale body", "fact");
    expired.expires_at = now - 10;
    memory_write(&mut conn, &expired, now).unwrap();
    let mut expiring_soon = note("d3", "later body", "gotcha");
    expiring_soon.expires_at = now + 3600;
    memory_write(&mut conn, &expiring_soon, now).unwrap();
    memory_write(&mut conn, &note("d4", "old rev", "decision"), now).unwrap();
    memory_write(&mut conn, &note("d4", "new rev", "decision"), now).unwrap();

    let all_rows: Vec<MemRow> = {
        let mut stmt = conn
            .prepare(&format!("SELECT {MEM_ROW_SQL} FROM memories"))
            .unwrap();
        stmt.query_map([], mem_row)
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    };
    (root, conn, now, all_rows)
}

#[test]
fn memory_current_rows_matches_is_current_predicate() {
    // The `--all` SQL filter and the ranked resolver's Rust predicate must
    // stay row-by-row equal across every filter combination, or the two
    // query modes drift on expiry/supersession semantics.
    let (root, conn, now, all_rows) = current_rows_fixture();
    // The fixture must actually contain the superseded and expired rows
    // the filter combinations below exist to distinguish.
    assert!(all_rows.iter().any(|r| r.superseded_by.is_some()));
    assert!(all_rows
        .iter()
        .any(|r| r.expires_at != 0 && r.expires_at <= now));
    for include_superseded in [false, true] {
        for kind in [
            None,
            Some(crate::memory::MemKind::Fact),
            Some(crate::memory::MemKind::Decision),
        ] {
            // Session plays the old "project" role: a scope with no rows in
            // this fixture, so the all-empty column still gets exercised.
            for scope in [
                None,
                Some(crate::memory::MemScope::Durable),
                Some(crate::memory::MemScope::Session),
            ] {
                let filter = MemFilter {
                    kind,
                    scope,
                    now,
                    include_superseded,
                    window: crate::memory::TimeWindow::default(),
                };
                let mut expected: Vec<i64> = all_rows
                    .iter()
                    .filter(|row| memory_row_is_current(row, &filter))
                    .map(|row| row.id)
                    .collect();
                expected.sort();
                let mut got: Vec<i64> = memory_current_rows(&conn, &filter, None)
                    .unwrap()
                    .iter()
                    .map(|row| row.id)
                    .collect();
                got.sort();
                assert_eq!(
                    got, expected,
                    "kind={kind:?} scope={scope:?} include_superseded={include_superseded}"
                );
            }
        }
    }
    drop(conn);
    remove_tree(&root);
}

/// Column names of `CREATE TABLE memories(...)` in schema/memory.sql:
/// first token per line inside the parens, with trailing comments and
/// constraint heads stripped.
fn schema_memory_columns(sql: &str) -> Vec<String> {
    let body = sql
        .split("CREATE TABLE memories(")
        .nth(1)
        .expect("schema/memory.sql must declare memories");
    let body = body.split(");").next().unwrap_or(body);
    const CONSTRAINT_HEADS: [&str; 5] = ["primary", "unique", "check", "foreign", "constraint"];
    body.lines()
        .filter_map(|line| {
            let line = line.split("--").next().unwrap_or("").trim();
            let head = line.split_whitespace().next()?.trim_end_matches(',');
            (!head.is_empty() && !CONSTRAINT_HEADS.contains(&head.to_ascii_lowercase().as_str()))
                .then(|| head.to_string())
        })
        .collect()
}

#[test]
fn memory_required_manifest_matches_schema_file() {
    // MEMORY_REQUIRED is the MINIMUM compatible structure the open gates
    // check. The schema may be a superset: v2 added `anchored_at`, and a v1
    // store without it must still open (readers fall back to created_at).
    // The pin is one-directional: every required column must exist in the
    // schema file, so a manifest edit that outlives a column fails here.
    let sql = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("schema")
            .join("memory.sql"),
    )
    .unwrap();
    let schema_columns = schema_memory_columns(&sql);
    let required: std::collections::HashSet<&str> = MEMORY_REQUIRED
        .iter()
        .map(|(table, column)| {
            assert_eq!(*table, "memories", "unexpected manifest table: {table}");
            *column
        })
        .collect();
    for column in &required {
        assert!(
            schema_columns.iter().any(|c| c == column),
            "MEMORY_REQUIRED column '{column}' is missing from schema/memory.sql"
        );
    }
    // The additive v2 column must exist in the create schema.
    assert!(
        schema_columns.iter().any(|c| c == "anchored_at"),
        "schema/memory.sql must carry the v2 anchored_at column"
    );
}

#[test]
fn memory_write_dedup_supersedes_versions() {
    let root = temp_root("mem-lifecycle");
    let mut conn = open_memory_store(&root).unwrap();
    let now = unix_now();
    let first = memory_write(&mut conn, &note("k", "body one", "decision"), now).unwrap();
    assert!(matches!(
        first,
        MemWriteOutcome::Written {
            rev: 1,
            superseded_rev: None
        }
    ));
    // Idempotent replay: identical content is a no-op naming the rev.
    let dup = memory_write(&mut conn, &note("k", "body one", "decision"), now).unwrap();
    assert!(matches!(dup, MemWriteOutcome::Duplicate { rev: 1 }));
    // Different content: new rev, previous marked superseded.
    let second = memory_write(&mut conn, &note("k", "body two", "decision"), now).unwrap();
    assert!(matches!(
        second,
        MemWriteOutcome::Written {
            rev: 2,
            superseded_rev: Some(1)
        }
    ));
    let (row, revs) = memory_get(&conn, "k").unwrap().unwrap();
    assert_eq!((row.rev, row.body.as_str(), revs), (2, "body two", 2));
    drop(conn);
    remove_tree(&root);
}

#[test]
fn memory_missing_key_reads_and_deletes_none() {
    // Missing key reads as None; deleting an unknown key deletes nothing.
    let root = temp_root("mem-missing-key");
    let mut conn = open_memory_store(&root).unwrap();
    assert!(memory_get(&conn, "missing").unwrap().is_none());
    assert!(memory_delete(&mut conn, "missing").unwrap().is_none());
    drop(conn);
    remove_tree(&root);
}

#[test]
fn memory_open_gates_reject_version_and_structure_skew() {
    let root = temp_root("mem-gates");
    drop(open_memory_store(&root).unwrap());
    // Newer than the binary -> exit 5 with the export-backup message.
    {
        let conn = Connection::open(memory_db_path(&root)).unwrap();
        conn.execute_batch("PRAGMA user_version = 99").unwrap();
    }
    let err = open_memory_query(&root).unwrap_err();
    assert_eq!(err.exit_code(), 5);
    assert!(
        err.to_string()
            .contains("restore from an eg mem export backup"),
        "{err}"
    );
    // Older but missing required structure -> the other exit-5 message.
    {
        let conn = Connection::open(memory_db_path(&root)).unwrap();
        conn.execute_batch(
            "PRAGMA user_version = 0;
                 DROP TABLE memories_fts;",
        )
        .unwrap();
    }
    let err = open_memory_store(&root).unwrap_err();
    assert_eq!(err.exit_code(), 5);
    assert!(
        err.to_string().contains("missing required structure"),
        "{err}"
    );
    remove_tree(&root);
}

#[test]
fn cross_key_supersede_removes_target_fts() {
    let root = temp_root("mem-cross-supersede");
    let mut conn = open_memory_store(&root).unwrap();
    let now = unix_now();
    memory_write(&mut conn, &note("k1", "alpha payload", "fact"), now).unwrap();
    let mut target = note("k2", "beta payload", "fact");
    target.supersedes = Some("k1");
    memory_write(&mut conn, &target, now).unwrap();
    // k1 is superseded: `mem get` hides it and its FTS row left the index.
    assert!(memory_get(&conn, "k1").unwrap().is_none());
    let hits = memory_fts_candidates(
        &conn,
        &crate::memory::mem_query_terms("alpha"),
        &crate::memory::TimeWindow::default(),
        crate::db::unix_now(),
        false,
        crate::memory::CANDIDATE_CAP,
    )
    .unwrap();
    assert!(hits.is_empty(), "superseded note still matched: {hits:?}");
    let orphan: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories_fts WHERE rowid NOT IN (SELECT id FROM memories)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(orphan, 0);
    remove_tree(&root);
}

#[test]
fn memory_touch_migrates_v1_store_and_keeps_created_at() {
    // A v1 store predates anchored_at: the write path adds the column and the
    // next touch records the anchor without rewriting the creation date.
    let dir = crate::db::test_support::temp_root("mem-migrate-v2");
    let db = dir.join("memory.db");
    let mut conn = crate::db::open_memory_store(&dir).unwrap();
    crate::db::memory_write(
        &mut conn,
        &crate::db::MemNoteInput {
            key: "k",
            kind: "fact",
            scope: "durable",
            session: "",
            body: "b",
            tags: "",
            refs: "",
            source: "cli",
            expires_at: 0,
            supersedes: None,
            touch: false,
        },
        500,
    )
    .unwrap();
    // Simulate v1: drop the additive column and stamp the old version.
    conn.execute_batch("ALTER TABLE memories DROP COLUMN anchored_at; PRAGMA user_version = 1;")
        .unwrap();
    drop(conn);

    let mut conn = crate::db::open_memory_mutation(&dir).unwrap();
    let cols: Vec<String> = conn
        .prepare("PRAGMA table_info(memories)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .flatten()
        .collect();
    assert!(
        cols.iter().any(|c| c == "anchored_at"),
        "migration adds the column"
    );
    crate::db::memory_touch(&mut conn, "k", 900).unwrap();
    let (created, anchored): (i64, i64) = conn
        .query_row(
            "SELECT created_at, anchored_at FROM memories WHERE key = 'k' AND superseded_by IS NULL",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(created, 500, "creation date is preserved");
    assert_eq!(anchored, 900, "touch records the anchor");
    drop(conn);
    crate::db::test_support::remove_tree(&dir);
    let _ = db;
}
