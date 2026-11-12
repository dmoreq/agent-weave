//! Advisory `mem export --scan` acceptance.

use std::path::Path;
use std::process::Command;

use super::super::common::{mem_out, mem_write, temp_repo};

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

#[test]
fn scan_reports_a_legacy_secret_without_changing_stdout() {
    let repo = temp_repo("mem-scan-body");
    mem_write(&repo, "legacy", "clean body", "fact");
    let secret = format!("sk-{}", "a".repeat(24));
    sql(
        &repo,
        &format!("UPDATE memories SET body = 'token {secret}' WHERE key = 'legacy'"),
    );

    let (code, plain, _) = mem_out(&repo, &["export"]);
    assert_eq!(code, 0);
    let (code, scanned, err) = mem_out(&repo, &["export", "--scan"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(plain, scanned, "scan must not change stdout");
    assert!(
        err.contains("eg: scan: 1 note contains suspected secrets"),
        "{err}"
    );
    assert!(err.contains("legacy: body"), "{err}");
    assert!(
        !err.contains(&secret),
        "scan must not echo the secret: {err}"
    );
}

#[test]
fn scan_redacts_a_key_match_and_lists_fields_in_order() {
    let repo = temp_repo("mem-scan-key");
    mem_write(&repo, "plain", "clean body", "fact");
    let secret = format!("sk-{}", "b".repeat(24));
    sql(
        &repo,
        &format!(
            "UPDATE memories SET key = '{secret}', tags = '{secret}', refs = '{secret}' WHERE key = 'plain'"
        ),
    );

    let (code, _, err) = mem_out(&repo, &["export", "--scan"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        err.contains("eg: scan: 1 note contains suspected secrets"),
        "{err}"
    );
    assert!(err.contains("row 1: key, tags, refs"), "{err}");
    assert!(!err.contains(&secret), "{err}");
}

#[test]
fn scan_is_silent_without_matches() {
    let repo = temp_repo("mem-scan-clean");
    mem_write(&repo, "clean", "ordinary words only", "fact");
    let (code, _, err) = mem_out(&repo, &["export", "--scan"]);
    assert_eq!(code, 0);
    assert!(!err.contains("eg: scan:"), "{err}");
}

#[test]
fn scan_plural_header_for_multiple_notes() {
    let repo = temp_repo("mem-scan-multi");
    mem_write(&repo, "one", "clean body one", "fact");
    mem_write(&repo, "two", "clean body two", "fact");
    let secret = format!("sk-{}", "c".repeat(24));
    for key in ["one", "two"] {
        sql(
            &repo,
            &format!("UPDATE memories SET body = 'token {secret}' WHERE key = '{key}'"),
        );
    }
    let (code, _, err) = mem_out(&repo, &["export", "--scan"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        err.contains("eg: scan: 2 notes contain suspected secrets"),
        "{err}"
    );
    assert!(err.contains("one: body"), "{err}");
    assert!(err.contains("two: body"), "{err}");
    assert!(!err.contains(&secret), "{err}");
}

#[test]
fn scan_respects_the_export_limit() {
    let repo = temp_repo("mem-scan-limit");
    mem_write(&repo, "old", "clean old body", "fact");
    mem_write(&repo, "new", "clean new body", "fact");
    let secret = format!("sk-{}", "d".repeat(24));
    sql(
        &repo,
        &format!("UPDATE memories SET body = 'token {secret}' WHERE key = 'old'"),
    );
    sql(
        &repo,
        "UPDATE memories SET created_at = 1000 WHERE key = 'old'",
    );
    sql(
        &repo,
        "UPDATE memories SET created_at = 3000 WHERE key = 'new'",
    );
    let (code, _, err) = mem_out(&repo, &["export", "--limit", "1", "--scan"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        !err.contains("eg: scan:"),
        "limit excludes the secret note: {err}"
    );
}
