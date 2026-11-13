//! Phase 2 proposal tests: pre-finalize signatures, callee evidence, and
//! numeric confidence selection.

use super::common::*;

fn proposal_repo(name: &str) -> TempRepo {
    let repo = temp_repo(name);
    write(
        &repo,
        "a.py",
        "def normalize(value):\n    return value\n\ndef helper(value):\n    return value\n\ndef parse_token(value):\n    return normalize(value) + helper(value)\n",
    );
    build(&repo);
    repo
}

#[test]
fn proposal_finds_canonical_by_callee() {
    let repo = proposal_repo("reuse-proposal-callee");
    let out = eg(&repo)
        .args([
            "reuse",
            "--propose",
            "def parse_token(value)",
            "--callees",
            "normalize,helper",
            "--explain",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("existing 'a.parse_token'"), "{stdout}");
    assert!(stdout.contains("confidence=high"), "{stdout}");
    assert!(stdout.contains("detector=D1"), "{stdout}");
}

#[test]
fn proposal_respects_signature_language() {
    let repo = temp_repo("reuse-proposal-language");
    write(
        &repo,
        "a.py",
        "def normalize(value):\n    return value\n\ndef helper(value):\n    return value\n\ndef parse_token(value):\n    return normalize(value) + helper(value)\n",
    );
    write(
        &repo,
        "b.rs",
        "fn normalize(value: i32) -> i32 { value }\nfn helper(value: i32) -> i32 { value }\nfn parse_token(value: i32) -> i32 { normalize(value) + helper(value) }\n",
    );
    build(&repo);
    let out = eg(&repo)
        .args([
            "reuse",
            "--propose",
            "def parse_token(value)",
            "--callees",
            "normalize,helper",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("a.parse_token"), "{stdout}");
    assert!(!stdout.contains("b.parse_token"), "{stdout}");
}

#[test]
fn proposal_ignores_unresolved_only_callees() {
    let repo = temp_repo("reuse-proposal-unresolved");
    write(
        &repo,
        "a.py",
        "def parse_token(value):\n    return external.normalize(value) + external.helper(value)\n",
    );
    build(&repo);
    let out = eg(&repo)
        .args([
            "reuse",
            "--propose",
            "def parse_token(value)",
            "--callees",
            "normalize,helper",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("0 duplication candidate(s) found"),
        "{stdout}"
    );
}

#[test]
fn proposal_rejects_paths_and_orphan_callees() {
    let repo = proposal_repo("reuse-proposal-validation");
    let paths = eg(&repo)
        .args(["reuse", "a.py", "--propose", "def parse_token(value)"])
        .output()
        .unwrap();
    assert_eq!(paths.status.code(), Some(2));
    let orphan = eg(&repo)
        .args(["reuse", "--callees", "normalize"])
        .output()
        .unwrap();
    assert_eq!(orphan.status.code(), Some(2));
}

#[test]
fn proposal_journals_propose_mode_and_uptake_ids() {
    let repo = proposal_repo("reuse-proposal-journal");
    let out = eg(&repo)
        .args([
            "reuse",
            "--propose",
            "def parse_token(value)",
            "--callees",
            "normalize,helper",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let log = std::fs::read_to_string(repo.join(".engram/usage.log")).unwrap();
    let last = log.lines().last().unwrap();
    let value: serde_json::Value = serde_json::from_str(last).unwrap();
    assert_eq!(value["mode"], "propose");
    assert!(value["args"]["candidates"]
        .as_str()
        .is_some_and(|v| v.contains("parse_token")));
}

#[test]
fn proposal_high_threshold_filters_token_only_signal() {
    let repo = proposal_repo("reuse-proposal-high-filter");
    let out = eg(&repo)
        .args([
            "reuse",
            "--propose",
            "def parse_token(value)",
            "--min-confidence",
            "high",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("below threshold"), "{stdout}");
}

#[test]
fn proposal_min_confidence_filters_weaker_signal() {
    let repo = proposal_repo("reuse-proposal-threshold");
    let out = eg(&repo)
        .args([
            "reuse",
            "--propose",
            "def parse_token(value)",
            "--callees",
            "normalize,helper",
            "--min-confidence",
            "high",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("confidence=high"), "{stdout}");
}
