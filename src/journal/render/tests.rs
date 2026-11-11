//! Unit tests for the global usage fold's live-repository rule.

use super::*;

#[test]
fn global_rows_require_a_live_repo_root() {
    let live_root = crate::db::test_support::temp_root("journal-global-live");
    let live = live_root.to_string_lossy().into_owned();
    let lines = [
        format!(r#"{{"tool":"callers","exit_code":0,"repo":"{live}","repo_name":"live-repo"}}"#),
        // Purge policy: a dead absolute root is dropped before any fold.
        r#"{"tool":"callers","exit_code":0,"repo":"/nonexistent-eg-global/deleted","repo_name":"agent-skills"}"#.to_string(),
        // Identityless legacy lines (relative path, missing repo) too.
        r#"{"tool":"callers","exit_code":0,"repo":".tmp/verify/repo1","repo_name":"repo1"}"#.to_string(),
        r#"{"tool":"callers","exit_code":0}"#.to_string(),
    ]
    .join("\n");

    let (aggs, repo_aggs, errors) = aggregate_global_usage(&lines);
    assert_eq!(
        aggs["callers"].calls, 1,
        "dead and identityless lines dropped"
    );
    assert_eq!(repo_aggs[&live].display, "live-repo");
    assert_eq!(repo_aggs.len(), 1);
    assert!(errors.is_empty());

    let out = render_global_summary(&aggs, &repo_aggs, &errors, false).join("\n");
    assert!(out.contains("across 1 repositories"), "{out}");
    assert!(!out.contains("unattributed"), "{out}");
    assert!(out.contains("live-repo"), "{out}");
    assert!(!out.contains("agent-skills"), "{out}");
    assert!(!out.contains("repo1"), "{out}");
    crate::db::test_support::remove_tree(&live_root);
}

#[test]
fn live_repo_errors_render_and_dead_ones_vanish() {
    let live_root = crate::db::test_support::temp_root("journal-global-err");
    let live = live_root.to_string_lossy().into_owned();
    let lines = [
        format!(r#"{{"tool":"callers","exit_code":3,"repo":"{live}","repo_name":"live-repo"}}"#),
        r#"{"tool":"callers","exit_code":3,"repo":"/nonexistent-eg-global/deleted","repo_name":"agent-skills"}"#.to_string(),
        r#"{"tool":"callers","exit_code":3,"repo":".tmp/verify/repo1","repo_name":"repo1"}"#.to_string(),
    ]
    .join("\n");

    let (aggs, repo_aggs, errors) = aggregate_global_usage(&lines);
    assert_eq!(errors.len(), 1, "only the live repo's error survives");
    assert_eq!(errors[0].repo_name.as_deref(), Some("live-repo"));
    assert_eq!(repo_aggs.len(), 1);

    let out = render_global_summary(&aggs, &repo_aggs, &errors, false).join("\n");
    assert!(out.contains("live-repo"), "{out}");
    assert!(!out.contains("agent-skills"), "{out}");
    assert!(!out.contains("repo1"), "{out}");
    crate::db::test_support::remove_tree(&live_root);
}
