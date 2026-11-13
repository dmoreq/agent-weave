//! Phase 4 opt-in coverage tests: class, unresolved, and test overlap.

use super::common::*;

#[test]
fn coverage_class_finds_duplicate_shapes() {
    let repo = temp_repo("reuse-coverage-class");
    write(
        &repo,
        "a.py",
        "class Parser:\n    def read(self):\n        return 1\n",
    );
    write(
        &repo,
        "b.py",
        "class Parser:\n    def read(self):\n        return 2\n",
    );
    build(&repo);
    let out = eg(&repo)
        .args(["reuse", "--coverage", "class", "--explain"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("class 'a.Parser'"), "{stdout}");
    assert!(stdout.contains("detector=D5"), "{stdout}");
}

#[test]
fn coverage_unresolved_finds_indexed_canonical() {
    let repo = temp_repo("reuse-coverage-unresolved");
    write(&repo, "a.py", "def rogue():\n    return normalize(1)\n");
    write(&repo, "b.py", "def normalize(value):\n    return value\n");
    build(&repo);
    let out = eg(&repo)
        .args(["reuse", "--coverage", "unresolved", "--explain"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("unresolved callee"), "{stdout}");
    assert!(stdout.contains("detector=D1L"), "{stdout}");
}

#[test]
fn coverage_tests_reports_test_production_overlap() {
    let repo = temp_repo("reuse-coverage-tests");
    write(
        &repo,
        "parse.py",
        "def parse_token(value):\n    return value\n",
    );
    write(
        &repo,
        "test_parse.py",
        "def test_parse_token(value):\n    return value\n",
    );
    build(&repo);
    let out = eg(&repo)
        .args(["reuse", "--coverage", "tests", "--explain"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("test helper"), "{stdout}");
    assert!(stdout.contains("detector=DT"), "{stdout}");
}

#[test]
fn coverage_rejects_unknown_kind() {
    let repo = temp_repo("reuse-coverage-invalid");
    write(&repo, "a.py", "def one():\n    return 1\n");
    build(&repo);
    let out = eg(&repo)
        .args(["reuse", "--coverage", "bogus"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}
