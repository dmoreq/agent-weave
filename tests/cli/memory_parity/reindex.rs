//! `eg mem reindex` acceptance: report, journal, global scope, missing store.

use std::fs;

use super::super::common::{eg, mem_out, mem_write, temp_repo};

#[test]
fn reindex_reports_rows_and_journals() {
    let repo = temp_repo("mem-reindex");
    mem_write(&repo, "a", "alpha body", "fact");
    mem_write(&repo, "b", "beta body", "fact");

    let (code, out, err) = mem_out(&repo, &["reindex"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out.trim(), "reindexed 2 rows", "{out}");

    let log = fs::read_to_string(repo.join(".engram/usage.log")).unwrap_or_default();
    assert!(log.contains("\"tool\":\"mem_reindex\""), "{log}");
}

#[test]
fn reindex_global_store() {
    let repo = temp_repo("mem-reindex-global");
    let out = eg(&repo)
        .args([
            "mem",
            "write",
            "g",
            "--body",
            "global body",
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
    let (code, out, err) = mem_out(&repo, &["reindex", "--global"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out.trim(), "reindexed 1 rows", "{out}");
}

#[test]
fn reindex_missing_store_exits_one() {
    let repo = temp_repo("mem-reindex-missing");
    let (code, _, err) = mem_out(&repo, &["reindex"]);
    assert_eq!(code, 1, "stderr: {err}");
    assert!(err.contains("no .engram/memory.db"), "{err}");
}
