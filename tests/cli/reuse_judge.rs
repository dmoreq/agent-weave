//! Regression guard for `eg reuse --judge`: a judge with no binary or checkpoint must raise
//! exactly one actionable stderr note and leave the static verdict byte-identical.

use super::common::*;

#[test]
fn judge_without_binary_reports_once_and_keeps_static_output() {
    let repo = temp_repo("reuse-judge-missing");
    write(
        &repo,
        "a.py",
        "def canonical_a():\n    return 1\n\ndef duplicate_a():\n    return 1\n",
    );
    build(&repo);

    let plain = eg(&repo).args(["reuse"]).output().unwrap();
    assert_eq!(plain.status.code(), Some(0));
    let plain_stdout = String::from_utf8_lossy(&plain.stdout).into_owned();
    assert!(
        plain_stdout.contains("candidate"),
        "the fixture must produce at least one static candidate: {plain_stdout}"
    );

    let judged = eg(&repo)
        .args(["reuse", "--judge"])
        .env("HOME", repo.join("no-home"))
        .env("PATH", "")
        .env_remove("EG_JUDGE_BIN")
        .env_remove("EG_JUDGE_MODEL")
        .output()
        .unwrap();
    assert_eq!(judged.status.code(), Some(0));
    let judged_stdout = String::from_utf8_lossy(&judged.stdout).into_owned();
    assert_eq!(
        plain_stdout, judged_stdout,
        "a missing judge must not change the static verdict"
    );

    let stderr = String::from_utf8_lossy(&judged.stderr);
    let notes = stderr.matches("eg: judge:").count();
    assert_eq!(notes, 1, "exactly one note is required: {stderr}");
}
