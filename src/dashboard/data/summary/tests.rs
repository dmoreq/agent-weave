//! Unit tests for the usage-summary fold. Kept out of `summary.rs` so the
//! module stays inside the 500-line cap.

use super::*;
use serde_json::json;

#[test]
fn untagged_sessions_are_counted_as_zero() {
    let entries = vec![
        json!({ "tool": "mem_query", "exit_code": 0 }),
        json!({ "tool": "mem_query", "exit_code": 0, "session": "" }),
    ];
    let summary = compute_summary(&entries);
    assert_eq!(summary["session_tagged_count"], 0);
    assert_eq!(summary["session_count"], 1);
}

#[test]
fn entries_without_repo_name_use_their_own_bucket() {
    let entries = vec![
        json!({ "tool": "symbols", "exit_code": 0 }),
        json!({ "tool": "symbols", "exit_code": 0, "repo_name": "workspace" }),
    ];
    let summary = compute_summary(&entries);
    assert_eq!(summary["repo_aggs"]["(unattributed)"]["calls"], 1);
    assert_eq!(summary["repo_aggs"]["workspace"]["calls"], 1);
}

#[test]
fn legacy_repo_spellings_share_the_unattributed_bucket() {
    let entries = vec![
        json!({ "tool": "build", "exit_code": 0, "repo": "." }),
        json!({ "tool": "build", "exit_code": 0, "repo": "/" }),
        json!({ "tool": "build", "exit_code": 0 }),
        json!({ "tool": "build", "exit_code": 0, "repo": "/tmp/real-repo", "repo_name": "real-repo" }),
    ];
    let summary = compute_summary(&entries);
    let aggs = summary["repo_aggs"].as_object().unwrap();
    assert_eq!(aggs.len(), 2, "{summary}");
    assert_eq!(summary["repo_aggs"]["(unattributed)"]["calls"], 3);
    assert_eq!(
        summary["repo_aggs"]["(unattributed)"]["display"],
        "(unattributed)"
    );
    assert_eq!(
        summary["repo_aggs"]["/tmp/real-repo"]["display"],
        "real-repo"
    );
}

#[test]
fn tagged_sessions_are_counted_per_entry() {
    let entries = vec![
        json!({ "tool": "mem_query", "exit_code": 0, "session": "r1-task-1" }),
        json!({ "tool": "mem_get", "exit_code": 0, "session": "r1-task-1" }),
        json!({ "tool": "mem_get", "exit_code": 0 }),
    ];
    let summary = compute_summary(&entries);
    assert_eq!(summary["session_tagged_count"], 2);
    assert_eq!(summary["session_count"], 2);
}
