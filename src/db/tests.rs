//! `db` unit tests (split by area; fs mutations go through `test_support`).

use super::test_support::{make_dir_all, remove_tree, rename_file, write_file};
use super::*;

#[test]
fn fresh_db_has_schema_v2() {
    let dir = temp_root("db-test");
    let conn = create_fresh(&dir).unwrap();
    let v: i32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, SCHEMA_VERSION);
    drop(conn);
    remove_tree(&dir);
}

#[test]
fn missing_index_is_exit1_not_panic() {
    let dir = temp_root("db-missing");
    let err = open_for_query(&dir).unwrap_err();
    assert_eq!(err.exit_code(), 1);
    remove_tree(&dir);
}

#[test]
fn corrupted_index_is_exit3() {
    let dir = temp_root("db-corrupt");
    make_dir_all(dir.join(".aw")).unwrap();
    write_file(db_path(&dir), b"this is not sqlite at all").unwrap();
    let err = open_for_query(&dir).unwrap_err();
    assert_eq!(err.exit_code(), 3);
    remove_tree(&dir);
}

#[test]
fn schema_skew_is_exit5() {
    let dir = temp_root("db-skew");
    {
        let conn = create_fresh(&dir).unwrap();
        conn.execute_batch("PRAGMA user_version = 99").unwrap();
    }
    // Build completes by renaming index.db.new onto index.db.
    rename_file(db_new_path(&dir), db_path(&dir)).unwrap();
    let err = open_for_query(&dir).unwrap_err();
    assert_eq!(err.exit_code(), 5);
    remove_tree(&dir);
}

#[test]
fn load_carried_chunks_large_path_sets() {
    let dir = temp_root("db-carried");
    let conn = create_fresh(&dir).unwrap();
    conn.execute(
        "INSERT INTO files(id, path, size, mtime, content_hash, parse_status, error_nodes)
             VALUES (1, 'a.py', 1, 0, 0, 'ok', 0)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO symbols(id, kind, name, qualname, file_id, line_start, line_end)
             VALUES (1, 'class', 'B', 'a.B', 1, 1, 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO edges(id, src_id, dst_id, dst_name, kind, confidence, site_line)
             VALUES (1, 1, NULL, 'Base', 'inherits', 'EXTRACTED', 1)",
        [],
    )
    .unwrap();

    // More paths than the default SQLite variable limit; only a.py exists.
    let mut rels: HashSet<String> = (0..33_000).map(|i| format!("f{i}.py")).collect();
    rels.insert("a.py".to_string());
    let index = load_carried(&conn, &rels).unwrap();
    assert!(index.symbols.iter().any(|s| s.qualname == "a.B"));
    assert_eq!(
        index.inherits,
        vec![("a.B".to_string(), "Base".to_string())]
    );
    drop(conn);
    remove_tree(&dir);
}

#[test]
fn gap_files_for_new_names_chunks_large_symbol_sets() {
    let dir = temp_root("db-gap-large");
    let conn = create_fresh(&dir).unwrap();
    conn.execute(
        "INSERT INTO files(id, path, size, mtime, content_hash, parse_status, error_nodes)
             VALUES (1, 'gap.py', 1, 0, 0, 'ok', 0)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO unresolved_calls(file_id, dst_name, site_line)
             VALUES (1, 'target_fn', 1)",
        [],
    )
    .unwrap();

    // 600 unique symbols (> 500 SQL_CHUNK) to verify SQLite expression tree depth stays <= 1000
    let mut new_names: HashSet<String> = (0..600).map(|i| format!("mod{i}.sym{i}")).collect();
    new_names.insert("pkg.target_fn".to_string());
    new_names.insert("other_pkg.target_fn".to_string());
    new_names.insert("another.target_fn".to_string());
    let gaps = gap_files_for_new_names(&conn, &new_names).unwrap();
    assert!(gaps.contains("gap.py"));
    drop(conn);
    remove_tree(&dir);
}
