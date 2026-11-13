//! Phase 3 allowlist tests: explicit suppression, visibility, and stale-ref
//! invalidation.

use super::common::*;

fn duplicate_repo(name: &str) -> TempRepo {
    let repo = temp_repo(name);
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
    repo
}

fn allow(repo: &TempRepo, without_refs: bool) {
    let mut args = vec![
        "mem",
        "write",
        "reuse-allow/b.parse_token->a.parse_token",
        "--kind",
        "decision",
        "--body",
        "intentional duplicate",
    ];
    if !without_refs {
        args.extend(["--refs", "b.py"]);
    }
    let out = eg(repo).args(args).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn allowlist_suppresses_and_show_exposes_count() {
    let repo = duplicate_repo("reuse-allowlist");
    allow(&repo, true);

    let hidden = eg(&repo).args(["reuse"]).output().unwrap();
    let hidden_stdout = String::from_utf8_lossy(&hidden.stdout);
    assert!(
        hidden_stdout.contains("0 duplication candidate(s) found"),
        "{hidden_stdout}"
    );
    assert!(hidden_stdout.contains("1 suppressed"), "{hidden_stdout}");
    let log = std::fs::read_to_string(repo.join(".engram/usage.log")).unwrap();
    let last: serde_json::Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
    assert_eq!(last["mode"], "scan");
    assert_eq!(last["reuse"]["suppressed"], 1);

    let shown = eg(&repo)
        .args(["reuse", "--show-suppressed"])
        .output()
        .unwrap();
    let shown_stdout = String::from_utf8_lossy(&shown.stdout);
    assert!(
        shown_stdout.contains("1 duplication candidate(s) found"),
        "{shown_stdout}"
    );
    assert!(shown_stdout.contains("1 suppressed"), "{shown_stdout}");
}

#[test]
fn allowlist_with_changed_ref_no_longer_suppresses() {
    let repo = duplicate_repo("reuse-allowlist-stale");
    allow(&repo, false);
    write(
        &repo,
        "b.py",
        "def parse_token(token: str):\n    return token.strip() + 'x'\n",
    );
    let out = eg(&repo).args(["reuse"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("symbol 'b.parse_token'"), "{stdout}");
    assert!(!stdout.contains("suppressed"), "{stdout}");
}
