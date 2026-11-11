use super::checks::{data_status, evaluate, overall_status, Status};
use super::collect::{collect, Mode};
use crate::db::test_support::{make_dir_all, remove_tree, temp_root, write_file};

fn seed(name: &str, lines: &[String]) -> std::path::PathBuf {
    let root = temp_root(name);
    make_dir_all(root.join(".engram")).unwrap();
    write_file(
        root.join(".engram/usage.log"),
        format!("{}\n", lines.join("\n")).as_bytes(),
    )
    .unwrap();
    root
}

fn line(root: &std::path::Path, now: i64, tool: &str, exit: i64, results: i64) -> String {
    serde_json::json!({
        "ts": now - 120,
        "tool": tool,
        "exit_code": exit,
        "result_count": results,
        "shown": results,
        "repo": root.to_string_lossy(),
    })
    .to_string()
}

fn check<'a>(checks: &'a [super::checks::CheckResult], id: &str) -> &'a super::checks::CheckResult {
    checks.iter().find(|c| c.id == id).unwrap()
}

#[test]
fn zero_empty_results_is_a_pass_not_insufficient() {
    let now = 1_800_000_000;
    let root = temp_root("audit-empty");
    let lines: Vec<String> = (0..12).map(|_| line(&root, now, "callers", 0, 3)).collect();
    let root = seed("audit-empty", &lines);
    let data = collect(&Mode::Local(root.clone()), now, 7, 7).unwrap();
    let checks = evaluate(&data);
    let c = check(&checks, "quality.empty_results");
    assert_eq!(c.status, Status::Pass, "{:?}", c.message);
    assert_eq!(c.value, Some(0.0));
    remove_tree(&root);
}

#[test]
fn navigation_share_counts_successes_only() {
    let now = 1_800_000_000;
    let root = temp_root("audit-nav");
    let mut lines: Vec<String> = (0..20).map(|_| line(&root, now, "callers", 0, 2)).collect();
    lines.extend((0..20).map(|_| line(&root, now, "callers", 1, 0)));
    let root = seed("audit-nav", &lines);
    let data = collect(&Mode::Local(root.clone()), now, 7, 7).unwrap();
    let checks = evaluate(&data);
    let c = check(&checks, "adoption.navigation");
    assert_eq!(c.value, Some(1.0));
    assert_eq!(c.status, Status::Pass);
    remove_tree(&root);
}

#[test]
fn malformed_and_invalid_lines_are_counted_separately() {
    let now = 1_800_000_000;
    let root = temp_root("audit-bad");
    let mut lines = vec![
        "not json".to_string(),
        serde_json::json!({"ts": now, "tool": "callers", "exit_code": 99,
            "repo": root.to_string_lossy()})
        .to_string(),
        line(&root, now, "callers", 0, 1),
    ];
    lines.push(line(&root, now, "callers", 0, 1));
    let root = seed("audit-bad", &lines);
    let data = collect(&Mode::Local(root.clone()), now, 7, 7).unwrap();
    assert_eq!(data.coverage.malformed_lines, 1);
    assert_eq!(data.coverage.invalid_lines, 1);
    assert_eq!(data.coverage.window_entries, 2);
    remove_tree(&root);
}

#[test]
fn critical_dominates_and_insufficient_never_becomes_pass() {
    let checks = vec![
        super::checks::CheckResult {
            id: "a",
            dimension: "x",
            status: Status::Insufficient,
            value: None,
            unit: "count",
            floor: None,
            baseline: None,
            threshold: super::checks::Threshold {
                warn: None,
                critical: None,
                direction: "lower_is_better",
                multiplier: None,
            },
            action: "none",
            repo_alias: None,
            message: String::new(),
        },
        super::checks::CheckResult {
            id: "b",
            dimension: "x",
            status: Status::Critical,
            value: None,
            unit: "count",
            floor: None,
            baseline: None,
            threshold: super::checks::Threshold {
                warn: None,
                critical: None,
                direction: "lower_is_better",
                multiplier: None,
            },
            action: "none",
            repo_alias: None,
            message: String::new(),
        },
    ];
    assert_eq!(overall_status(&checks), Status::Critical);
    let data = collect(&Mode::Local(temp_root("audit-status")), 1_800_000_000, 7, 7).unwrap();
    assert_eq!(data_status(&data, &checks), "insufficient");
}

#[test]
fn alias_is_stable_and_path_free() {
    let a = super::repo_alias(std::path::Path::new("/a/repo"));
    let b = super::repo_alias(std::path::Path::new("/a/repo"));
    assert_eq!(a, b);
    assert!(a.starts_with("repo-"));
    assert_eq!(a.len(), 21);
    assert!(!a.contains('/'));
}
