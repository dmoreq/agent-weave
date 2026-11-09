//! `db` unit tests (split by area; fs mutations go through `test_support`).

use super::memory_tests::note;
use super::test_support::{make_dir_all, remove_tree, write_file};
use super::*;

#[test]
fn memory_corrupt_file_is_exit3_and_missing_is_exit1() {
    let root = temp_root("mem-corrupt");
    make_dir_all(eg_dir(&root)).unwrap();
    write_file(memory_db_path(&root), b"definitely not a sqlite header").unwrap();
    let err = open_memory_query(&root).unwrap_err();
    assert_eq!(err.exit_code(), 3, "{err}");
    let empty = temp_root("mem-missing");
    let err = open_memory_query(&empty).unwrap_err();
    assert_eq!(err.exit_code(), 1, "{err}");
    remove_tree(&root);
    remove_tree(&empty);
}

#[test]
fn mem_commands_work_without_an_index() {
    // Memory is independent of index.db: no build, no index, full write+read.
    let root = temp_root("mem-no-index");
    let mut conn = open_memory_store(&root).unwrap();
    let now = unix_now();
    memory_write(&mut conn, &note("standalone", "works alone", "fact"), now).unwrap();
    let candidates = memory_fts_candidates(
        &conn,
        &crate::memory::mem_query_terms("works alone"),
        crate::memory::CANDIDATE_CAP,
    )
    .unwrap();
    assert_eq!(candidates.len(), 1);
    assert!(!db_path(&root).exists(), "no index.db was created");
    drop(conn);
    remove_tree(&root);
}

#[test]
fn gc_purges_expired_rows_and_their_fts_entries() {
    let root = temp_root("mem-gc-expiry");
    let mut conn = open_memory_store(&root).unwrap();
    let now = unix_now();
    // A session note already expired (injected clock) and a live one.
    let mut expired = note("expired-note", "transient scratch", "fact");
    expired.expires_at = now - 1;
    memory_write(&mut conn, &expired, now - 100).unwrap();
    memory_write(
        &mut conn,
        &note("live-note", "durable knowledge", "fact"),
        now,
    )
    .unwrap();
    // Boundary: a row whose deadline is exactly now is expired (<= now).
    let mut boundary = note("boundary-note", "edge case", "fact");
    boundary.expires_at = now;
    memory_write(&mut conn, &boundary, now).unwrap();

    let (removed_expired, removed_superseded, durable) = memory_gc(&mut conn, now).unwrap();
    assert_eq!((removed_expired, removed_superseded, durable), (2, 0, 1));
    // The expired rows and their FTS entries are gone.
    let fts_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories_fts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fts_count, 1, "only the live note keeps an FTS row");
    let current = memory_current_rows(
        &conn,
        &MemFilter {
            kind: None,
            scope: None,
            now: now + 10,
            include_superseded: false,
        },
        None,
    )
    .unwrap();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].key, "live-note");
    // A row one second inside the deadline survives gc.
    let mut alive = note("almost-expired", "still inside", "fact");
    alive.expires_at = now + 1;
    memory_write(&mut conn, &alive, now).unwrap();
    let (removed, _, _) = memory_gc(&mut conn, now).unwrap();
    assert_eq!(removed, 0, "expires_at > now is not expired");
    drop(conn);
    remove_tree(&root);
}

#[test]
fn gc_retention_clock_is_superseded_at_not_created_at() {
    let root = temp_root("mem-gc-retention");
    let mut conn = open_memory_store(&root).unwrap();
    let now = unix_now();
    let day: i64 = 86_400;
    // Superseded yesterday but CREATED 40 days ago: must survive (the
    // retention clock is superseded_at).
    memory_write(
        &mut conn,
        &note("old-key", "v1 body", "fact"),
        now - 40 * day,
    )
    .unwrap();
    memory_write(&mut conn, &note("old-key", "v2 body", "fact"), now - day).unwrap();
    // Superseded 31 days ago (created 32 days ago): must be purged.
    memory_write(
        &mut conn,
        &note("ancient-key", "v1 body", "fact"),
        now - 32 * day,
    )
    .unwrap();
    memory_write(
        &mut conn,
        &note("ancient-key", "v2 body", "fact"),
        now - 31 * day,
    )
    .unwrap();

    let (_, removed_superseded, durable) = memory_gc(&mut conn, now).unwrap();
    assert_eq!(
        removed_superseded, 1,
        "only the 31-day-old supersession purges"
    );
    assert_eq!(durable, 2, "both current revs survive");
    let (row, revs) = memory_get(&conn, "old-key").unwrap().unwrap();
    assert_eq!(
        (row.rev, revs),
        (2, 2),
        "the young supersession survives audit"
    );
    drop(conn);
    remove_tree(&root);
}

