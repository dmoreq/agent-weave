//! Integration tests: CUJs 1-8, G4 staleness, and the read-only tree
//! snapshot invariant, exercised against a throwaway fixture repo.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn temp_repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kg-it-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(repo: &Path, rel: &str, content: &str) {
    let path = repo.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn kg(repo: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_kg"));
    c.arg("--root").arg(repo);
    c
}

fn build(repo: &Path) -> (i32, String, String) {
    let out = kg(repo).args(["build"]).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const WIDGET: &str = "\
class Widget:\n\
    def render(self):\n\
        return normalize('x')\n\
\n\
    def render2(self):\n\
        return self.render()\n\
";
const UTILS: &str = "\
def normalize(text):\n\
    return text.strip()\n\
\n\
def caller():\n\
    return normalize('a')\n\
";
const APP: &str = "\
from utils import normalize\n\
from .widget import Widget as W\n\
\n\
def run():\n\
    w = W()\n\
    return normalize(w.render())\n\
";
const BROKEN: &str = "def broken(:\n    pass\n";

fn fixture(name: &str) -> PathBuf {
    let repo = temp_repo(name);
    write(&repo, "utils.py", UTILS);
    write(&repo, "widget.py", WIDGET);
    write(&repo, "app.py", APP);
    repo
}

/// Walk the tree collecting (path, len) for files outside .kg/ — the read-only
/// invariant probe.
fn snapshot(repo: &Path) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    fn rec(dir: &Path, acc: &mut Vec<(String, u64)>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.file_name().is_some_and(|n| n == ".kg") {
                continue;
            }
            if p.is_dir() {
                rec(&p, acc);
            } else {
                let len = e.metadata().map(|m| m.len()).unwrap_or(0);
                acc.push((p.to_string_lossy().into_owned(), len));
            }
        }
    }
    rec(repo, &mut out);
    out.sort();
    out
}

#[test]
fn cuj1_build_happy_path_and_readonly_tree() {
    let repo = fixture("cuj1");
    let before = snapshot(&repo);
    let (code, stdout, _stderr) = build(&repo);
    assert_eq!(code, 0);
    let line = stdout.lines().find(|l| l.starts_with("kg: built index")).unwrap();
    for key in ["files=3", "parse_errors=0", "wall="] {
        assert!(line.contains(key), "stats line missing {key}: {line}");
    }
    assert!(repo.join(".kg/index.db").exists());
    assert!(repo.join(".kg/.gitignore").exists());
    let after = snapshot(&repo);
    assert_eq!(before, after, "tree changed outside .kg/ (invariant 3)");
}

