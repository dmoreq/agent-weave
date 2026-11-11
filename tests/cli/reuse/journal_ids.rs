//! Journal identity of surfaced reuse candidates.

use super::super::common::*;

#[test]
fn reuse_journals_surfaced_candidate_ids() {
    let repo = temp_repo("reuse-journal-ids");
    write(
        &repo,
        "a.py",
        "def parse_token(token: str):\n    return token.strip()\n",
    );
    write(
        &repo,
        "b.py",
        "def parse_token(token: str):\n    return token.strip()\n",
    );
    build(&repo);
    let out = eg(&repo).args(["reuse"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));

    // The invocation line carries the surfaced identities so the dashboard can
    // report per-candidate usage.
    let log = fs::read_to_string(repo.join(".engram/usage.log")).unwrap();
    let last = log.lines().last().expect("journal line");
    let val: serde_json::Value = serde_json::from_str(last).unwrap();
    let candidates = val["args"]["candidates"].as_str().unwrap_or("");
    assert!(
        candidates.contains("parse_token->") || candidates.contains("->parse_token"),
        "candidate ids missing: {last}"
    );
    assert!(
        candidates.lines().count() >= 1,
        "at least one surfaced id: {last}"
    );
}
