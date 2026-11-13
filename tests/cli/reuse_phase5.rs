//! Phase 5 judge tests: missing judge degrades without changing static output.

use super::common::*;

#[test]
fn judge_missing_binary_degrades_with_one_note() {
    let repo = temp_repo("reuse-judge-missing");
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
    let out = eg(&repo)
        .env("EG_JUDGE_BIN", "/nonexistent/eg-judge")
        .env("EG_JUDGE_MODEL", "/nonexistent/model")
        .args(["reuse", "--judge"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("duplication candidate(s) found"),
        "{stdout}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(stderr.matches("judge unavailable").count(), 1, "{stderr}");
}