#[test]
fn cuj2_callers_with_cap_and_pointer_format() {
    let repo = fixture("cuj2");
    build(&repo);
    // >50 callers so the cap path fires: generate 60 caller functions.
    let mut big = String::from("from utils import normalize\n");
    for i in 0..58 {
        big.push_str(&format!("def c{i}():\n    return normalize('{i}')\n\n"));
    }
    write(&repo, "big.py", &big);
    build(&repo);

    let out = kg(&repo).args(["callers", "normalize"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    let pointers: Vec<&str> = stdout
        .lines()
        .filter(|l| l.contains(':') && !l.starts_with("kg:"))
        .collect();
    assert_eq!(pointers.len(), 50, "cap default 50");
    assert!(pointers.iter().all(|l| l.split(':').count() >= 2));
    assert!(stdout.contains("showing 50 of 60"), "stdout: {stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("[kg] capped at 50 results (60 total)"),
        "stderr: {stderr}"
    );
}

#[test]
fn cuj3_corrupt_index_exit3() {
    let repo = fixture("cuj3");
    build(&repo);
    fs::write(repo.join(".kg/index.db"), b"garbage").unwrap();
    let out = kg(&repo).args(["callers", "normalize"]).output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("kg: corrupt-index:"), "{stderr}");
    assert!(stdout_is_clean(&out.stdout));
}

fn stdout_is_clean(stdout: &[u8]) -> bool {
    // CUJ3: no partial output on stdout.
    stdout.is_empty()
}

#[test]
fn cuj4_ambiguous_name_exit2_with_candidates() {
    let repo = fixture("cuj4");
    write(&repo, "a.py", "class A:\n    def process(self):\n        return 1\n");
    write(&repo, "b.py", "class B:\n    def process(self):\n        return 2\n");
    build(&repo);
    let out = kg(&repo).args(["callers", "process"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stdout.contains("a.py:"), "{stdout}");
    assert!(stdout.contains("b.py:"), "{stdout}");
    assert!(stdout.contains("A.process"), "{stdout}");
    assert!(stdout.contains("B.process"), "{stdout}");
    assert!(stderr.contains("ambiguous name 'process'"), "{stderr}");
}

#[test]
fn cuj5_schema_skew_exit5() {
    let repo = fixture("cuj5");
    build(&repo);
    // Hand-bump user_version beyond the binary's schema (skew simulation).
    let out = Command::new("python3")
        .arg("-c")
        .arg(format!(
            "import sqlite3; c = sqlite3.connect('{}/.kg/index.db'); c.execute('PRAGMA user_version = 99'); c.commit()",
            repo.display()
        ))
        .output()
        .unwrap();
    assert!(out.status.success());
    let out = kg(&repo).args(["callers", "normalize"]).output().unwrap();
    assert_eq!(out.status.code(), Some(5));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("schema v99, binary expects v1 — run kg build"),
        "{stderr}"
    );
}

#[test]
fn cuj6_concurrent_build_lock_exit4() {
    let repo = fixture("cuj6");
    // Hold the lock from this process; the child build must exit 4.
    let lock_path = repo.join(".kg/build.lock");
    fs::create_dir_all(repo.join(".kg")).unwrap();
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .read(true)
        .open(&lock_path)
        .unwrap();
    file.try_lock().unwrap();
    let out = kg(&repo).args(["build"]).output().unwrap();
    assert_eq!(out.status.code(), Some(4));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("kg: locked:"), "{stderr}");
    drop(file);
}

#[test]
fn cuj7_query_without_index_exit1() {
    let repo = temp_repo("cuj7");
    write(&repo, "a.py", "def f():\n    pass\n");
    let out = kg(&repo).args(["callers", "f"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("kg: no-results: no .kg/index.db under") && stderr.contains("run kg build"),
        "{stderr}"
    );
}

#[test]
fn cuj8_parse_quality_abort_exit7() {
    let repo = fixture("cuj8");
    write(&repo, "broken.py", BROKEN);
    // 1/4 files broken = 25% > default 5%.
    let out = kg(&repo).args(["build"]).output().unwrap();
    assert_eq!(out.status.code(), Some(7));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("kg: parse-quality: 25% of 4 files"), "{stderr}");
    // --force proceeds.
    let out = kg(&repo).args(["build", "--force"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("parse_errors=1"), "{stdout}");
}

#[test]
fn g4_staleness_header_after_edit() {
    let repo = fixture("g4");
    build(&repo);
    // Edit a result file after build (mtime + size change).
    write(&repo, "app.py", &format!("{APP}\n# trailing comment\n"));
    let out = kg(&repo).args(["callers", "normalize"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("kg: 1 of "),
        "staleness header first line: {stdout}"
    );
    assert!(stdout.contains("result files changed since build — run kg build"));
    // FNV-1a re-hash path: same size, different content -> still stale.
    let original = fs::read_to_string(repo.join("utils.py")).unwrap();
    let swapped: String = original.chars().rev().collect();
    fs::write(repo.join("utils.py"), &swapped).unwrap();
    let out = kg(&repo).args(["callers", "normalize"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("kg: 1 of "), "re-hash staleness: {stdout}");
}

#[test]
fn response_contract_limit_clamps() {
    let repo = fixture("limit");
    build(&repo);
    let out = kg(&repo).args(["callers", "normalize", "--limit", "0"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = kg(&repo).args(["callers", "normalize", "--limit", "999999"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "clamped, not rejected");
}

#[test]
fn fast_path_second_build_carries_everything() {
    let repo = fixture("fastpath");
    let (_, out1, _) = build(&repo);
    let (_, out2, _) = build(&repo);
    assert!(out1.contains("carried=0"), "{out1}");
    assert!(out2.contains("carried=3"), "{out2}");
}
