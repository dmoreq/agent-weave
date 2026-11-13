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

#[test]
fn worktree_entries_fold_into_their_main_repository() {
    let main = crate::db::test_support::temp_root("summary-wt-main");
    crate::db::test_support::make_dir_all(main.join(".git/worktrees/wt")).unwrap();
    let wt = crate::db::test_support::temp_root("summary-wt-link");
    let pointer = format!("gitdir: {}/.git/worktrees/wt\n", main.display());
    crate::db::test_support::write_file(wt.join(".git"), pointer.as_bytes()).unwrap();
    // The fold keys on the canonical main root: temp roots spell /var/folders
    // while resolved answers spell /private/var/folders on macOS.
    let main_canon = std::fs::canonicalize(&main).unwrap_or_else(|_| main.clone());
    let main_str = main_canon.to_string_lossy().into_owned();
    let wt_str = wt.to_string_lossy().into_owned();
    let main_name = main.file_name().unwrap().to_string_lossy().into_owned();

    let entries = vec![
        json!({ "tool": "build", "exit_code": 0, "repo": main_str, "repo_name": "main-repo" }),
        json!({ "tool": "callers", "exit_code": 0, "repo": wt_str, "repo_name": "wt", "worktree": true }),
        json!({ "tool": "callers", "exit_code": 3, "repo": wt_str, "repo_name": "wt", "worktree": true }),
    ];
    let summary = compute_summary(&entries);

    // One row for the main repository; the worktree never gets its own row.
    let aggs = summary["repo_aggs"].as_object().unwrap();
    assert_eq!(aggs.len(), 1, "{summary}");
    assert_eq!(summary["repo_aggs"][&main_str]["display"], "main-repo");
    assert_eq!(summary["repo_aggs"][&main_str]["calls"], 3);
    assert_eq!(summary["repo_aggs"][&main_str]["errors"], 1);
    assert_eq!(
        summary["repo_aggs"][&main_str]["worktrees"]["wt"]["calls"],
        2
    );
    assert_eq!(
        summary["repo_aggs"][&main_str]["worktrees"]["wt"]["errors"],
        1
    );

    // Feed rows keep the worktree name plus the main-repository stamp.
    let rows = summary["graph_recent_queries"].as_array().unwrap();
    let stamped = rows
        .iter()
        .find(|r| r["repo_name"] == "wt")
        .expect("worktree row in the graph window");
    assert_eq!(stamped["repo_main_name"], main_name, "{stamped}");

    crate::db::test_support::remove_tree(&main);
    crate::db::test_support::remove_tree(&wt);
}

#[test]
fn unresolvable_worktree_entries_keep_their_own_row() {
    let wt = crate::db::test_support::temp_root("summary-wt-orphan");
    // A pointer file with no worktree admin directory behind it does not fold.
    crate::db::test_support::write_file(
        wt.join(".git"),
        b"gitdir: /nonexistent-eg-wt/.git/worktrees/wt\n",
    )
    .unwrap();
    let wt_str = wt.to_string_lossy().into_owned();
    let entries = vec![
        json!({ "tool": "callers", "exit_code": 0, "repo": wt_str, "repo_name": "orphan", "worktree": true }),
    ];
    let summary = compute_summary(&entries);
    let aggs = summary["repo_aggs"].as_object().unwrap();
    assert_eq!(aggs.len(), 1, "{summary}");
    assert_eq!(summary["repo_aggs"][&wt_str]["display"], "orphan");
    assert_eq!(summary["repo_aggs"][&wt_str]["worktree"], true);
    crate::db::test_support::remove_tree(&wt);
}