#[test]
fn supersedes_marks_the_target_and_rolls_back_on_unknown() {
    let root = temp_root("mem-gc-supersedes");
    let mut conn = open_memory_store(&root).unwrap();
    let now = unix_now();
    memory_write(
        &mut conn,
        &note("target", "old conclusion", "decision"),
        now,
    )
    .unwrap();
    let mut replacement = note("replacement", "new conclusion", "decision");
    replacement.supersedes = Some("target");
    let outcome = memory_write(&mut conn, &replacement, now).unwrap();
    assert!(matches!(
        outcome,
        MemWriteOutcome::Written {
            rev: 1,
            superseded_rev: None
        }
    ));
    // The target's current rev is superseded by the new note.
    let target_rev: Option<i64> = conn
        .query_row(
            "SELECT superseded_by FROM memories WHERE key = 'target' AND rev = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let new_id: i64 = conn
        .query_row(
            "SELECT id FROM memories WHERE key = 'replacement'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(target_rev, Some(new_id));
    // Unknown target: the write rolls back entirely.
    let mut doomed = note("doomed", "never lands", "fact");
    doomed.supersedes = Some("missing-key");
    let err = memory_write(&mut conn, &doomed, now).unwrap_err();
    assert_eq!(err.exit_code(), 1, "{err}");
    assert!(
        memory_get(&conn, "doomed").unwrap().is_none(),
        "the failed supersedes write must leave no row"
    );
    drop(conn);
    remove_tree(&root);
}

static GLOBAL_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn global_memory_creates_dir_and_schema_safely() {
    let _guard = GLOBAL_ENV_LOCK.lock().unwrap();
    let base = temp_root("global-mem-schema");
    let tmp = base.join("sub-global-eg");
    let prev = std::env::var("ENGRAM_GLOBAL_DIR").ok();
    std::env::set_var("ENGRAM_GLOBAL_DIR", &tmp);

    let path = global_memory_db_path().unwrap();
    assert_eq!(path, tmp.join("memory.db"));
    assert!(!tmp.is_dir(), "path resolution must not create directory");

    let conn = open_global_memory_store().unwrap();
    assert!(tmp.is_dir(), "store creation must create directory");
    let v: i32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, MEMORY_SCHEMA_VERSION);
    drop(conn);

    if let Some(val) = prev {
        std::env::set_var("ENGRAM_GLOBAL_DIR", val);
    } else {
        std::env::remove_var("ENGRAM_GLOBAL_DIR");
    }
    remove_tree(&base);
}

#[cfg(unix)]
#[test]
fn global_store_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let _guard = GLOBAL_ENV_LOCK.lock().unwrap();
    let tmp = temp_root("global-mem-modes");
    let prev = std::env::var("ENGRAM_GLOBAL_DIR").ok();
    std::env::set_var("ENGRAM_GLOBAL_DIR", &tmp);

    let conn = open_global_memory_store().unwrap();
    drop(conn);
    let dir_mode = std::fs::metadata(&tmp).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700, "global directory must be owner-only");
    let file_mode = std::fs::metadata(tmp.join("memory.db"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(file_mode, 0o600, "global store must be owner-only");

    if let Some(val) = prev {
        std::env::set_var("ENGRAM_GLOBAL_DIR", val);
    } else {
        std::env::remove_var("ENGRAM_GLOBAL_DIR");
    }
    remove_tree(&tmp);
}

#[test]
fn global_memory_write_and_query_roundtrip() {
    let _guard = GLOBAL_ENV_LOCK.lock().unwrap();
    let tmp = temp_root("global-mem-roundtrip");
    let prev = std::env::var("ENGRAM_GLOBAL_DIR").ok();
    std::env::set_var("ENGRAM_GLOBAL_DIR", &tmp);

    // Before creation, query returns NoMemory
    assert!(matches!(
        open_global_memory_query().unwrap_err(),
        EgError::NoMemory(_)
    ));

    let mut conn = open_global_memory_store().unwrap();
    let now = unix_now();
    let outcome = memory_write(
        &mut conn,
        &note("global-pref", "rust minimal", "decision"),
        now,
    )
    .unwrap();
    assert!(matches!(outcome, MemWriteOutcome::Written { rev: 1, .. }));
    drop(conn);

    // Read back with query
    let q_conn = open_global_memory_query().unwrap();
    let (row, revs) = memory_get(&q_conn, "global-pref").unwrap().unwrap();
    assert_eq!(row.key, "global-pref");
    assert_eq!(row.body, "rust minimal");
    assert_eq!(revs, 1);
    drop(q_conn);

    if let Some(val) = prev {
        std::env::set_var("ENGRAM_GLOBAL_DIR", val);
    } else {
        std::env::remove_var("ENGRAM_GLOBAL_DIR");
    }
    remove_tree(&tmp);
}
