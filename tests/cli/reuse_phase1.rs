//! Phase 1 scan-truthfulness tests: path status, limit range, and
//! min-confidence validation.

use super::common::*;

#[test]
fn reuse_path_typo_exits_one() {
    let repo = temp_repo("reuse-path-typo");
    write(&repo, "a.py", "def unique_one():\n    return 1\n");
    build(&repo);
    let out = eg(&repo).args(["reuse", "missing.py"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("path not found"), "{stderr}");
}

#[test]
fn reuse_absolute_path_exits_two() {
    let repo = temp_repo("reuse-absolute-path");
    write(&repo, "a.py", "def unique_one():\n    return 1\n");
    build(&repo);
    let out = eg(&repo).args(["reuse", "/etc/passwd"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("repo-relative"), "{stderr}");
}

#[test]
fn reuse_limit_zero_rejected_before_index_open() {
    let repo = temp_repo("reuse-limit-zero-unbuilt");
    let out = eg(&repo).args(["reuse", "--limit", "0"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("limit must be 1..=100"), "{stderr}");
    assert!(!stderr.contains("no index"), "{stderr}");
}

#[test]
fn reuse_limit_above_100_exits_two() {
    let repo = temp_repo("reuse-limit-101");
    write(&repo, "a.py", "def unique_one():\n    return 1\n");
    build(&repo);
    let out = eg(&repo)
        .args(["reuse", "--limit", "101"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("limit"), "{stderr}");
}

#[test]
fn reuse_subset_without_shared_callee_is_rejected() {
    let repo = temp_repo("reuse-subset-d0");
    write(&repo, "a.py", "def parse_token(value):\n    return value\n");
    write(
        &repo,
        "b.py",
        "def parse_token_value(value):\n    return value\n",
    );
    build(&repo);
    let out = eg(&repo).args(["reuse", "b.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("parse_token_value' shares"),
        "subset with d=0 must be rejected: {stdout}"
    );
}

#[test]
fn reuse_composition_guard_rejects_callee_pair() {
    let repo = temp_repo("reuse-composition");
    write(
        &repo,
        "a.py",
        "def parse_token(value: str):\n    return value\n\ndef parse_token_wrapper(value: str):\n    return parse_token(value)\n",
    );
    build(&repo);
    let out = eg(&repo).args(["reuse"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("parse_token_wrapper' shares"),
        "callee must not be reported as a duplicate: {stdout}"
    );
}

#[test]
fn reuse_min_confidence_rejects_medium() {
    let repo = temp_repo("reuse-min-confidence");
    write(&repo, "a.py", "def unique_one():\n    return 1\n");
    build(&repo);
    let out = eg(&repo)
        .args(["reuse", "--min-confidence", "medium"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn reuse_stale_header_after_file_change() {
    let repo = temp_repo("reuse-stale");
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
    write(
        &repo,
        "b.py",
        "def parse_token(token: str):\n    return token.strip() + 'x'\n",
    );
    let out = eg(&repo).args(["reuse"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("result files changed since build"),
        "expected stale header: {stdout}"
    );
}

#[test]
fn reuse_file_line_selects_enclosing_symbol() {
    let repo = temp_repo("reuse-file-line");
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
    let out = eg(&repo).args(["reuse", "b.py:1"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("path_matches=1"), "{stdout}");
    assert!(stdout.contains("parse_token' shares"), "{stdout}");
}

#[test]
fn reuse_file_line_without_symbol_exits_one() {
    let repo = temp_repo("reuse-file-line-empty");
    write(
        &repo,
        "a.py",
        "def parse_token(token: str):\n    return token\n",
    );
    build(&repo);
    let out = eg(&repo).args(["reuse", "a.py:99"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no symbol at a.py:99"), "{stderr}");
}

#[test]
fn reuse_file_scope_does_not_prefix_match_sibling_extension() {
    let repo = temp_repo("reuse-file-prefix");
    write(
        &repo,
        "a.ts",
        "export function parseToken(v: string) { return v; }\n",
    );
    write(
        &repo,
        "a.tsx",
        "export function parseToken(v: string) { return v; }\n",
    );
    write(
        &repo,
        "b.ts",
        "export function parseToken(v: string) { return v; }\n",
    );
    build(&repo);
    let out = eg(&repo).args(["reuse", "a.ts"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("a.tsx:"), "{stdout}");
}

#[test]
fn reuse_ambiguous_bare_name_lists_candidates() {
    let repo = temp_repo("reuse-ambiguous");
    write(&repo, "a.py", "def parse(v):\n    return v\n");
    write(&repo, "b.py", "def parse(v):\n    return v\n");
    build(&repo);
    let out = eg(&repo).args(["reuse", "parse"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("a.py:") && stdout.contains("b.py:"),
        "{stdout}"
    );
    assert!(stderr.contains("ambiguous name 'parse'"), "{stderr}");
}

#[test]
fn reuse_explain_appends_detector_and_reasons() {
    let repo = temp_repo("reuse-explain");
    write(
        &repo,
        "a.py",
        "def normalize(value):\n    return value\n\ndef parse_token(value):\n    return normalize(value)\n",
    );
    write(
        &repo,
        "b.py",
        "from a import normalize\n\ndef parse_token_value(value):\n    return normalize(value)\n",
    );
    build(&repo);
    let out = eg(&repo)
        .args(["reuse", "b.py", "--explain"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("detector=D2"), "{stdout}");
    assert!(
        stdout.contains("reasons=[same-name-subset]"),
        "expected subset reason: {stdout}"
    );
}

#[test]
fn reuse_path_not_indexed_exits_one() {
    let repo = temp_repo("reuse-unindexed-path");
    write(&repo, "a.py", "def unique_one():\n    return 1\n");
    build(&repo);
    write(&repo, "new.py", "def unique_two():\n    return 2\n");
    let out = eg(&repo).args(["reuse", "new.py"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("path is not indexed"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn reuse_symlink_path_exits_two() {
    let repo = temp_repo("reuse-symlink-path");
    write(&repo, "a.py", "def unique_one():\n    return 1\n");
    std::os::unix::fs::symlink(repo.join("a.py"), repo.join("link.py")).unwrap();
    build(&repo);
    let out = eg(&repo).args(["reuse", "link.py"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("repo-relative and contained"), "{stderr}");
}

#[test]
fn reuse_bare_name_zero_match_exits_one() {
    let repo = temp_repo("reuse-zero-match");
    write(&repo, "a.py", "def unique_one():\n    return 1\n");
    build(&repo);
    let out = eg(&repo)
        .args(["reuse", "missing_symbol"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("symbol not in index"), "{stderr}");
}

#[test]
fn reuse_multiple_paths_abort_on_invalid() {
    let repo = temp_repo("reuse-multi-path");
    write(&repo, "a.py", "def unique_one():\n    return 1\n");
    build(&repo);
    let out = eg(&repo)
        .args(["reuse", "a.py", "missing.py"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("path not found"), "{stderr}");
}

#[test]
fn reuse_truncation_reports_showing_and_omitted() {
    let repo = temp_repo("reuse-truncation");
    let mut a = String::new();
    let mut b = String::new();
    for i in 0..20 {
        a.push_str(&format!("def parse_item_{i}(value):\n    return value\n"));
        b.push_str(&format!("def parse_item_{i}(value):\n    return value\n"));
    }
    write(&repo, "a.py", &a);
    write(&repo, "b.py", &b);
    build(&repo);
    let out = eg(&repo)
        .args(["reuse", "b.py", "--limit", "10"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("10 omitted"), "{stdout}");
    assert!(stdout.contains("showing 10 of 20"), "{stdout}");
}

#[test]
fn reuse_deleted_target_file_is_stale() {
    let repo = temp_repo("reuse-deleted-file");
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
    std::fs::remove_file(repo.join("b.py")).unwrap();
    let out = eg(&repo).args(["reuse"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("result files changed since build"),
        "expected stale header after deletion: {stdout}"
    );
}

#[test]
fn scan_self_match_rejected() {
    let repo = temp_repo("reuse-self-match");
    write(
        &repo,
        "a.py",
        "def recurse(n):\n    if n:\n        return recurse(n - 1)\n    return 0\n",
    );
    build(&repo);
    let out = eg(&repo).args(["reuse", "a.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("0 duplication candidate(s) found"),
        "{stdout}"
    );
    assert!(!stdout.contains("recurse' shares"), "{stdout}");
}

#[test]
fn scan_sanitizes_echoed_input() {
    let repo = temp_repo("reuse-sanitize");
    write(&repo, "a.py", "def unique_one():\n    return 1\n");
    build(&repo);
    let out = eg(&repo).args(["reuse", "missing\n.py"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains(r"missing\n.py"), "{stderr}");
    assert!(!stderr.contains("missing\n.py"), "{stderr}");
}

#[test]
fn reuse_empty_scope_warns_about_new_unindexed_file() {
    let repo = temp_repo("reuse-new-file");
    write(&repo, "a.py", "def unique_one():\n    return 1\n");
    build(&repo);
    write(&repo, "new_file.py", "def unique_two():\n    return 2\n");
    let out = eg(&repo).args(["reuse"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("freshness unknown (new-file)"),
        "expected new-file warning: {stdout}"
    );
}

#[test]
fn reuse_min_confidence_filters_low() {
    let repo = temp_repo("reuse-min-confidence-low");
    let mut source = String::new();
    for i in 0..7 {
        source.push_str(&format!("def h{i}():\n    return {i}\n"));
    }
    source.push_str("def big():\n    return h0() + h1() + h2() + h3() + h4() + h5() + h6()\n");
    write(&repo, "a.py", &source);
    write(
        &repo,
        "b.py",
        "from a import h0, h1, h2\ndef small():\n    return h0() + h1() + h2()\n",
    );
    build(&repo);
    let all = eg(&repo).args(["reuse"]).output().unwrap();
    let all_stdout = String::from_utf8_lossy(&all.stdout);
    assert!(all_stdout.contains("confidence=low"), "{all_stdout}");

    let filtered = eg(&repo)
        .args(["reuse", "--min-confidence", "provisional"])
        .output()
        .unwrap();
    let filtered_stdout = String::from_utf8_lossy(&filtered.stdout);
    assert!(
        filtered_stdout.contains("below threshold"),
        "expected filtered summary: {filtered_stdout}"
    );
}
