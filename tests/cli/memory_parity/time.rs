//! Time-window acceptance for `mem query` and `mem export`.

use std::path::Path;
use std::process::Command;

use super::super::common::{eg, mem_out, mem_write, temp_repo};

fn sql(repo: &Path, statement: &str) {
    let out = Command::new("sqlite3")
        .arg(repo.join(".engram/memory.db"))
        .arg(statement)
        .output()
        .expect("sqlite3");
    assert!(
        out.status.success(),
        "sqlite3 failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn backdate(repo: &Path, key: &str, at: i64) {
    sql(
        repo,
        &format!("UPDATE memories SET created_at = {at} WHERE key = '{key}'"),
    );
}

fn seed_three(repo: &Path) {
    mem_write(repo, "alpha", "alpha body", "fact");
    mem_write(repo, "beta", "beta body", "fact");
    mem_write(repo, "gamma", "gamma body", "fact");
    backdate(repo, "alpha", 1_000);
    backdate(repo, "beta", 2_000);
    backdate(repo, "gamma", 3_000);
}

#[test]
fn time_window_is_half_open_and_applies_to_all() {
    let repo = temp_repo("mem-time-window");
    seed_three(&repo);

    let (code, out, err) = mem_out(&repo, &["query", "--all", "--since", "@1500"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("beta r"), "{out}");
    assert!(out.contains("gamma r"), "{out}");
    assert!(!out.contains("alpha r"), "{out}");

    let (code, out, _) = mem_out(&repo, &["query", "--all", "--until", "@2500"]);
    assert_eq!(code, 0);
    assert!(out.contains("alpha r"));
    assert!(out.contains("beta r"));
    assert!(!out.contains("gamma r"));

    let (code, out, _) = mem_out(
        &repo,
        &["query", "--all", "--since", "@2000", "--until", "@3000"],
    );
    assert_eq!(code, 0);
    assert!(out.contains("beta r"));
    assert!(!out.contains("alpha r"));
    assert!(!out.contains("gamma r"));
}

#[test]
fn time_window_filters_export() {
    let repo = temp_repo("mem-time-export");
    seed_three(&repo);
    let (code, out, err) = mem_out(&repo, &["export", "--since", "@2000"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(!out.contains("\"alpha\""), "{out}");
    assert!(out.contains("\"beta\""), "{out}");
    assert!(out.contains("\"gamma\""), "{out}");
}

#[test]
fn invalid_bounds_are_usage_errors() {
    let repo = temp_repo("mem-time-invalid");
    mem_write(&repo, "alpha", "alpha body", "fact");

    for args in [
        vec!["query", "--all", "--since", "20261111"],
        vec!["query", "--all", "--since", "2026-02-30"],
        vec!["query", "--all", "--since", "@2", "--until", "@1"],
        vec!["export", "--until", "2026-11-12T00:00:00+07:00"],
    ] {
        let (code, _, err) = mem_out(&repo, &args);
        assert_eq!(code, 2, "{args:?} stderr: {err}");
        assert!(
            err.contains("expects") || err.contains("must not be after"),
            "{err}"
        );
    }
}

#[test]
fn boundary_row_at_until_is_excluded() {
    let repo = temp_repo("mem-time-boundary");
    mem_write(&repo, "edge", "edge body", "fact");
    backdate(&repo, "edge", 2_000);
    let (code, _, _) = mem_out(&repo, &["query", "--all", "--until", "@2000"]);
    assert_eq!(code, 1, "a row exactly at until must be excluded");
}

#[test]
fn explain_requires_terms_and_all_rejects_it() {
    let repo = temp_repo("mem-explain-guard");
    mem_write(&repo, "alpha", "alpha body", "fact");
    let (code, _, err) = mem_out(&repo, &["query", "--all", "--explain"]);
    assert_eq!(code, 2, "stderr: {err}");
    assert!(err.contains("--explain requires terms"), "{err}");
    let _ = eg(&repo);
}

#[test]
fn read_only_unmigrated_store_uses_legacy_terms() {
    let repo = temp_repo("mem-unmigrated");
    mem_write(&repo, "viet", "Bộ nhớ dự án lưu quyết định", "fact");
    sql(
        &repo,
        "DROP TABLE memories_fts_v2; DELETE FROM memory_meta;",
    );

    // The legacy table keeps accented queries working, and the unaccented
    // query must not invent a match before a write-open repair.
    let (code, out, err) = mem_out(&repo, &["query", "bộ nhớ"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("viet r"), "{out}");
    let (code, _, _) = mem_out(&repo, &["query", "bo nho"]);
    assert_eq!(code, 1);
}

#[test]
fn window_applies_to_merged_shadow_probe() {
    let repo = temp_repo("mem-window-shadow");
    mem_write(&repo, "shared", "deploy local", "fact");
    let out = eg(&repo)
        .args([
            "mem",
            "write",
            "shared",
            "--body",
            "deploy global",
            "--kind",
            "fact",
            "--global",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    backdate(&repo, "shared", 1_000);
    let global_db = repo.join(".test_global/memory.db");
    let out = Command::new("sqlite3")
        .arg(&global_db)
        .arg("UPDATE memories SET created_at = 3000 WHERE key = 'shared'")
        .output()
        .unwrap();
    assert!(out.status.success());

    // Outside the window the local note must not shadow the in-window global.
    let (code, out, err) = mem_out(&repo, &["query", "deploy", "--since", "@2000"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("shared r"), "{out}");
    assert!(out.contains("[global]"), "{out}");
}

#[test]
fn journal_records_time_and_scope_filters() {
    let repo = temp_repo("mem-window-journal");
    mem_write(&repo, "note", "resolver body", "fact");
    let (code, _, err) = mem_out(
        &repo,
        &[
            "query",
            "resolver",
            "--since",
            "@0",
            "--until",
            "@9999999999",
            "--local",
        ],
    );
    assert_eq!(code, 0, "stderr: {err}");
    let log = std::fs::read_to_string(repo.join(".engram/usage.log")).unwrap_or_default();
    assert!(log.contains("\"since\":\"0\""), "{log}");
    assert!(log.contains("\"until\":\"9999999999\""), "{log}");
    assert!(log.contains("\"local\":\"true\""), "{log}");
}
