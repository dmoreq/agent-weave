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

// Raw strings: the previous `\`-continuation form stripped the leading
// indentation of every line, so class bodies collapsed to top-level defs and
// the fixtures never exercised method resolution at all.
const WIDGET: &str = r#"class Widget:
    def render(self):
        return normalize('x')

    def render2(self):
        return self.render()
"#;
const UTILS: &str = r#"def normalize(text):
    return text.strip()

def caller():
    return normalize('a')
"#;
const APP: &str = r#"from utils import normalize
from .widget import Widget as W

def run():
    w = W()
    return normalize(w.render())
"#;
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
fn snapshot(repo: &Path) -> Vec<(String, u64, u64)> {
    // (path, len, content hash) — hashes catch same-size edits (invariant 3).
    let mut out = Vec::new();
    fn rec(dir: &Path, acc: &mut Vec<(String, u64, u64)>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.file_name().is_some_and(|n| n == ".kg") {
                continue;
            }
            if p.is_dir() {
                rec(&p, acc);
            } else {
                let len = e.metadata().map(|m| m.len()).unwrap_or(0);
                let hash = fs::read(&p)
                    .map(|b| {
                        use std::hash::{Hash, Hasher};
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        b.hash(&mut h);
                        h.finish()
                    })
                    .unwrap_or(0);
                acc.push((p.to_string_lossy().into_owned(), len, hash));
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
        stderr.contains("schema v99, binary expects v2 — run kg build"),
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
    // Nanosecond mtimes make the mtime mismatch (and thus the re-hash)
    // deterministic; both result files now report changed.
    let original = fs::read_to_string(repo.join("utils.py")).unwrap();
    let swapped: String = original.chars().rev().collect();
    fs::write(repo.join("utils.py"), &swapped).unwrap();
    let out = kg(&repo).args(["callers", "normalize"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("kg: 2 of "), "re-hash staleness: {stdout}");
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

#[test]
fn fast_path_preserves_callers_of_changed_file() {
    // Dirty-closure regression: a carried file with edges into a changed file
    // must be re-extracted, or `callers`/`impact` silently lose results.
    let repo = fixture("closure");
    build(&repo);
    let out = kg(&repo).args(["callers", "normalize"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("app.py:6"), "baseline caller present: {stdout}");
    // Change utils.py (fresh); app.py/widget.py are carried.
    write(&repo, "utils.py", &format!("{UTILS}\n\ndef extra():\n    return 1\n"));
    build(&repo);
    let out = kg(&repo).args(["callers", "normalize"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "caller edges must survive: {stdout}");
    assert!(stdout.contains("app.py:6"), "app.py caller survived: {stdout}");
    assert!(stdout.contains("utils.py:5"), "utils.py caller survived: {stdout}");
    let out = kg(&repo).args(["impact", "utils.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "impact must survive: {stdout}");
    assert!(stdout.contains("app.py:6"), "impact edge survived: {stdout}");
}

#[test]
fn prebuilt_index_restore_carries_on_hash_match() {
    // Amendment 9: a restored index.db (CI cache) whose file mtimes differ
    // from the fresh checkout is carried on content-hash match — no re-parse,
    // correct queries, no stale flags.
    let repo = fixture("m4-restore");
    let (_, out1, _) = build(&repo);
    assert!(out1.contains("carried=0"), "{out1}");

    // Cache the index artifact.
    let cache = temp_repo("m4-cache");
    fs::create_dir_all(&cache).unwrap();
    fs::copy(repo.join(".kg/index.db"), cache.join("index.db")).unwrap();

    // Fresh checkout: same content, new mtimes; drop .kg; restore the artifact.
    fs::write(repo.join("utils.py"), UTILS).unwrap();
    fs::write(repo.join("widget.py"), WIDGET).unwrap();
    fs::write(repo.join("app.py"), APP).unwrap();
    fs::remove_dir_all(repo.join(".kg")).unwrap();
    fs::create_dir_all(repo.join(".kg")).unwrap();
    fs::copy(cache.join("index.db"), repo.join(".kg/index.db")).unwrap();

    let (_, out2, _) = build(&repo);
    assert!(
        out2.contains("carried=3"),
        "hash-carried despite mtime mismatch: {out2}"
    );

    // Queries correct after restore; no staleness flag (mtime refreshed).
    let out = kg(&repo).args(["callers", "normalize"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout.contains("app.py:6"), "{stdout}");
    assert!(
        !stdout.starts_with("kg: 1 of "),
        "carried rows must not read as stale: {stdout}"
    );

    // And a real edit still re-resolves correctly on top of the restore.
    write(&repo, "utils.py", &format!("{UTILS}\n\ndef extra():\n    return 1\n"));
    let (_, _, _) = build(&repo);
    let out = kg(&repo).args(["callers", "normalize"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("app.py:6"), "dirty closure on restored index: {stdout}");
}

#[test]
fn unresolved_count_stable_across_incremental_builds() {
    let repo = temp_repo("unres");
    write(&repo, "m.py", "def go():\n    return ghost()\n");
    let (_, out1, _) = build(&repo);
    assert!(out1.contains("unresolved=1"), "{out1}");
    // Re-resolve the same file: the total must not inflate.
    write(&repo, "m.py", "def go():\n    return ghost()\n# c\n");
    let (_, out2, _) = build(&repo);
    assert!(out2.contains("unresolved=1"), "{out2}");
}

#[test]
fn callees_depth_two_is_transitive() {
    let repo = temp_repo("depth");
    write(
        &repo,
        "a.py",
        "def leaf():\n    return 1\n\ndef mid():\n    return leaf()\n\ndef top():\n    return mid()\n",
    );
    build(&repo);
    let out = kg(&repo).args(["callees", "top", "--depth", "2"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("a.py:4"), "direct callee mid: {stdout}");
    assert!(stdout.contains("a.py:1"), "transitive callee leaf: {stdout}");
}

#[test]
fn impact_unknown_path_exit1() {
    let repo = fixture("impact-neg");
    build(&repo);
    let out = kg(&repo).args(["impact", "nope.py"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not indexed"), "{stderr}");
}

#[test]
fn dead_lists_uncalled_and_excludes_called() {
    // fixture: normalize is called; Widget.render is called (self.render and
    // app.run's w.render()); run, caller, and render2 have zero inbound edges.
    let repo = fixture("dead");
    build(&repo);
    let out = kg(&repo).args(["dead"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.starts_with("kg: dead-code candidates —"),
        "caveat header first: {stdout}"
    );
    assert!(stdout.contains("app.py:4"), "run is dead: {stdout}");
    assert!(stdout.contains("utils.py:4"), "caller is dead: {stdout}");
    assert!(stdout.contains("widget.py:5"), "render2 is dead: {stdout}");
    assert!(!stdout.contains("utils.py:1"), "normalize is called: {stdout}");
    assert!(!stdout.contains("widget.py:1"), "Widget is constructed: {stdout}");
    assert!(!stdout.contains("widget.py:2"), "render is called: {stdout}");
}

#[test]
fn unresolved_reports_counts_and_zero_is_exit1() {
    let repo = temp_repo("unresolved");
    write(&repo, "m.py", "def go():\n    return ghost()\n");
    build(&repo);
    let out = kg(&repo).args(["unresolved"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout.contains("kg: unresolved=1 callees in 1 files"), "{stdout}");
    assert!(stdout.contains("m.py unresolved=1"), "{stdout}");

    let clean = temp_repo("unresolved-clean");
    write(&clean, "a.py", "def g():\n    return 1\n\ndef go():\n    return g()\n");
    build(&clean);
    let out = kg(&clean).args(["unresolved"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("0 unresolved callees"), "{stderr}");
}

#[test]
fn symbols_lists_file_symbols_and_unknown_exit1() {
    let repo = fixture("symbols");
    build(&repo);
    let out = kg(&repo).args(["symbols", "utils.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout.contains("utils.py:1 module utils"), "{stdout}");
    assert!(stdout.contains("utils.py:1 function utils.normalize"), "{stdout}");
    assert!(stdout.contains("utils.py:4 function utils.caller"), "{stdout}");

    let out = kg(&repo).args(["symbols", "nope.py"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not indexed"), "{stderr}");
}

#[test]
fn journal_written_and_usage_summarizes() {
    // Amendment 6: build + callers journal entries; kg usage aggregates them.
    let repo = fixture("journal");
    kg(&repo).env_remove("KG_NO_JOURNAL").args(["build"]).output().unwrap();
    kg(&repo).env_remove("KG_NO_JOURNAL").args(["callers", "normalize"]).output().unwrap();
    let log = fs::read_to_string(repo.join(".kg/usage.log")).unwrap();
    assert_eq!(log.lines().count(), 2, "one entry per invocation: {log}");
    assert!(log.contains("\"tool\":\"build\""), "{log}");
    assert!(log.contains("\"tool\":\"callers\""), "{log}");
    assert!(log.contains("\"surface\":\"cli\""), "{log}");

    let out = kg(&repo).env_remove("KG_NO_JOURNAL").args(["usage"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout.contains("kg usage: 2 journaled calls"), "{stdout}");
    assert!(stdout.contains("callers: 1 calls"), "{stdout}");
    assert!(stdout.contains("build: 1 calls"), "{stdout}");
}

#[test]
fn journal_opt_out_via_env_and_flag() {
    let repo = fixture("journal-env");
    kg(&repo).env("KG_NO_JOURNAL", "1").args(["build"]).output().unwrap();
    assert!(!repo.join(".kg/usage.log").exists(), "env opt-out must suppress the journal");

    let repo = fixture("journal-flag");
    kg(&repo).args(["--no-journal", "build"]).output().unwrap();
    assert!(!repo.join(".kg/usage.log").exists(), "--no-journal must suppress the journal");

    // usage on an empty journal: exit 1 with a reason.
    let out = kg(&repo).args(["usage"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no journal entries"), "{stderr}");
}

#[test]
fn g5_no_child_processes_survive() {
    // Invariant 2: zero long-lived children after each subcommand. Run a build
    // tagged with a unique token, then assert nothing matching the token is
    // alive (the kg process itself has exited).
    let repo = fixture("proc");
    let marker = format!("kg-proc-{}", std::process::id());
    write(&repo, &format!("{marker}.py"), "def f():\n    pass\n");
    let out = kg(&repo).args(["build"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let probe = Command::new("pgrep")
        .args(["-f", &marker])
        .output()
        .unwrap();
    let alive = String::from_utf8_lossy(&probe.stdout);
    assert!(alive.trim().is_empty(), "processes still alive: {alive}");
}

#[test]
fn p15_stats_hotspots_report() {
    let repo = fixture("p15");
    build(&repo);
    let out = kg(&repo).args(["stats"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout.contains("files=3") && stdout.contains("symbols=") && stdout.contains("edges="), "{stdout}");

    let out = kg(&repo).args(["hotspots"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout.contains("loc=") && stdout.contains("cyc="), "{stdout}");

    let out = kg(&repo).args(["report"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    for section in ["## 1. index overview", "## 2. architecture map", "## 3. hotspots", "## 4. most-called symbols"] {
        assert!(stdout.contains(section), "missing {section}: {stdout}");
    }
}
