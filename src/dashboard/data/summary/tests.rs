//! Unit tests for the usage-summary fold. Kept out of `summary.rs` so the
//! module stays inside the 500-line cap.

use super::*;
use serde_json::json;

/// Session metrics only count calls against a live repository, so every
/// session fixture needs one. Distinct names keep parallel tests apart.
fn with_live_repo(name: &str) -> String {
    let root = crate::db::test_support::temp_root(name);
    root.to_string_lossy().into_owned()
}

#[test]
fn untagged_sessions_create_no_session() {
    // The recall rate measures memory use, not tagging discipline: untagged
    // calls must not fabricate a synthetic "default" session.
    let repo = with_live_repo("summary-sessions-untagged");
    let entries = vec![
        json!({ "tool": "mem_query", "exit_code": 0, "repo": repo }),
        json!({ "tool": "mem_query", "exit_code": 0, "repo": repo, "session": "" }),
    ];
    let summary = compute_summary(&entries);
    assert_eq!(summary["session_count"], 0);
    assert_eq!(summary["memory_session_count"], 0);
    assert_eq!(summary["session_recall_pct"], 0.0);
    assert_eq!(summary["total_calls"], 2, "calls still count");
    crate::db::test_support::remove_tree(std::path::Path::new(&repo));
}

#[test]
fn entries_without_a_live_repo_path_are_skipped() {
    let entries = vec![
        json!({ "tool": "symbols", "exit_code": 0 }),
        json!({ "tool": "symbols", "exit_code": 0, "repo_name": "workspace" }),
        json!({ "tool": "symbols", "exit_code": 0, "repo": ".tmp/verify/repo2", "repo_name": "repo2" }),
    ];
    let summary = compute_summary(&entries);
    let aggs = summary["repo_aggs"].as_object().unwrap();
    assert_eq!(aggs.len(), 0, "{summary}");
    assert_eq!(summary["total_calls"], 0);
}

#[test]
fn only_a_live_root_gets_a_repo_row() {
    let root = crate::db::test_support::temp_root("summary-live-repo");
    let root_str = root.to_string_lossy().into_owned();
    let entries = vec![
        json!({ "tool": "build", "exit_code": 0, "repo": "." }),
        json!({ "tool": "build", "exit_code": 0, "repo": "/" }),
        json!({ "tool": "build", "exit_code": 0 }),
        json!({ "tool": "build", "exit_code": 0, "repo": root_str, "repo_name": "real-repo" }),
    ];
    let summary = compute_summary(&entries);
    let aggs = summary["repo_aggs"].as_object().unwrap();
    assert_eq!(aggs.len(), 1, "{summary}");
    assert_eq!(summary["repo_aggs"][&root_str]["calls"], 1);
    assert_eq!(summary["repo_aggs"][&root_str]["display"], "real-repo");
    crate::db::test_support::remove_tree(&root);
}

#[test]
fn deleted_repository_is_skipped() {
    let entries = vec![
        json!({ "tool": "callers", "exit_code": 0, "repo": "/nonexistent-eg-summary/deleted", "repo_name": "agent-skills" }),
        json!({ "tool": "callers", "exit_code": 0, "repo": "/nonexistent-eg-summary/deleted", "repo_name": "agent-skills" }),
    ];
    let summary = compute_summary(&entries);
    let aggs = summary["repo_aggs"].as_object().unwrap();
    assert_eq!(aggs.len(), 0, "{summary}");
    assert_eq!(summary["total_calls"], 0);
}

#[test]
fn recall_is_tagged_only_and_distinct_per_session() {
    let repo = with_live_repo("summary-sessions-tagged");
    let entries = vec![
        json!({ "tool": "mem_query", "exit_code": 0, "repo": repo, "session": "s-1" }),
        json!({ "tool": "mem_get", "exit_code": 0, "repo": repo, "session": "s-1" }),
        json!({ "tool": "callers", "exit_code": 0, "repo": repo, "session": "s-2" }),
        json!({ "tool": "mem_get", "exit_code": 0, "repo": repo }),
    ];
    let summary = compute_summary(&entries);
    // Two distinct tagged sessions, one of which read memory -> 50%.
    assert_eq!(summary["session_count"], 2);
    assert_eq!(summary["memory_session_count"], 1);
    assert_eq!(summary["session_recall_pct"], 50.0);
    crate::db::test_support::remove_tree(std::path::Path::new(&repo));
}

#[test]
fn per_tool_savings_sum_to_the_global_estimate() {
    let repo = with_live_repo("summary-savings");
    let entries = vec![
        json!({ "tool": "callers", "exit_code": 0, "repo": repo, "shown": 4, "result_count": 4, "latency_us": 1000 }),
        json!({ "tool": "callers", "exit_code": 0, "repo": repo, "shown": 2, "result_count": 2, "latency_us": 2000 }),
        // Empty and error calls are priced at zero by the global fold.
        json!({ "tool": "callers", "exit_code": 1, "repo": repo, "shown": 0, "result_count": 0 }),
        json!({ "tool": "callers", "exit_code": 3, "repo": repo, "shown": 9, "result_count": 9 }),
    ];
    let summary = compute_summary(&entries);
    let tool_saved = summary["tool_aggs"]["callers"]["saved_tokens"]
        .as_u64()
        .unwrap();
    assert_eq!(tool_saved, summary["saved_tokens"].as_u64().unwrap());
    assert_eq!(summary["tool_aggs"]["callers"]["p95"], 2.0);
}

#[test]
fn payload_carries_provenance_and_no_dead_fields() {
    let summary = compute_summary(&[]);
    assert_eq!(summary["schema_version"], 2);
    assert_eq!(summary["window"], "retained_journal");
    assert!(summary["generated_at"].as_i64().is_some());
    assert_eq!(summary["saved_tokens_estimated"], true);
    assert_eq!(summary["graph_saved_tokens_estimated"], true);
    for dead in [
        "recent_queries",
        "recent_errors",
        "graph_recent_errors",
        "warning_count",
        "reuse_hits",
        "judged_calls",
        "session_tagged_count",
    ] {
        assert!(
            summary.get(dead).is_none(),
            "{dead} must be absent: {summary}"
        );
    }
}
