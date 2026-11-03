//! Integration tests: CUJs 1-8, G4 staleness, and the read-only tree
//! snapshot invariant, exercised against a throwaway fixture repo.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn temp_repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aw-it-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(repo: &Path, rel: &str, content: &str) {
    let path = repo.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn aw(repo: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_aw"));
    c.arg("--root").arg(repo);
    c
}

fn build(repo: &Path) -> (i32, String, String) {
    let out = aw(repo).args(["build"]).output().unwrap();
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

/// Walk the tree collecting (path, len) for files outside .aw/ — the read-only
/// invariant probe.
fn snapshot(repo: &Path) -> Vec<(String, u64, u64)> {
    // (path, len, content hash) — hashes catch same-size edits (invariant 3).
    let mut out = Vec::new();
    fn rec(dir: &Path, acc: &mut Vec<(String, u64, u64)>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.file_name().is_some_and(|n| n == ".aw") {
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
    let line = stdout
        .lines()
        .find(|l| l.starts_with("aw: built index"))
        .unwrap();
    for key in ["files=3", "parse_errors=0", "wall="] {
        assert!(line.contains(key), "stats line missing {key}: {line}");
    }
    assert!(repo.join(".aw/index.db").exists());
    assert!(repo.join(".aw/.gitignore").exists());
    let after = snapshot(&repo);
    assert_eq!(before, after, "tree changed outside .aw/ (invariant 3)");
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

    let out = aw(&repo).args(["callers", "normalize"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    let pointers: Vec<&str> = stdout
        .lines()
        .filter(|l| l.contains(':') && !l.starts_with("aw:"))
        .collect();
    assert_eq!(pointers.len(), 50, "cap default 50");
    assert!(pointers.iter().all(|l| l.split(':').count() >= 2));
    assert!(stdout.contains("showing 50 of 60"), "stdout: {stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("[aw] capped at 50 results (60 total)"),
        "stderr: {stderr}"
    );
}

#[test]
fn cuj3_corrupt_index_exit3() {
    let repo = fixture("cuj3");
    build(&repo);
    fs::write(repo.join(".aw/index.db"), b"garbage").unwrap();
    let out = aw(&repo).args(["callers", "normalize"]).output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("aw: corrupt-index:"), "{stderr}");
    assert!(stdout_is_clean(&out.stdout));
}

fn stdout_is_clean(stdout: &[u8]) -> bool {
    // CUJ3: no partial output on stdout.
    stdout.is_empty()
}

#[test]
fn cuj4_ambiguous_name_exit2_with_candidates() {
    let repo = fixture("cuj4");
    write(
        &repo,
        "a.py",
        "class A:\n    def process(self):\n        return 1\n",
    );
    write(
        &repo,
        "b.py",
        "class B:\n    def process(self):\n        return 2\n",
    );
    build(&repo);
    let out = aw(&repo).args(["callers", "process"]).output().unwrap();
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
            "import sqlite3; c = sqlite3.connect('{}/.aw/index.db'); c.execute('PRAGMA user_version = 99'); c.commit()",
            repo.display()
        ))
        .output()
        .unwrap();
    assert!(out.status.success());
    let out = aw(&repo).args(["callers", "normalize"]).output().unwrap();
    assert_eq!(out.status.code(), Some(5));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("schema v99, binary expects v7 — run aw build"),
        "{stderr}"
    );
}

#[test]
fn cuj6_concurrent_build_lock_exit4() {
    let repo = fixture("cuj6");
    // Hold the lock from this process; the child build must exit 4.
    let lock_path = repo.join(".aw/build.lock");
    fs::create_dir_all(repo.join(".aw")).unwrap();
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .read(true)
        .open(&lock_path)
        .unwrap();
    file.try_lock().unwrap();
    let out = aw(&repo).args(["build"]).output().unwrap();
    assert_eq!(out.status.code(), Some(4));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("aw: locked:"), "{stderr}");
    drop(file);
}

#[test]
fn cuj7_query_without_index_exit1() {
    let repo = temp_repo("cuj7");
    write(&repo, "a.py", "def f():\n    pass\n");
    let out = aw(&repo).args(["callers", "f"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("aw: no-results: no .aw/index.db under") && stderr.contains("run aw build"),
        "{stderr}"
    );
}

#[test]
fn cuj8_parse_quality_abort_exit7() {
    let repo = fixture("cuj8");
    write(&repo, "broken.py", BROKEN);
    // 1/4 files broken = 25% > default 5%.
    let out = aw(&repo).args(["build"]).output().unwrap();
    assert_eq!(out.status.code(), Some(7));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("aw: parse-quality: 25% of 4 files"),
        "{stderr}"
    );
    // --force proceeds.
    let out = aw(&repo).args(["build", "--force"]).output().unwrap();
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
    let out = aw(&repo).args(["callers", "normalize"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("aw: 1 of "),
        "staleness header first line: {stdout}"
    );
    assert!(stdout.contains("result files changed since build — run aw build"));
    // FNV-1a re-hash path: same size, different content -> still stale.
    // Nanosecond mtimes make the mtime mismatch (and thus the re-hash)
    // deterministic; both result files now report changed.
    let original = fs::read_to_string(repo.join("utils.py")).unwrap();
    let swapped: String = original.chars().rev().collect();
    fs::write(repo.join("utils.py"), &swapped).unwrap();
    let out = aw(&repo).args(["callers", "normalize"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("aw: 2 of "),
        "re-hash staleness: {stdout}"
    );
}

#[test]
fn symbols_directory_unknown_reports_zero_symbols() {
    let repo = temp_repo("symbols-unknown-dir");
    write(&repo, "pkg/a.py", "def a():\n    pass\n");
    build(&repo);

    let out = aw(&repo).args(["symbols", "nosuchdir"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("0 symbols in 'nosuchdir' (indexed)"),
        "{combined}"
    );
}

#[test]
fn context_limit_is_capped_at_50() {
    let repo = temp_repo("context-cap");
    write(&repo, "a.py", "def target():\n    pass\n");
    let mut callers = String::from("from a import target\n");
    for i in 0..60 {
        callers.push_str(&format!("def c{i}():\n    target()\n"));
    }
    write(&repo, "b.py", &callers);
    build(&repo);

    let out = aw(&repo)
        .args(["context", "target", "--limit", "10000"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("callers (showing 50 of 60):"),
        "context limit not capped: {stdout}"
    );
}

#[test]
fn timing_assert_measures_semantic_exit_codes() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scripts/timing_assert.sh");
    let out = std::process::Command::new("bash")
        .arg(script)
        .args(["2", "exit-one", "5000", "sh", "-c", "exit 1"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("best-of-2"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn path_scope_rejects_embedded_dotdot_and_api_root() {
    let repo = fixture("path-invalid");
    build(&repo);
    for args in [
        vec!["files", "--path", "../x"],
        vec!["files", "--path", "a/../b"],
        vec!["api", ""],
        vec!["api", "."],
    ] {
        let out = aw(&repo).args(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn cleanup_removed_features_are_unrecognized() {
    let repo = fixture("path-invalid");
    build(&repo);
    for args in [
        vec!["api", "a.py"],
        vec!["search", "x", "--kind", "function"],
        vec!["search", "x", "--path", "src"],
        vec!["search", "x", "--exact"],
        vec!["dead", "--exclude-generated"],
        vec!["complexity", "--exclude-generated"],
        vec!["clones", "--exclude-generated"],
        vec!["hotspots", "--exclude-generated"],
        vec!["report", "--exclude-generated"],
    ] {
        let out = aw(&repo).args(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn impact_sites_empty_reason_keeps_edge_suffix() {
    let repo = temp_repo("impact-empty-sites");
    write(&repo, "a.py", "def f():\n    pass\n");
    build(&repo);
    let out = aw(&repo)
        .args(["impact", "a.py", "--edges", "imports", "--sites"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr.contains("0 impacting files found (unresolved imports are omitted)"),
        "{stderr}"
    );
}

#[test]
fn dead_exclude_tests_matches_nested_test_filename() {
    let repo = temp_repo("dead-nested-test");
    write(&repo, "pkg/test_helpers.py", "def helper():\n    pass\n");
    build(&repo);
    let out = aw(&repo)
        .args(["dead", "--exclude-tests"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("pkg/test_helpers.py"), "{stdout}");
}

#[test]
fn incremental_carries_trait_and_qualified_impl_targets() {
    let repo = temp_repo("incremental-trait");
    write(
        &repo,
        "src/lib.rs",
        "pub trait Greet {\n    fn hello(&self);\n}\n",
    );
    build(&repo);
    write(
        &repo,
        "src/user.rs",
        "use crate::Greet;\npub struct Thing;\nimpl Greet for Thing {\n    fn hello(&self) {}\n}\n",
    );
    build(&repo);
    let out = aw(&repo).args(["implementers", "Greet"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("src/user.rs:2 src.user.Thing"), "{stdout}");

    write(
        &repo,
        "src/qualified.rs",
        "use crate::Greet;\npub struct Other;\nimpl crate::Greet for Other {\n    fn hello(&self) {}\n}\n",
    );
    build(&repo);
    let out = aw(&repo).args(["implementers", "Greet"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("src/qualified.rs:2 src.qualified.Other"),
        "{stdout}"
    );
}

#[test]
fn root_ts_index_and_python_root_parent_imports_resolve() {
    let repo = temp_repo("root-imports");
    write(&repo, "index.ts", "export function idx() {}\n");
    write(
        &repo,
        "rootidx.ts",
        "import { idx } from './index';\n\nexport function use() {\n    return idx();\n}\n",
    );
    write(&repo, "app.py", "def app():\n    pass\n");
    write(&repo, "pkg/__init__.py", "");
    write(
        &repo,
        "pkg/relinner.py",
        "from .. import app\n\nuse = app\n",
    );
    build(&repo);

    let out = aw(&repo).args(["imports", "rootidx.ts"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("rootidx.ts:1 root"), "{stdout}");
    assert!(stdout.contains("rootidx.ts:1 root.idx"), "{stdout}");
    let out = aw(&repo)
        .args(["imports", "pkg/relinner.py"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("pkg/relinner.py:1 app"), "{stdout}");
    let out = aw(&repo).args(["unresolved"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "no unresolved expected");
}

#[test]
fn pre_manifest_index_rebuilds_instead_of_empty_complexity() {
    let repo = temp_repo("pre-manifest");
    write(
        &repo,
        "app.py",
        "def f(x):\n    if x:\n        return 1\n    return 0\n\ndef g():\n    if true:\n        return 2\n    return 3\n",
    );
    build(&repo);
    let idx = repo.join(".aw/index.db");
    let status = std::process::Command::new("sqlite3")
        .args([
            idx.to_string_lossy().as_ref(),
            "DELETE FROM meta WHERE key='callable_count';",
        ])
        .output()
        .unwrap();
    assert!(status.status.success(), "sqlite3 demotion failed");
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let out = aw(&repo)
        .args(["complexity", "--limit", "1"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("showing 1 of"), "{stdout}");
}

#[test]
fn response_contract_limit_clamps() {
    let repo = fixture("limit");
    build(&repo);
    let out = aw(&repo)
        .args(["callers", "normalize", "--limit", "0"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = aw(&repo)
        .args(["callers", "normalize", "--limit", "999999"])
        .output()
        .unwrap();
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
    let out = aw(&repo).args(["callers", "normalize"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("app.py:6"),
        "baseline caller present: {stdout}"
    );
    // Change utils.py (fresh); app.py/widget.py are carried.
    write(
        &repo,
        "utils.py",
        &format!("{UTILS}\n\ndef extra():\n    return 1\n"),
    );
    build(&repo);
    let out = aw(&repo).args(["callers", "normalize"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(0),
        "caller edges must survive: {stdout}"
    );
    assert!(
        stdout.contains("app.py:6"),
        "app.py caller survived: {stdout}"
    );
    assert!(
        stdout.contains("utils.py:5"),
        "utils.py caller survived: {stdout}"
    );
    let out = aw(&repo).args(["impact", "utils.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "impact must survive: {stdout}");
    assert!(
        stdout.contains("app.py:6"),
        "impact edge survived: {stdout}"
    );
}

#[test]
fn pre_signature_version_index_rebuilds_once() {
    // Amendment 18: an index built before multi-line signature capture must
    // re-extract once (meta marker absent), not carry truncated signatures.
    let repo = fixture("signature-upgrade");
    build(&repo);
    let db = repo.join(".aw/index.db");
    let out = Command::new("python3")
        .arg("-c")
        .arg(format!(
            "import sqlite3; c=sqlite3.connect('{}'); c.execute(\"DELETE FROM meta WHERE key='signature_version'\"); c.commit()",
            db.display()
        ))
        .output()
        .unwrap();
    assert!(out.status.success());
    let (_, out2, _) = build(&repo);
    assert!(out2.contains("carried=0"), "full re-extract: {out2}");
    let out = aw(&repo).args(["search", "normalize"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("def normalize(text)"));
}

#[test]
fn noop_build_leaves_the_index_untouched() {
    // R7 L4: a build with zero changed/deleted files must not rewrite the
    // database (0.05-0.06 s of a 0.10 s build at 2.2k files).
    let repo = fixture("noop-build");
    let (_, out1, _) = build(&repo);
    assert!(out1.contains("carried=0"), "{out1}");
    let db = repo.join(".aw/index.db");
    let stamp = |p: &Path| -> (u64, std::time::SystemTime) {
        let m = fs::metadata(p).unwrap();
        (m.len(), m.modified().unwrap())
    };
    let before = stamp(&db);
    std::thread::sleep(std::time::Duration::from_millis(20));
    let (code, out2, err2) = build(&repo);
    assert_eq!(code, 0, "{err2}");
    assert!(out2.contains("carried=3"), "{out2}");
    assert!(out2.contains("db=0.00s"), "no-op write skipped: {out2}");
    let after = stamp(&db);
    assert_eq!(before, after, "no-op build rewrote index.db: {out2}");
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
    fs::copy(repo.join(".aw/index.db"), cache.join("index.db")).unwrap();

    // Fresh checkout: same content, new mtimes; drop .aw; restore the artifact.
    fs::write(repo.join("utils.py"), UTILS).unwrap();
    fs::write(repo.join("widget.py"), WIDGET).unwrap();
    fs::write(repo.join("app.py"), APP).unwrap();
    fs::remove_dir_all(repo.join(".aw")).unwrap();
    fs::create_dir_all(repo.join(".aw")).unwrap();
    fs::copy(cache.join("index.db"), repo.join(".aw/index.db")).unwrap();

    let (_, out2, _) = build(&repo);
    assert!(
        out2.contains("carried=3"),
        "hash-carried despite mtime mismatch: {out2}"
    );

    // Queries correct after restore; no staleness flag (mtime refreshed).
    let out = aw(&repo).args(["callers", "normalize"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout.contains("app.py:6"), "{stdout}");
    assert!(
        !stdout.starts_with("aw: 1 of "),
        "carried rows must not read as stale: {stdout}"
    );

    // And a real edit still re-resolves correctly on top of the restore.
    write(
        &repo,
        "utils.py",
        &format!("{UTILS}\n\ndef extra():\n    return 1\n"),
    );
    let (_, _, _) = build(&repo);
    let out = aw(&repo).args(["callers", "normalize"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("app.py:6"),
        "dirty closure on restored index: {stdout}"
    );
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
    // R6d A3: depth N must close exactly N levels. Chain: top -> mid ->
    // leaf -> base (as.py lines 7, 4, 1).
    let repo = temp_repo("depth");
    write(
        &repo,
        "a.py",
        "def base():\n    return 0\n\ndef leaf():\n    return base()\n\ndef mid():\n    return leaf()\n\ndef top():\n    return mid()\n",
    );
    build(&repo);
    let out = aw(&repo)
        .args(["callees", "top", "--depth", "2"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("a.py:7"), "direct callee mid: {stdout}");
    assert!(
        stdout.contains("a.py:4"),
        "transitive callee leaf: {stdout}"
    );
    assert!(
        !stdout.contains("a.py:1"),
        "depth 2 must NOT include the distance-3 callee base: {stdout}"
    );
}

#[test]
fn impact_unknown_path_exit1() {
    let repo = fixture("impact-neg");
    build(&repo);
    let out = aw(&repo).args(["impact", "nope.py"]).output().unwrap();
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
    let out = aw(&repo).args(["dead"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.starts_with("aw: dead-code candidates —"),
        "caveat header first: {stdout}"
    );
    assert!(stdout.contains("app.py:4"), "run is dead: {stdout}");
    assert!(stdout.contains("utils.py:4"), "caller is dead: {stdout}");
    assert!(stdout.contains("widget.py:5"), "render2 is dead: {stdout}");
    assert!(
        !stdout.contains("utils.py:1"),
        "normalize is called: {stdout}"
    );
    assert!(
        !stdout.contains("widget.py:1"),
        "Widget is constructed: {stdout}"
    );
    assert!(
        !stdout.contains("widget.py:2"),
        "render is called: {stdout}"
    );
}

#[test]
fn p8a_implementers_lists_subclasses_and_dead_keeps_inherited_base() {
    let repo = temp_repo("implementers");
    write(
        &repo,
        "hier.py",
        "class Base:\n    def run(self):\n        pass\n\nclass Child(Base):\n    pass\n\nclass Uses(Base):\n    pass\n",
    );
    build(&repo);

    let out = aw(&repo).args(["implementers", "Base"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.starts_with("aw: implementers — resolved static inherits edges only"),
        "caveat first: {stdout}"
    );
    assert!(stdout.contains("hier.py:5 hier.Child"), "{stdout}");
    assert!(stdout.contains("hier.py:8 hier.Uses"), "{stdout}");

    let out = aw(&repo).args(["dead"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("hier.py:1"),
        "Base has inheritors and is not dead: {stdout}"
    );
}

#[test]
fn p8a_implementers_resolves_rust_trait_impl_same_file() {
    let repo = temp_repo("rust-trait");
    write(
        &repo,
        "lib.rs",
        "pub trait Greet {\n    fn hello(&self);\n}\n\npub struct English;\n\nimpl Greet for English {\n    fn hello(&self) {}\n}\n",
    );
    build(&repo);

    let out = aw(&repo).args(["symbols", "lib.rs"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("trait lib.Greet"), "{stdout}");
    assert!(stdout.contains("class lib.English"), "{stdout}");

    let out = aw(&repo).args(["implementers", "Greet"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("lib.rs:5 lib.English"), "{stdout}");
}

#[test]
fn p8a_impact_edges_include_importers_and_inheritors() {
    let repo = temp_repo("impact-edges");
    write(
        &repo,
        "hier.py",
        "class Base:\n    pass\n\nclass Child(Base):\n    pass\n",
    );
    write(&repo, "imports.py", "from hier import Base\n");
    build(&repo);

    let out = aw(&repo)
        .args(["impact", "hier.py", "--edges", "imports"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("imports.py:1"), "{stdout}");
    assert!(
        stdout.starts_with("aw: impact — resolved static edges only"),
        "caveat first: {stdout}"
    );

    let out = aw(&repo)
        .args(["impact", "hier.py", "--edges", "inherits"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("hier.py:4"), "{stdout}");
}

#[test]
fn import_python_dotted_import_keeps_parent_module_edges() {
    let repo = temp_repo("py-dotted-parents");
    write(&repo, "pkg/__init__.py", "");
    write(&repo, "pkg/sub/__init__.py", "");
    write(&repo, "pkg/sub/b.py", "def f():\n    pass\n");
    write(&repo, "app.py", "import pkg.sub.b\n");
    build(&repo);

    let out = aw(&repo).args(["imports", "app.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    for row in ["app.py:1 pkg", "app.py:1 pkg.sub", "app.py:1 pkg.sub.b"] {
        assert!(stdout.contains(row), "missing {row}: {stdout}");
    }
}

#[test]
fn import_rust_root_modules_wildcard_and_single_segment_resolve() {
    let repo = temp_repo("rust-roots");
    write(
        &repo,
        "src/main.rs",
        "mod a;\nuse crate::rootfn;\nfn main() {\n    rootfn();\n}\n",
    );
    write(&repo, "src/lib.rs", "pub fn rootfn() {}\n");
    write(&repo, "src/a.rs", "pub fn f() {}\n");
    write(&repo, "lib.rs", "mod a;\nuse crate::a::f;\n");
    write(&repo, "a.rs", "pub fn f() {}\n");
    build(&repo);

    let out = aw(&repo).args(["imports", "src/main.rs"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("src/main.rs:1 src.a"), "{stdout}");
    assert!(stdout.contains("src/main.rs:2 src.rootfn"), "{stdout}");

    let out = aw(&repo).args(["imports", "lib.rs"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("lib.rs:1 a"), "root mod mapping: {stdout}");
    assert!(stdout.contains("lib.rs:2 a.f"), "root crate use: {stdout}");

    let out = aw(&repo).args(["unresolved", "--names"]).output().unwrap();
    assert!(
        out.status.code() == Some(1),
        "root/wildcard/single-segment left unresolved: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn import_rust_wildcard_import_emits_ambiguous_module_edge() {
    let repo = temp_repo("rust-star");
    write(&repo, "src/lib.rs", "mod a;\nmod b;\n");
    write(&repo, "src/a.rs", "pub fn shared() {}\n");
    write(
        &repo,
        "src/b.rs",
        "use crate::a::*;\n\nfn use_it() {\n    shared();\n}\n",
    );
    build(&repo);

    let out = aw(&repo).args(["imports", "src/b.rs"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("src/b.rs:1 src.a"), "star edge: {stdout}");
    let out = aw(&repo).args(["callers", "shared"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("src/b.rs:4"),
        "star call resolution: {stdout}"
    );
}

#[test]
fn import_python_star_import_heals_after_restore() {
    let repo = temp_repo("py-star-heal");
    write(&repo, "pkg/__init__.py", "");
    write(&repo, "pkg/later.py", "def gone():\n    pass\n");
    write(&repo, "app.py", "from pkg.later import *\n");
    build(&repo);
    fs::remove_file(repo.join("pkg/later.py")).unwrap();
    build(&repo);
    let out = aw(&repo).args(["unresolved", "--names"]).output().unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("app.py:1 import pkg.later"),
        "star deletion must leave an unresolved site: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    write(&repo, "pkg/later.py", "def gone():\n    pass\n");
    build(&repo);
    let out = aw(&repo).args(["imports", "app.py"]).output().unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("app.py:1 pkg.later"),
        "star restore must heal: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn unresolved_inherit_reports_cross_file_and_mixin_calls() {
    let repo = temp_repo("unresolved-inherit");
    write(
        &repo,
        "src/traits.rs",
        "pub trait Shape {\n    fn area(&self);\n}\n",
    );
    write(&repo, "src/cross.rs", "pub struct Cross;\n");
    write(
        &repo,
        "src/use.rs",
        "use crate::traits::Shape;\nuse crate::cross::Cross;\n\nimpl Shape for Cross {}\n",
    );
    write(&repo, "web/a.ts", "class Bad extends mixin(1) {}\n");
    build(&repo);

    let out = aw(&repo).args(["unresolved", "--names"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("src/use.rs:4 inherit Shape"),
        "cross-file impl unresolved row: {stdout}"
    );
    assert!(
        stdout.contains("web/a.ts:1 inherit mixin"),
        "mixin unresolved row: {stdout}"
    );
}

#[test]
fn impact_import_site_counts_one_per_statement() {
    let repo = temp_repo("impact-import-sites");
    write(&repo, "a.py", "def target():\n    return 1\n");
    write(&repo, "b.py", "from a import target\n");
    build(&repo);

    let out = aw(&repo)
        .args(["impact", "a.py", "--edges", "imports"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("b.py:1 (1 sites)"),
        "module+symbol edges counted twice: {stdout}"
    );

    let out = aw(&repo)
        .args(["impact", "a.py", "--edges", "imports", "--sites"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.matches("b.py:1").count(),
        1,
        "duplicate site rows: {stdout}"
    );

    let out = aw(&repo)
        .args(["impact", "a.py", "--edges", "imports", "--context"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("[in b]"),
        "module-level site tagged as context: {stdout}"
    );
    assert!(
        stdout.starts_with("aw: impact — resolved static edges only"),
        "imports context caveat missing: {stdout}"
    );
}

#[test]
fn impact_context_tags_only_callable_sites() {
    let repo = temp_repo("impact-context");
    write(&repo, "a.py", "def target():\n    return 1\n");
    write(
        &repo,
        "b.py",
        "from a import target\n\ndef caller():\n    return target()\n",
    );
    build(&repo);

    let out = aw(&repo)
        .args(["impact", "a.py", "--context", "--edges", "calls"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("b.py:4 [in b.caller]"),
        "callable context tag missing: {stdout}"
    );
    assert!(
        stdout.starts_with("aw: impact — resolved static call edges only"),
        "calls-only context caveat missing: {stdout}"
    );
}

#[test]
fn impact_depth_footer_only_when_frontier_extended() {
    let repo = temp_repo("impact-depth-footer");
    write(&repo, "c.py", "def h():\n    pass\n");
    write(&repo, "b.py", "from c import h\n\ndef g():\n    h()\n");
    build(&repo);

    let out = aw(&repo)
        .args(["impact", "c.py", "--depth", "5", "--edges", "calls"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        !stdout.contains("depth cap"),
        "exhausted BFS reported a cap: {stdout}"
    );
}

#[test]
fn p8a_impact_depth_two_reaches_transitive_callers() {
    let repo = temp_repo("impact-depth");
    write(&repo, "c.py", "def h():\n    pass\n");
    write(&repo, "b.py", "from c import h\n\ndef g():\n    h()\n");
    write(&repo, "a.py", "from b import g\n\ndef f():\n    g()\n");
    build(&repo);

    let out = aw(&repo)
        .args(["impact", "c.py", "--depth", "2", "--edges", "calls"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("b.py"), "{stdout}");
    assert!(stdout.contains("a.py"), "{stdout}");
}

#[test]
fn impact_depth_two_includes_module_level_caller() {
    let repo = temp_repo("impact-module-caller");
    write(&repo, "m1.py", "def f():\n    f()\n");
    write(&repo, "mid.py", "from m1 import f\n\ndef g():\n    f()\n");
    write(&repo, "other.py", "from m1 import f\n\ndef h():\n    f()\n");
    write(&repo, "top.py", "from mid import g\n\ng()\n");
    build(&repo);

    let out = aw(&repo)
        .args(["impact", "m1.py", "--depth", "2"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("mid.py"), "{stdout}");
    assert!(
        stdout.contains("top.py"),
        "module-level caller missing: {stdout}"
    );
}

#[test]
fn p8a_symbols_directory_groups_files_and_counts_symbols() {
    let repo = temp_repo("symbols-dir");
    write(&repo, "pkg/a.py", "def a():\n    pass\n");
    write(&repo, "pkg/b.py", "def b():\n    pass\n");
    build(&repo);

    let out = aw(&repo)
        .args(["symbols", "pkg", "--outline"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("aw: dir pkg — 2 files"), "{stdout}");
    assert!(stdout.contains("pkg/a.py"), "{stdout}");
    assert!(stdout.contains("pkg/b.py"), "{stdout}");
    assert!(stdout.contains("function pkg.a.a"), "{stdout}");
}

#[test]
fn import_edges_cover_plain_and_from_imports() {
    let repo = temp_repo("import-edges");
    write(&repo, "a.py", "def only():\n    pass\n");
    write(
        &repo,
        "b.py",
        "from a import only\n\ndef use():\n    only()\n",
    );
    write(&repo, "c.py", "import a\n");
    write(&repo, "d.py", "from a import only\n");
    build(&repo);

    let out = aw(&repo)
        .args(["impact", "a.py", "--edges", "imports"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("b.py:1"), "{stdout}");
    assert!(stdout.contains("c.py:1"), "plain import edge: {stdout}");
    assert!(stdout.contains("d.py:1"), "{stdout}");
}

#[test]
fn imports_lists_outbound_resolved_edges() {
    let repo = temp_repo("imports-cmd");
    write(&repo, "a.py", "def only():\n    pass\n");
    write(
        &repo,
        "b.py",
        "from a import only\n\ndef use():\n    only()\n",
    );
    build(&repo);

    let out = aw(&repo).args(["imports", "b.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.starts_with("aw: imports —"), "{stdout}");
    assert!(stdout.contains("b.py:1 a.only"), "{stdout}");
}

#[test]
fn ts_arrow_metrics_use_the_arrow_range() {
    let repo = temp_repo("ts-arrow-range");
    write(
        &repo,
        "app.ts",
        "const arrow =\n  (\n    x: number,\n  ) => {\n    return x;\n  };\n",
    );
    build(&repo);

    let out = aw(&repo)
        .args(["symbols", "app.ts", "--metrics"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("app.arrow") && stdout.contains("loc=5"),
        "arrow metrics range is inconsistent: {stdout}"
    );
}

#[test]
fn effective_loc_uses_comment_node_coverage() {
    let repo = temp_repo("effective-loc");
    write(
        &repo,
        "rs.rs",
        "fn block_tail() {\n    let x = 1;\n    let y = 1; /* start\n    comment\n    comment */\n    x + y\n}\n\nfn raw_str() {\n    let s = r#\"line\n// literal\nline\"#;\n    s.len()\n}\n",
    );
    build(&repo);
    let out = aw(&repo)
        .args(["symbols", "rs.rs", "--metrics"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("rs.block_tail") && stdout.contains("loc=5"),
        "{stdout}"
    );
    assert!(
        stdout.contains("rs.raw_str") && stdout.contains("loc=6"),
        "{stdout}"
    );
}

#[test]
fn p8c_complexity_excludes_zero_metric_trait_signatures() {
    let repo = temp_repo("complexity-signatures");
    write(
        &repo,
        "lib.rs",
        "trait Shape {\n    fn area(&self) -> f64;\n}\n\n#[inline]\nfn real() {\n    // comment\n}\n",
    );
    build(&repo);

    let out = aw(&repo)
        .args(["complexity", "--min-cyc", "0"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        !stdout.contains("Shape.area"),
        "trait signature listed: {stdout}"
    );
    assert!(
        stdout.contains("lib.real"),
        "real function missing: {stdout}"
    );
    assert!(
        stdout.contains("loc=2"),
        "attribute/comment lines changed loc: {stdout}"
    );
}

#[test]
fn import_ts_relative_imports_collapse_extensions_and_index() {
    let repo = temp_repo("ts-extensions");
    write(
        &repo,
        "src/app.ts",
        "import Widget from './widget';\nimport { helper } from './util.mjs';\nimport * as ns from './pkg/index';\nexport * from './reexport';\nconst dyn = import('./lazy.cjs');\nconst req = require('./legacy.mjs');\n",
    );
    write(&repo, "src/widget.ts", "export default class Widget {}\n");
    write(&repo, "src/util.mts", "export function helper() {}\n");
    write(&repo, "src/pkg/index.ts", "export function inner() {}\n");
    write(&repo, "src/reexport.ts", "export function re() {}\n");
    write(&repo, "src/lazy.cts", "export function lazy() {}\n");
    write(&repo, "src/legacy.cts", "export function legacy() {}\n");
    build(&repo);

    let out = aw(&repo).args(["imports", "src/app.ts"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    for row in [
        "src/app.ts:1 src.widget",
        "src/app.ts:2 src.util",
        "src/app.ts:2 src.util.helper",
        "src/app.ts:3 src.pkg",
        "src/app.ts:4 src.reexport",
        "src/app.ts:5 src.lazy",
        "src/app.ts:6 src.legacy",
    ] {
        assert!(stdout.contains(row), "missing {row}: {stdout}");
    }
    assert!(
        !stdout.contains("src.widget.default"),
        "default import leaked a symbol edge: {stdout}"
    );

    let out = aw(&repo).args(["unresolved"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "TS imports left unresolved rows: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn import_python_from_import_module_suppresses_redundant_parent_edge() {
    let repo = temp_repo("from-import-module");
    write(&repo, "pkg/__init__.py", "def b():\n    pass\n");
    write(&repo, "pkg/mod.py", "def inner():\n    pass\n");
    write(&repo, "app.py", "from pkg import mod\n");
    write(&repo, "app2.py", "from pkg import mod, b\n");
    build(&repo);

    let out = aw(&repo).args(["imports", "app.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let rows: Vec<&str> = stdout
        .lines()
        .filter(|line| line.starts_with("app.py:"))
        .collect();
    assert_eq!(
        rows,
        vec!["app.py:1 pkg.mod"],
        "module-only import keeps only the child edge: {stdout}"
    );

    let out = aw(&repo).args(["imports", "app2.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("app2.py:1 pkg"),
        "symbol import must keep the parent module edge: {stdout}"
    );
    assert!(stdout.contains("app2.py:1 pkg.mod"), "{stdout}");
    assert!(stdout.contains("app2.py:1 pkg.b"), "{stdout}");
}

#[test]
fn import_rust_use_tree_and_module_mapping_resolve() {
    let repo = temp_repo("rust-use-tree");
    write(&repo, "src/lib.rs", "pub mod a;\npub mod util;\n");
    write(&repo, "src/util.rs", "pub fn u() {}\n");
    write(
        &repo,
        "src/a/mod.rs",
        "pub fn f() {}\npub struct S;\nmod inner;\n",
    );
    write(&repo, "src/a/inner.rs", "pub fn deep() {}\n");
    write(&repo, "src/a/b.rs", "pub fn g() {}\n");
    write(
        &repo,
        "src/use_tree.rs",
        "use crate::a::{f, S as T, self};\nuse super::util::u;\nuse crate::a::b::g;\nuse crate::util;\nuse std::fmt::Debug;\n\nmod outer { use crate::util::u; }\nmod child;\n",
    );
    write(&repo, "src/use_tree/child.rs", "pub fn c() {}\n");
    build(&repo);

    let out = aw(&repo)
        .args(["imports", "src/use_tree.rs"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    for row in [
        "src/use_tree.rs:1 src.a",
        "src/use_tree.rs:1 src.a.S",
        "src/use_tree.rs:1 src.a.f",
        "src/use_tree.rs:2 src.util.u",
        "src/use_tree.rs:3 src.a.b.g",
        "src/use_tree.rs:4 src.util",
        "src/use_tree.rs:7 src.util.u",
        "src/use_tree.rs:8 src.use_tree.child",
    ] {
        assert!(stdout.contains(row), "missing {row}: {stdout}");
    }
    assert!(!stdout.contains("std"), "external crate leaked: {stdout}");

    let out = aw(&repo)
        .args(["imports", "src/a/mod.rs"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("src/a/mod.rs:3 src.a.inner"),
        "mod.rs child mapping: {stdout}"
    );

    let out = aw(&repo).args(["unresolved"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "std/external use must not leave unresolved imports"
    );
}

#[test]
fn import_star_suppression_is_per_import_statement() {
    let repo = temp_repo("star-per-statement");
    write(&repo, "pkg/__init__.py", "def f():\n    pass\n");
    write(&repo, "pkg/mod.py", "def f():\n    pass\n");
    write(
        &repo,
        "app.py",
        "import pkg\nfrom pkg import *\n\ndef use():\n    return pkg.mod.f()\n",
    );
    build(&repo);

    let out = aw(&repo).args(["imports", "app.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("app.py:1 pkg"),
        "plain import suppressed by a later star: {stdout}"
    );
    assert!(
        stdout.contains("app.py:2 pkg"),
        "star edge missing: {stdout}"
    );
    let out = aw(&repo).args(["unresolved"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "star must resolve pkg.mod.f");
}

#[test]
fn import_deleted_destination_heals_unresolved_and_restore() {
    let repo = temp_repo("deletion-heal");
    write(&repo, "pkg/__init__.py", "");
    write(&repo, "pkg/later.py", "def gone():\n    pass\n");
    write(&repo, "app.py", "from pkg.later import gone\n");
    build(&repo);

    let out = aw(&repo).args(["imports", "app.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("app.py:1 pkg.later.gone"),
        "resolved import before deletion: {stdout}"
    );

    fs::remove_file(repo.join("pkg/later.py")).unwrap();
    build(&repo);
    let out = aw(&repo).args(["unresolved", "--names"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("app.py:1 import pkg.later"),
        "deleted destination must leave an unresolved import: {stdout}"
    );

    write(&repo, "pkg/later.py", "def gone():\n    pass\n");
    build(&repo);
    let out = aw(&repo).args(["imports", "app.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("app.py:1 pkg.later.gone"),
        "restored destination must heal: {stdout}"
    );
    let out = aw(&repo).args(["unresolved"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "restored destination left an unresolved row"
    );
}

#[test]
fn tests_impact_seed_covers_direct_indirect_depth_and_file_seed() {
    let repo = temp_repo("tests-cmd");
    write(&repo, "src/core.py", "def f():\n    pass\n");
    write(
        &repo,
        "src/mid.py",
        "from src.core import f\n\ndef g():\n    return f()\n",
    );
    write(
        &repo,
        "tests/test_core.py",
        "from src.core import f\n\ndef test_direct():\n    f()\n",
    );
    write(
        &repo,
        "tests/test_mid.py",
        "from src.mid import g\n\ndef test_indirect():\n    g()\n",
    );
    build(&repo);

    let out = aw(&repo).args(["tests", "f"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.starts_with("aw: test candidates —"), "{stdout}");
    assert!(stdout.contains("tests/test_core.py:3"), "{stdout}");

    // Depth 1 finds only the direct test; depth 2 adds the indirect route.
    let out = aw(&repo)
        .args(["tests", "f", "--depth", "1"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("test_indirect"), "depth 1: {stdout}");
    let out = aw(&repo)
        .args(["tests", "f", "--depth", "2"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("test_indirect"), "depth 2: {stdout}");

    // A file seed uses every symbol in that file.
    let out = aw(&repo).args(["tests", "src/mid.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("test_indirect"), "file seed: {stdout}");
}

#[test]
fn p8c_unresolved_names_lists_sites_with_kind() {
    let repo = temp_repo("unresolved-names");
    write(&repo, "app.py", "def f():\n    missing_thing()\n");
    build(&repo);

    let out = aw(&repo).args(["unresolved", "--names"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("aw: unresolved=1 rows"), "{stdout}");
    assert!(stdout.contains("app.py:2 call missing_thing"), "{stdout}");
}

#[test]
fn p8c_complexity_ranks_symbols_and_symbols_metrics_append() {
    let repo = temp_repo("complexity");
    write(
        &repo,
        "app.py",
        "def f(x):\n    if x:\n        return 1\n    elif x == 2:\n        return 2\n    return 0\n",
    );
    build(&repo);

    let out = aw(&repo)
        .args(["symbols", "app.py", "--metrics"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("cyc=3"), "{stdout}");
    assert!(stdout.contains("branches=2"), "{stdout}");

    let out = aw(&repo)
        .args(["complexity", "--min-cyc", "2"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.starts_with("aw: complexity —"), "{stdout}");
    assert!(stdout.contains("app.py:1 cyc=3"), "{stdout}");
}

#[test]
fn p8c_files_lists_indexed_inventory_and_generated_tag() {
    let repo = temp_repo("files-cmd");
    write(&repo, "src/a.py", "def a():\n    pass\n");
    write(
        &repo,
        "src/gen.pb.py",
        "# Code generated by tool. DO NOT EDIT.\ndef g():\n    pass\n",
    );
    build(&repo);

    let out = aw(&repo).args(["files", "--path", "src"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("src/a.py symbols="), "{stdout}");
    assert!(stdout.contains("generated=1"), "{stdout}");
}

#[test]
fn generated_tag_negative_controls_remain_unfiltered() {
    let repo = temp_repo("generated-negatives");
    write(
        &repo,
        "src/auto.py",
        "# @generated
def auto():
    pass
",
    );
    write(
        &repo,
        "src/gen/inside.py",
        "def handwritten():
    pass
",
    );
    write(
        &repo,
        "src/bare.ts",
        "// DO NOT EDIT
const bare = 1;
",
    );
    build(&repo);

    let out = aw(&repo).args(["files", "--path", "src"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let auto = stdout.lines().find(|l| l.contains("src/auto.py")).unwrap();
    assert!(auto.contains("generated=1"), "{stdout}");
    for row in ["src/gen/inside.py", "src/bare.ts"] {
        let line = stdout
            .lines()
            .find(|l| l.contains(row))
            .unwrap_or_else(|| panic!("missing {row}: {stdout}"));
        assert!(line.contains("generated=0"), "{line}");
    }
    let stats = aw(&repo).args(["stats"]).output().unwrap();
    assert!(
        String::from_utf8_lossy(&stats.stdout).contains("generated=1"),
        "{}",
        String::from_utf8_lossy(&stats.stdout)
    );
}

#[test]
fn resolve_batch_reports_found_and_missing_with_exit1() {
    let repo = temp_repo("resolve");
    write(&repo, "app.py", "def public():\n    pass\n");
    build(&repo);

    let mut child = aw(&repo)
        .args(["resolve", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"public\nmissing_thing\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    assert!(
        stdout.contains("found public app.py:1 app.public"),
        "{stdout}"
    );
    assert!(stdout.contains("missing missing_thing"), "{stdout}");
}

#[test]
fn resolve_honors_no_journal_and_records_parsed_count() {
    let repo = temp_repo("resolve-journal");
    write(&repo, "app.py", "def public():\n    pass\n");
    aw(&repo)
        .env("AW_NO_JOURNAL", "1")
        .args(["build"])
        .output()
        .unwrap();
    assert!(!repo.join(".aw/usage.log").exists(), "build opt-out");

    let mut child = aw(&repo)
        .args(["--no-journal", "resolve", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"public\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(
        !repo.join(".aw/usage.log").exists(),
        "--no-journal must suppress resolve journal"
    );

    let mut child = aw(&repo)
        .env_remove("AW_NO_JOURNAL")
        .args(["resolve", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"public\n# comment\n\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let log = fs::read_to_string(repo.join(".aw/usage.log")).unwrap();
    let line = log.lines().last().unwrap();
    assert!(line.contains("\"tool\":\"resolve\""), "{line}");
    assert!(line.contains("\"count\":\"1\""), "{line}");
    assert!(line.contains("\"candidate_limit\":\"50\""), "{line}");
}

#[test]
fn import_edges_dedup_and_resolved_imports_are_not_unresolved() {
    let repo = temp_repo("import-dedup");
    write(&repo, "pkg/__init__.py", "");
    write(&repo, "pkg/mod.py", "def b():\n    pass\n");
    write(&repo, "c.py", "from pkg.mod import b\n");
    build(&repo);

    let out = aw(&repo).args(["imports", "c.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert_eq!(stdout.matches("c.py:1 pkg.mod.b").count(), 1, "{stdout}");
    assert!(stdout.contains("c.py:1 pkg.mod"), "{stdout}");

    let out = aw(&repo).args(["unresolved", "--names"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn nested_function_calls_resolve_and_dead_keeps_inner_alive() {
    let repo = temp_repo("nested-call");
    write(
        &repo,
        "app.py",
        "def outer():\n    def inner():\n        pass\n    inner()\n",
    );
    build(&repo);

    let out = aw(&repo).args(["callers", "inner"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("app.py:4 app.outer"), "{stdout}");

    let out = aw(&repo).args(["callees", "outer"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("app.py:2 app.outer.inner"), "{stdout}");

    let out = aw(&repo).args(["unresolved", "--names"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );

    let out = aw(&repo).args(["dead"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.lines().any(|line| line == "app.py:2"), "{stdout}");
}

#[test]
fn inheritance_ts_path_alias_is_pinned_unresolved() {
    let repo = temp_repo("inherit-ts-alias");
    write(&repo, "src/lib/base.ts", "export class Base {}\n");
    write(
        &repo,
        "src/child.ts",
        "import { Base } from '@/lib/base';\nexport class Child extends Base {}\n",
    );
    build(&repo);

    let out = aw(&repo).args(["implementers", "Base"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    assert!(
        stderr.contains("1 unresolved inherit sites may refer to this symbol"),
        "{stderr}"
    );

    let out = aw(&repo).args(["unresolved", "--names"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("src/child.ts:2 inherit Base"), "{stdout}");
}

#[test]
fn inheritance_ignores_generic_arguments_and_external_paths() {
    let repo = temp_repo("inherit-false-edges");
    write(
        &repo,
        "a.ts",
        "class T {}\nclass Base<X> {}\nclass C extends Base<T> {}\n",
    );
    write(
        &repo,
        "lib.rs",
        "trait AppError: std::error::Error {}\nstruct std;\nstruct error;\nstruct Error;\n",
    );
    build(&repo);

    for name in ["T", "std", "error", "Error"] {
        let out = aw(&repo).args(["implementers", name]).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(1),
            "{name}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

#[test]
fn tests_ignores_module_symbols_named_like_tests() {
    let repo = temp_repo("tests-kind");
    write(&repo, "src/core.py", "def f():\n    pass\n");
    write(
        &repo,
        "pkg/test_service.py",
        "from src.core import f\nf()\n",
    );
    build(&repo);

    let out = aw(&repo).args(["tests", "f"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn unresolved_reports_counts_and_zero_is_exit1() {
    let repo = temp_repo("unresolved");
    write(&repo, "m.py", "def go():\n    return ghost()\n");
    build(&repo);
    let out = aw(&repo).args(["unresolved"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("aw: unresolved=1 callees in 1 files"),
        "{stdout}"
    );
    assert!(stdout.contains("m.py unresolved=1"), "{stdout}");

    let clean = temp_repo("unresolved-clean");
    write(
        &clean,
        "a.py",
        "def g():\n    return 1\n\ndef go():\n    return g()\n",
    );
    build(&clean);
    let out = aw(&clean).args(["unresolved"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("0 unresolved in indexed scope"), "{stderr}");
}

#[test]
fn symbols_lists_file_symbols_and_unknown_exit1() {
    let repo = fixture("symbols");
    build(&repo);
    let out = aw(&repo).args(["symbols", "utils.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout.contains("utils.py:1 module utils"), "{stdout}");
    assert!(
        stdout.contains("utils.py:1 function utils.normalize"),
        "{stdout}"
    );
    assert!(
        stdout.contains("utils.py:4 function utils.caller"),
        "{stdout}"
    );

    let out = aw(&repo).args(["symbols", "nope.py"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not indexed"), "{stderr}");
}

#[test]
fn symbols_outline_adds_signature_and_default_stays_byte_identical() {
    let repo = fixture("symbols-outline");
    build(&repo);
    let base = aw(&repo).args(["symbols", "utils.py"]).output().unwrap();
    assert_eq!(base.status.code(), Some(0));
    let plain = String::from_utf8_lossy(&base.stdout).into_owned();
    let out = aw(&repo)
        .args(["symbols", "utils.py", "--outline"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let outlined = String::from_utf8_lossy(&out.stdout).into_owned();

    // Amendment 12a: identical line count; each outline line is the plain
    // line plus a one-line signature (<= 160 chars, never a body).
    let plain_lines: Vec<&str> = plain.lines().collect();
    let outlined_lines: Vec<&str> = outlined.lines().collect();
    assert_eq!(plain_lines.len(), outlined_lines.len(), "{outlined}");
    for (p, o) in plain_lines.iter().zip(outlined_lines.iter()) {
        // Empty-signature rows (module symbols) stay byte-identical.
        let sig = match o.strip_prefix(format!("{p} ").as_str()) {
            Some(s) => s,
            None => {
                assert_eq!(p, o, "outline line '{o}' diverges from plain '{p}'");
                continue;
            }
        };
        assert!(sig.chars().count() <= 160, "signature over cap: {sig}");
        assert!(
            !sig.contains("return"),
            "signature must not carry bodies: {sig}"
        );
    }
    // At least one def line actually gained a signature.
    assert!(
        outlined_lines
            .iter()
            .zip(plain_lines.iter())
            .any(|(o, p)| o.len() > p.len() + 1),
        "no line gained a signature: {outlined}"
    );
}

#[test]
fn search_finds_symbols_ranked_and_empty_or_nomatch_exit_codes() {
    let repo = fixture("search");
    build(&repo);
    // Hit: bm25-ranked pointers, tier position empty (symbol identities).
    let _ = fs::remove_file(repo.join(".aw/usage.log"));
    let out = aw(&repo).args(["search", "normalize"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    // Every search exit path journals, including the exact-name fast path.
    let log = fs::read_to_string(repo.join(".aw/usage.log")).unwrap();
    let rec = log.lines().last().unwrap();
    assert!(
        rec.contains("\"tool\":\"search\"") && !rec.contains("\"result_count\":0"),
        "exact-name search must not journal an empty call: {rec}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("utils.py:1"), "{stdout}");
    assert!(
        !stdout.contains("[ambiguous]"),
        "search hits carry no tier tag: {stdout}"
    );
    // Amendment 14b: the hit line carries the stored signature.
    assert!(
        stdout.contains("def normalize(text)"),
        "search hit carries the signature: {stdout}"
    );
    // Amendment 14a: signature-only vocabulary is searchable (no symbol
    // name contains the `def` token; every signature does).
    let out = aw(&repo).args(["search", "def"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("utils.py:1"),
        "signature tokens reach symbols: {stdout}"
    );

    // Morphology fallback (R7): plural asks match the singular stem.
    let out = aw(&repo).args(["search", "normalizes"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("utils.py:1"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );

    // No match: exit 1 with the documented reason line.
    let out = aw(&repo).args(["search", "zzzqqqxyz"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("0 symbols matched"), "{stderr}");

    // Empty terms: usage error, exit 2.
    let out = aw(&repo).args(["search", "   "]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn clones_seed_gate() {
    // G16 (Amendment 12d): 10 seeded clone pairs + distractors. Pass:
    // >= 8/10 pairs detected, <= 20% spurious pair lines, caveat first,
    // min-lines filter, carry-forward survival.
    let repo = fixture("clones-g16");
    // Per-pair unique identifiers: no line may be shared across pairs, or the
    // 5-line text-equality windows cross-match (true per the semantics, but a
    // fixture artifact).
    let body = |name: &str, n: usize| {
        let mut s = format!("def process_{name}(items):\n");
        s.push_str(&format!("    acc_{name} = 0\n"));
        for k in 0..n.saturating_sub(3) {
            s.push_str(&format!("    acc_{name} += scale_{name}(items[{k}])\n"));
        }
        s.push_str(&format!("    return finish_{name}(acc_{name})\n"));
        s
    };
    let mut expected_pairs: Vec<(String, String)> = Vec::new();
    for i in 0..10 {
        let n = 6 + i * 3; // varied spans, all >= window
        let a = format!("pair{:02}_a.py", i);
        let b = format!("pair{:02}_b.py", i);
        write(&repo, &a, &body(&format!("a{i}"), n));
        // Same content with the same identifiers: text-equality clone.
        write(&repo, &b, &body(&format!("a{i}"), n));
        expected_pairs.push((a, b));
    }
    // Distractors: unique files (no duplication) and a near-duplicate whose
    // identifiers differ (normalized-text inequality -> no window matches).
    for u in 0..8 {
        write(
            &repo,
            &format!("unique{u}.py"),
            &body(&format!("u{u}"), 6 + u),
        );
    }
    write(&repo, "near.py", &body("near_variant", 9));
    build(&repo);

    let out = aw(&repo)
        .args(["clones", "--limit", "10000"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let first = stdout.lines().next().unwrap().to_string();
    assert_eq!(
        first, "aw: clone candidates — normalized-text equality, not proof of semantic duplication",
        "{stdout}"
    );

    let pair_lines: Vec<&str> = stdout
        .lines()
        .filter(|l| l.contains(":") && !l.starts_with("aw:") && !l.starts_with("showing "))
        .collect();
    let mut detected = 0usize;
    for (a, b) in &expected_pairs {
        if pair_lines
            .iter()
            .any(|l| l.contains(a.as_str()) && l.contains(b.as_str()))
        {
            detected += 1;
        }
    }
    assert!(
        detected >= 8,
        "seeded pairs detected {detected}/10 (bar >= 8): {stdout}"
    );
    let spurious = pair_lines.len() - detected;
    assert!(
        spurious * 5 <= pair_lines.len(),
        "spurious {spurious}/{} exceeds 20%: {stdout}",
        pair_lines.len()
    );

    // min-lines above every seeded span: gate to zero, exit 1.
    let out = aw(&repo)
        .args(["clones", "--min-lines", "100"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));

    // Carry-forward: no-change rebuild keeps the candidates (re-count from
    // the second run's own stdout — detection must hold independently).
    build(&repo);
    let out = aw(&repo)
        .args(["clones", "--limit", "10000"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout2 = String::from_utf8_lossy(&out.stdout);
    let pair_lines2: Vec<&str> = stdout2
        .lines()
        .filter(|l| l.contains(":") && !l.starts_with("aw:") && !l.starts_with("showing "))
        .collect();
    let detected2 = expected_pairs
        .iter()
        .filter(|(a, b)| {
            pair_lines2
                .iter()
                .any(|l| l.contains(a.as_str()) && l.contains(b.as_str()))
        })
        .count();
    assert!(
        detected2 >= 8,
        "post-rebuild detection {detected2}/10: {stdout2}"
    );
}

#[test]
fn journal_written_and_usage_summarizes() {
    // Amendment 6: build + callers journal entries; aw usage aggregates them.
    let repo = fixture("journal");
    aw(&repo)
        .env_remove("AW_NO_JOURNAL")
        .args(["build"])
        .output()
        .unwrap();
    aw(&repo)
        .env_remove("AW_NO_JOURNAL")
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    let log = fs::read_to_string(repo.join(".aw/usage.log")).unwrap();
    assert_eq!(log.lines().count(), 2, "one entry per invocation: {log}");
    assert!(log.contains("\"tool\":\"build\""), "{log}");
    assert!(log.contains("\"tool\":\"callers\""), "{log}");
    assert!(log.contains("\"surface\":\"cli\""), "{log}");

    let out = aw(&repo)
        .env_remove("AW_NO_JOURNAL")
        .args(["usage"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout.contains("aw usage: 2 journaled calls"), "{stdout}");
    assert!(stdout.contains("callers: 1 calls"), "{stdout}");
    assert!(stdout.contains("build: 1 calls"), "{stdout}");
}

#[test]
fn journal_opt_out_via_env_and_flag() {
    let repo = fixture("journal-env");
    aw(&repo)
        .env("AW_NO_JOURNAL", "1")
        .args(["build"])
        .output()
        .unwrap();
    assert!(
        !repo.join(".aw/usage.log").exists(),
        "env opt-out must suppress the journal"
    );

    let repo = fixture("journal-flag");
    aw(&repo).args(["--no-journal", "build"]).output().unwrap();
    assert!(
        !repo.join(".aw/usage.log").exists(),
        "--no-journal must suppress the journal"
    );

    // usage on an empty journal: exit 1 with a reason.
    let out = aw(&repo).args(["usage"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no journal entries"), "{stderr}");
}

/// Parse the `pointers` array of one hand-serialized payload line (Amendment 10).
fn payload_pointers(line: &str) -> Vec<String> {
    let marker = "\"pointers\":[";
    let start = line.find(marker).expect("payload line carries pointers") + marker.len();
    let end = start + line[start..].find("]}").expect("pointers array terminator");
    let inner = &line[start..end];
    if inner.is_empty() {
        return Vec::new();
    }
    inner
        .split("\",\"")
        .map(|s| s.trim_matches('"').to_string())
        .collect()
}

#[test]
fn payload_log_written_for_pointer_tools_only() {
    // Amendment 10a: callers/dead/symbols append self-contained payload lines;
    // build/stats write none; pointers are byte-identical to stdout data lines.
    let repo = fixture("payload");
    build(&repo);
    assert!(
        !repo.join(".aw/usage-results.jsonl").exists(),
        "build writes no payload line"
    );

    let out = aw(&repo)
        .env_remove("AW_NO_JOURNAL")
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    let data_lines: Vec<String> = stdout
        .lines()
        .filter(|l| !l.starts_with("aw: "))
        .map(String::from)
        .collect();
    assert!(!data_lines.is_empty());

    let payload = fs::read_to_string(repo.join(".aw/usage-results.jsonl")).unwrap();
    let mut lines = payload.lines();
    let line = lines.next().unwrap();
    assert!(line.contains("\"tool\":\"callers\""), "{line}");
    assert!(line.contains("\"name\":\"normalize\""), "{line}");
    assert!(line.contains("\"surface\":\"cli\""), "{line}");
    assert!(line.contains("\"stale\":false"), "{line}");
    assert!(line.contains("\"built_at\":"), "{line}");
    assert!(line.contains("\"source_rev\":\""), "{line}");
    assert_eq!(payload_pointers(line), data_lines, "byte-equal pointers");

    aw(&repo)
        .env_remove("AW_NO_JOURNAL")
        .args(["dead"])
        .output()
        .unwrap();
    aw(&repo)
        .env_remove("AW_NO_JOURNAL")
        .args(["symbols", "utils.py"])
        .output()
        .unwrap();
    let payload = fs::read_to_string(repo.join(".aw/usage-results.jsonl")).unwrap();
    assert_eq!(
        payload.lines().count(),
        3,
        "one payload line per pointer call"
    );
    assert!(payload.contains("\"tool\":\"dead\""), "{payload}");
    assert!(payload.contains("\"tool\":\"symbols\""), "{payload}");
    assert!(
        payload.contains("\"pointers\":[\"utils.py:1 module utils"),
        "symbols data lines, no header/cap: {payload}"
    );

    // Non-pointer tools journal to usage.log but add no payload line.
    let before = payload.lines().count();
    aw(&repo)
        .env_remove("AW_NO_JOURNAL")
        .args(["stats"])
        .output()
        .unwrap();
    let after = fs::read_to_string(repo.join(".aw/usage-results.jsonl"))
        .unwrap()
        .lines()
        .count();
    assert_eq!(before, after, "stats writes no payload line");
}

#[test]
fn session_and_arm_tags_recorded_in_both_logs() {
    // Amendment 10b: AW_SESSION/AW_ARM land in usage.log AND payload lines.
    let repo = fixture("payload-tags");
    build(&repo);
    aw(&repo)
        .env_remove("AW_NO_JOURNAL")
        .env("AW_SESSION", "  r1-task-1  ")
        .env("AW_ARM", "aw")
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    let usage = fs::read_to_string(repo.join(".aw/usage.log")).unwrap();
    assert!(usage.contains("\"session\":\"r1-task-1\""), "{usage}");
    assert!(usage.contains("\"arm\":\"aw\""), "{usage}");
    let payload = fs::read_to_string(repo.join(".aw/usage-results.jsonl")).unwrap();
    assert!(payload.contains("\"session\":\"r1-task-1\""), "{payload}");
    assert!(payload.contains("\"arm\":\"aw\""), "{payload}");
}

#[test]
fn payload_opt_out_and_empty_or_ambiguous_writes_none() {
    // Opt-out suppresses both journal files.
    let repo = fixture("payload-optout");
    aw(&repo)
        .env("AW_NO_JOURNAL", "1")
        .args(["build"])
        .output()
        .unwrap();
    aw(&repo)
        .env("AW_NO_JOURNAL", "1")
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    assert!(!repo.join(".aw/usage.log").exists());
    assert!(!repo.join(".aw/usage-results.jsonl").exists());

    // Empty result: journaled with exit 1, no payload line.
    let repo = fixture("payload-empty");
    build(&repo);
    let out = aw(&repo)
        .env_remove("AW_NO_JOURNAL")
        .args(["callers", "ghost"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        !repo.join(".aw/usage-results.jsonl").exists(),
        "empty result writes no payload"
    );

    // Ambiguous name: exit 2 with candidates on stdout, no payload line.
    let repo = temp_repo("payload-ambig");
    write(&repo, "a.py", "def dup():\n    return 1\n");
    write(&repo, "b.py", "def dup():\n    return 2\n");
    build(&repo);
    let out = aw(&repo).args(["callers", "dup"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        !repo.join(".aw/usage-results.jsonl").exists(),
        "ambiguous result writes no payload"
    );
}

#[test]
fn usage_verify_reports_ok_changed_fail() {
    // Amendment 10c: fresh queries verify OK; an edit-after-build turns that
    // file's pointers CHANGED (unverifiable, not wrong); a corrupt pointer
    // (line beyond bounds) FAILs; the report is informational (exit 0).
    let repo = fixture("verify");
    build(&repo);
    aw(&repo)
        .env_remove("AW_NO_JOURNAL")
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    aw(&repo)
        .env_remove("AW_NO_JOURNAL")
        .args(["dead"])
        .output()
        .unwrap();

    let out = aw(&repo).args(["usage", "--verify"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("aw usage --verify: 5 pointers sampled from 2 entries"),
        "{stdout}"
    );
    assert!(
        stdout.contains("changed=0 fail=0"),
        "all OK on a fresh build: {stdout}"
    );

    // Edit app.py (dead pointers include app.py:4; callers include app.py:6):
    // the index still holds the old hash -> those pointers are CHANGED.
    write(
        &repo,
        "app.py",
        "from utils import normalize\n\ndef run():\n    return normalize('z')\n",
    );
    let out = aw(&repo).args(["usage", "--verify"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("changed=2"),
        "app.py pointers unverifiable: {stdout}"
    );
    assert!(stdout.contains("fail=0"), "changed is not fail: {stdout}");

    // Corrupt one pointer beyond bounds -> FAIL, still exit 0.
    let ppath = repo.join(".aw/usage-results.jsonl");
    let payload = fs::read_to_string(&ppath)
        .unwrap()
        .replace("utils.py:5", "utils.py:9999");
    fs::write(&ppath, payload).unwrap();
    let out = aw(&repo).args(["usage", "--verify"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "report-only");
    assert!(stdout.contains("fail=1"), "{stdout}");
    assert!(
        stdout.contains("FAIL utils.py:9999") && stdout.contains("(line beyond"),
        "{stdout}"
    );
}

#[test]
fn usage_verify_stride_sample_and_empty_payload() {
    // >limit entries sample exactly `limit` entries, deterministically.
    let repo = fixture("verify-stride");
    build(&repo);
    for _ in 0..60 {
        aw(&repo)
            .env_remove("AW_NO_JOURNAL")
            .args(["callers", "normalize"])
            .output()
            .unwrap();
    }
    let out = aw(&repo)
        .args(["usage", "--verify", "--limit", "10"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    // 10 entries x 2 pointers each.
    assert!(
        stdout.contains("20 pointers sampled from 60 entries"),
        "{stdout}"
    );

    // No payload at all: exit 1 with a reason.
    let repo = fixture("verify-empty");
    build(&repo);
    let out = aw(&repo).args(["usage", "--verify"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no payload entries"), "{stderr}");

    // --limit 0 is a usage error (exit 2).
    let repo = fixture("verify-limit0");
    build(&repo);
    aw(&repo)
        .env_remove("AW_NO_JOURNAL")
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    let out = aw(&repo)
        .args(["usage", "--verify", "--limit", "0"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn usage_verify_epoch_rebuild_is_changed_not_fail() {
    // A payload entry recorded against a superseded index (edit + rebuild
    // between query and verify) cannot prove a pointer wrong: the untouched
    // file's pointer still verifies OK, the edited file's is CHANGED, and no
    // pointer becomes FAIL.
    let repo = fixture("verify-epoch");
    build(&repo);
    aw(&repo)
        .env_remove("AW_NO_JOURNAL")
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    write(
        &repo,
        "app.py",
        "from utils import normalize\n\ndef run():\n    return normalize('z')\n",
    );
    build(&repo);
    let out = aw(&repo).args(["usage", "--verify"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("ok=1 changed=1 fail=0"),
        "old-epoch pointers: {stdout}"
    );
    assert!(
        !stdout.contains("FAIL"),
        "no structural failures across epochs: {stdout}"
    );
}

#[test]
fn usage_summary_payload_section_and_doctor_health() {
    // Amendment 10: usage summary carries the demand map over the payload log;
    // doctor reports payload-journal volume (or its absence).
    let repo = fixture("usage-payload");
    build(&repo);
    aw(&repo)
        .env_remove("AW_NO_JOURNAL")
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    let out = aw(&repo).args(["usage"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("payload: 1 entries, 2 pointers"),
        "{stdout}"
    );
    assert!(stdout.contains("normalize: 1 calls"), "{stdout}");

    let out = aw(&repo).args(["doctor"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout.contains("payload journal: 1 lines, "), "{stdout}");

    // No pointer queries yet -> absent line, and no payload section in usage.
    let repo = fixture("usage-payload-absent");
    build(&repo);
    let out = aw(&repo).args(["usage"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("payload:"),
        "no section without payload: {stdout}"
    );
    let out = aw(&repo).args(["doctor"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("payload journal: absent"),
        "doctor states absence: {stdout}"
    );
}

#[test]
fn g5_no_child_processes_survive() {
    // Invariant 2: aw spawns only short-lived git commands and leaves no
    // descendants. Poll the child tree during the build (the previous test
    // only looked after exit, R6e finding), then assert nothing tagged with
    // the fixture token survives.
    let repo = fixture("proc");
    let marker = format!("aw-proc-{}", std::process::id());
    write(&repo, &format!("{marker}.py"), "def f():\n    pass\n");
    for i in 0..200 {
        write(&repo, &format!("pad{i}.py"), "def p():\n    return 1\n");
    }
    let mut child = aw(&repo).arg("build").spawn().unwrap();
    let pid = child.id().to_string();
    let mut seen: std::collections::HashSet<String> = Default::default();
    loop {
        if let Ok(out) = Command::new("ps")
            .args(["-o", "comm=", "-P", &pid])
            .output()
        {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let comm = line.trim();
                if !comm.is_empty() {
                    seen.insert(comm.to_string());
                }
            }
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let status = child.wait().unwrap();
    assert!(status.success());
    for comm in &seen {
        assert!(
            comm == "git" || comm.ends_with("/git"),
            "unexpected child process during build: {comm} (seen: {seen:?})"
        );
    }
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
    let out = aw(&repo).args(["stats"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("files=3") && stdout.contains("symbols=") && stdout.contains("edges="),
        "{stdout}"
    );

    let out = aw(&repo).args(["hotspots"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("loc=") && stdout.contains("cyc="),
        "{stdout}"
    );

    // R7 L4: report is a build snapshot; an edit must surface a first-line
    // freshness caveat before the sections.
    write(
        &repo,
        "utils.py",
        &format!("{UTILS}\n\ndef extra():\n    return 1\n"),
    );
    let out = aw(&repo).args(["report"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("aw: 1 of 3 sampled files changed since build"),
        "freshness header: {stdout}"
    );
    for section in [
        "## 1. index overview",
        "## 2. architecture map",
        "## 3. hotspots",
        "## 4. most-called symbols",
    ] {
        assert!(stdout.contains(section), "missing {section}: {stdout}");
    }
}

#[test]
fn partial_complexity_index_rebuilds_fresh() {
    // Amendment 19/20: an index missing a required index must disable the
    // fast path and rebuild, not fail carry-forward.
    let repo = fixture("partial-index");
    build(&repo);
    let idx = repo.join(".aw/index.db");
    let status = std::process::Command::new("sqlite3")
        .args([
            idx.to_string_lossy().as_ref(),
            "DROP INDEX symbols_complexity_idx;",
        ])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "sqlite3 demotion failed (fixture-only tool)"
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(
        code, 0,
        "rebuild over partial index failed: {stdout}{stderr}"
    );
}

#[test]
fn partial_v7_file_path_column_rebuilds_fresh() {
    // Amendment 20: a v7-stamped index missing symbols.file_path must disable
    // the fast path. Recreate the v6 table shape behind a v7 stamp.
    let repo = fixture("partial-file-path");
    build(&repo);
    let idx = repo.join(".aw/index.db");
    let status = std::process::Command::new("sqlite3")
        .args([
            idx.to_string_lossy().as_ref(),
            "DROP INDEX symbols_complexity_idx;
             ALTER TABLE symbols DROP COLUMN file_path;
             CREATE INDEX symbols_complexity_idx
               ON symbols(cyclomatic DESC, effective_loc DESC, qualname, file_id, line_start);",
        ])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "sqlite3 demotion failed (fixture-only tool)"
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(
        code, 0,
        "rebuild over missing file_path failed: {stdout}{stderr}"
    );
    let out = aw(&repo).args(["complexity"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("app.py"),
        "fresh index lost symbols after v7 rebuild"
    );
}

#[test]
fn v6_database_rebuilds_to_v7() {
    // Amendment 20: a v6 user_version disables the fast path; build rewrites
    // the index at v7 instead of carrying v6 rows.
    let repo = fixture("v6-to-v7");
    build(&repo);
    let idx = repo.join(".aw/index.db");
    let status = std::process::Command::new("sqlite3")
        .args([idx.to_string_lossy().as_ref(), "PRAGMA user_version = 6;"])
        .output()
        .unwrap();
    assert!(status.status.success(), "sqlite3 version demotion failed");
    let out = aw(&repo).args(["stats"]).output().unwrap();
    assert_eq!(out.status.code(), Some(5), "v6 query open must exit 5");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("schema v6, binary expects v7 — run aw build"),
        "{stderr}"
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "v6 rebuild failed: {stdout}{stderr}");
    let out = std::process::Command::new("sqlite3")
        .args([idx.to_string_lossy().as_ref(), "PRAGMA user_version;"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "7",
        "build did not stamp v7"
    );
}

#[test]
fn p8c_complexity_path_tiebreak_is_stable_fresh_and_carried() {
    // Amendment 20: pkg.f exists in pkg.py and pkg/__init__.py with equal
    // metrics. Ordering must use file_path, so a carried index whose file_id
    // order differs from a fresh one still prints byte-identical output.
    let repo = temp_repo("complexity-path");
    let body = "def f(x):\n    if x:\n        return 1\n    return 0\n";
    write(&repo, "pkg/__init__.py", body);
    build(&repo);
    write(&repo, "pkg.py", body);
    build(&repo);

    let carried = aw(&repo)
        .args(["complexity", "--limit", "10"])
        .output()
        .unwrap();
    let carried_out = String::from_utf8_lossy(&carried.stdout).into_owned();
    assert_eq!(carried.status.code(), Some(0), "{carried_out}");
    let py_at = carried_out.find("pkg.py:1").expect("pkg.py row");
    let init_at = carried_out
        .find("pkg/__init__.py:1")
        .expect("pkg/__init__.py row");
    assert!(
        py_at < init_at,
        "carried order must be path-ordered: {carried_out}"
    );

    fs::remove_dir_all(repo.join(".aw")).unwrap();
    build(&repo);
    let fresh = aw(&repo)
        .args(["complexity", "--limit", "10"])
        .output()
        .unwrap();
    let fresh_out = String::from_utf8_lossy(&fresh.stdout).into_owned();
    assert_eq!(fresh.status.code(), Some(0), "{fresh_out}");
    assert_eq!(
        fresh_out, carried_out,
        "fresh and carried complexity outputs differ"
    );
}

#[test]
fn partial_v4_index_rebuilds_fresh() {
    // Amendment 12 regression: a v4-stamped index from the first landing
    // wave (symbols_fts present, clone_groups absent) must disable the fast
    // path and rebuild fully, not fail carry-forward on the missing table.
    let repo = fixture("partial-v4");
    build(&repo);
    // Demote the index to a first-wave v4: drop clone_groups in place.
    let idx = repo.join(".aw/index.db");
    let status = std::process::Command::new("sqlite3")
        .args([idx.to_string_lossy().as_ref(), "DROP TABLE clone_groups;"])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "sqlite3 demotion failed (fixture-only tool)"
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(
        code, 0,
        "rebuild over partial v4 index failed: {stdout}{stderr}"
    );
    assert!(!stderr.contains("clone_groups"), "{stdout}{stderr}");
}

// ---------------------------------------------------------------------------
// G17 (Amendment 13): `aw path` seeded fixtures — 10 scenarios, each asserting
// exact stdout/stderr bytes against a hand-pinned fixture graph.

/// Fixture repo for the path gate. Every file is pinned line-by-line; the
/// expected call sites below are derived from these exact byte layouts.
fn path_fixture(name: &str) -> PathBuf {
    let repo = temp_repo(name);
    write(
        &repo,
        "chain.py",
        "def ca():\n    return cb()\n\ndef cb():\n    return cc()\n\ndef cc():\n    return cd()\n\ndef cd():\n    return 1\n",
    );
    write(
        &repo,
        "diamond.py",
        "def dtop():\n    x = dm1()\n    return dm2()\n\ndef dm1():\n    return dbot()\n\ndef dm2():\n    return dbot()\n\ndef dbot():\n    return 0\n",
    );
    write(
        &repo,
        "cycle.py",
        "def cya():\n    return cyb()\n\ndef cyb():\n    return cyc()\n\ndef cyc():\n    return cya()\n",
    );
    write(
        &repo,
        "disconnected.py",
        "def iso1():\n    return 1\n\ndef iso2():\n    return 2\n",
    );
    let mut deep = String::new();
    for i in 1..=12 {
        deep.push_str(&format!("def f{i:02}():\n"));
        if i < 12 {
            deep.push_str(&format!("    return f{i:02}()\n", i = i + 1));
        } else {
            deep.push_str("    return 0\n");
        }
    }
    write(&repo, "deep.py", &deep);
    write(
        &repo,
        "multi.py",
        "def multi_caller():\n    x = mtarget()\n    y = mtarget()\n    return x + y\n\ndef mtarget():\n    return 1\n",
    );
    write(
        &repo,
        "amb.py",
        "class Apage:\n    def dup(self):\n        return target()\n\nclass Bpage:\n    def dup(self):\n        return 2\n\ndef target():\n    return 1\n",
    );
    build(&repo);
    repo
}

#[test]
fn path_seed_gate() {
    let repo = path_fixture("path-g17");

    // (1) 3-hop chain: hop call-site pointers in path order + resolved
    // qualname chain line.
    let out = aw(&repo).args(["path", "ca", "cd"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "aw: path — resolved static edges only; dynamic dispatch may hide paths\n\
         chain.py:2\nchain.py:5\nchain.py:8\n\
         aw: chain chain.ca -> chain.cb -> chain.cc -> chain.cd (3 hops)\n",
        "scenario 1: {stdout}"
    );

    // (2) Diamond tie-break: dtop reaches dbot through BOTH dm1 and dm2 at
    // 2 hops; dm1 expands before dm2 (qualname order), so the pinned
    // first-discoverer parent rule yields dtop -> dm1 -> dbot.
    let out = aw(&repo).args(["path", "dtop", "dbot"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "aw: path — resolved static edges only; dynamic dispatch may hide paths\n\
         diamond.py:2\ndiamond.py:6\n\
         aw: chain diamond.dtop -> diamond.dm1 -> diamond.dbot (2 hops)\n",
        "scenario 2: {stdout}"
    );

    // (3) Cycle: cya reaches cyc in 2 hops and terminates.
    let out = aw(&repo).args(["path", "cya", "cyc"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "aw: path — resolved static edges only; dynamic dispatch may hide paths\n\
         cycle.py:2\ncycle.py:5\n\
         aw: chain cycle.cya -> cycle.cyb -> cycle.cyc (2 hops)\n",
        "scenario 3: {stdout}"
    );

    // (4) Disconnected pair: definitive exhaustive negative.
    let out = aw(&repo).args(["path", "iso1", "iso2"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(stdout, "", "scenario 4 stdout: {stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        stderr,
        "aw: no-results: no call path from 'disconnected.iso1' to 'disconnected.iso2' at any depth <= 10 (resolved edges only)\n",
        "scenario 4 stderr: {stderr}"
    );

    // (5) Depth-cap miss: the real path is 11 hops, beyond the 1-10 clamp,
    // so any --depth clamps to 10 and yields the cap-truncated negative.
    // Boundary: f01 -> f11 is exactly 10 hops and is found at the default;
    // --depth 11 behaves byte-identically (clamped).
    let out = aw(&repo).args(["path", "f01", "f12"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        stderr,
        "aw: no-results: no call path from 'deep.f01' to 'deep.f12' within depth 10; a longer path may exist (resolved edges only)\n",
        "scenario 5 truncated stderr: {stderr}"
    );
    let mut expected =
        String::from("aw: path — resolved static edges only; dynamic dispatch may hide paths\n");
    for i in 1..=10 {
        expected.push_str(&format!("deep.py:{}\n", 2 * i));
    }
    expected.push_str("aw: chain deep.f01 -> deep.f02 -> deep.f03 -> deep.f04 -> deep.f05 -> deep.f06 -> deep.f07 -> deep.f08 -> deep.f09 -> deep.f10 -> deep.f11 (10 hops)\n");
    let out = aw(&repo).args(["path", "f01", "f11"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(stdout, expected, "scenario 5 boundary: {stdout}");
    let out = aw(&repo)
        .args(["path", "f01", "f11", "--depth", "11"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(stdout, expected, "scenario 5 clamp: {stdout}");

    // (5b) Min-clamp boundary: --depth 0 clamps to 1; ca -> cc needs 2 hops,
    // so the frontier stops with unvisited out-neighbors (cap-truncated).
    let out = aw(&repo)
        .args(["path", "ca", "cc", "--depth", "0"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        stderr,
        "aw: no-results: no call path from 'chain.ca' to 'chain.cc' within depth 1; a longer path may exist (resolved edges only)\n",
        "scenario 5b min-clamp stderr: {stderr}"
    );

    // (6) from == to: same-symbol negative (exit 1); an ambiguous shared
    // name resolves FIRST and exits 2 (pinned order: ambiguity precedes the
    // same-symbol check).
    let out = aw(&repo).args(["path", "ca", "ca"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        stderr, "aw: no-results: 'chain.ca' and 'chain.ca' resolve to the same symbol\n",
        "scenario 6 stderr: {stderr}"
    );
    let out = aw(&repo).args(["path", "dup", "dup"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "ambiguous shared name exits 2, not 1"
    );

    // (7) Ambiguous `from` endpoint: exit 2, role in the detail, candidates
    // on stdout; the Class.method form resolves.
    let out = aw(&repo).args(["path", "dup", "ca"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout, "amb.py:2 amb.Apage.dup\namb.py:6 amb.Bpage.dup\n",
        "scenario 7 stdout: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        stderr,
        "aw: usage: ambiguous name 'dup' — 2 candidates as the from endpoint; use the Class.method qualname form\n",
        "scenario 7 stderr: {stderr}"
    );
    let out = aw(&repo)
        .args(["path", "amb.Apage.dup", "target"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "aw: path — resolved static edges only; dynamic dispatch may hide paths\n\
         amb.py:3\n\
         aw: chain amb.Apage.dup -> amb.target (1 hops)\n",
        "scenario 7 resolved form: {stdout}"
    );

    // (8) Ambiguous `to` endpoint: exit 2 with the `to` role.
    let out = aw(&repo).args(["path", "ca", "dup"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout, "amb.py:2 amb.Apage.dup\namb.py:6 amb.Bpage.dup\n",
        "scenario 8 stdout: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        stderr,
        "aw: usage: ambiguous name 'dup' — 2 candidates as the to endpoint; use the Class.method qualname form\n",
        "scenario 8 stderr: {stderr}"
    );

    // (9) Multi-edge representative: two call sites of mtarget; the pointer
    // is the smallest site_line (line 2).
    let out = aw(&repo)
        .args(["path", "multi_caller", "mtarget"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "aw: path — resolved static edges only; dynamic dispatch may hide paths\n\
         multi.py:2\n\
         aw: chain multi.multi_caller -> multi.mtarget (1 hops)\n",
        "scenario 9: {stdout}"
    );

    // (10) Stale-index header: change a displayed file post-build; the
    // staleness line follows the caveat on the displayed set.
    write(
        &repo,
        "chain.py",
        "def ca():\n    return cb()\n\ndef cb():\n    return cc()\n\ndef cc():\n    return cd()\n\ndef cd():\n    return 1\n# touched\n",
    );
    let out = aw(&repo).args(["path", "ca", "cd"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "aw: path — resolved static edges only; dynamic dispatch may hide paths\n\
         aw: 1 of 1 result files changed since build — run aw build\n\
         chain.py:2\nchain.py:5\nchain.py:8\n\
         aw: chain chain.ca -> chain.cb -> chain.cc -> chain.cd (3 hops)\n",
        "scenario 10: {stdout}"
    );
}

#[test]
fn deep_nesting_does_not_abort_build() {
    // R6d Blocker 1: source nesting is attacker-controlled. 2,000 nested
    // parens used to abort the build (SIGABRT). The build must exit 0 and
    // index the file (the extractor guards above MAX_AST_DEPTH).
    let repo = temp_repo("deep-nesting");
    let py = format!("x = {}1{}\n", "(".repeat(2000), ")".repeat(2000));
    let ts = format!("const v = {}1{};\n", "[".repeat(2000), "]".repeat(2000));
    let rs = format!(
        "fn f() {{ let x = {}1{}; }}\n",
        "(".repeat(2000),
        ")".repeat(2000)
    );
    write(&repo, "deep.py", &py);
    write(&repo, "deep.ts", &ts);
    write(&repo, "deep.rs", &rs);
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "deep nesting must not abort: {stdout}{stderr}");
    assert!(stdout.contains("files=3"), "{stdout}");
    let out = aw(&repo).args(["stats"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn clones_pair_generation_is_bounded() {
    // R6d Blocker 2: a shared header puts every file in one shingle group;
    // the full pair set is quadratic. The caps must engage and report a
    // lower bound instead of materializing millions of pairs.
    let repo = temp_repo("clones-cap");
    let header = "# shared license header line one\n# shared license header line two\n# shared license header line three\n# shared license header line four\n# shared license header line five\n";
    for i in 0..120 {
        write(
            &repo,
            &format!("f{i:03}.py"),
            &format!("{header}def solo_{i}(x):\n    return x + {i}\n"),
        );
    }
    build(&repo);
    let out = aw(&repo).args(["clones", "--limit", "5"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("lower bound"),
        "cap note expected when regions exceed MAX_REGIONS_PER_GROUP: {stderr}"
    );
}

#[test]
fn incremental_build_heals_resolution_gaps() {
    // R6d A4 / Amendment 15: a carried caller whose unresolved call names a
    // symbol added later must be re-extracted by the fast path.
    let repo = temp_repo("gap-heal");
    write(&repo, "b.py", "def existing():\n    return 1\n");
    write(
        &repo,
        "a.py",
        "import b\n\ndef use():\n    return b.foo()\n",
    );
    build(&repo);
    let out = aw(&repo).args(["callers", "foo"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "foo does not exist yet");
    // Add the missing symbol; the next incremental build must heal a.py.
    write(
        &repo,
        "b.py",
        "def existing():\n    return 1\n\ndef foo():\n    return 2\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let out = aw(&repo).args(["callers", "foo"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "fast path healed the gap: {:?}",
        out
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("a.py:4"), "{stdout}");
}

#[test]
fn constructor_calls_attribute_to_the_class() {
    // R6d A6: `Widget()` must be a caller of the class (not only __init__),
    // so `callers Class` works and `dead` does not list an instantiated class.
    let repo = temp_repo("ctor");
    write(
        &repo,
        "c.py",
        "class Widget:\n    def __init__(self):\n        self.x = 1\n\ndef make():\n    return Widget()\n",
    );
    build(&repo);
    let out = aw(&repo).args(["callers", "Widget"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "class has an inbound call edge: {:?}",
        out
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("c.py:6"), "{stdout}");
    let out = aw(&repo)
        .args(["callers", "c.Widget.__init__"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "ctor keeps its own edge");
    let out = aw(&repo).args(["dead"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("c.py:1"),
        "instantiated class must not be dead: {stdout}"
    );
}

#[test]
fn grammar_version_matches_cargo_lock() {
    // R6d A8: the fast-path grammar identity is derived from Cargo.lock and
    // must not drift from the pinned crates.
    let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock")).unwrap();
    let mut versions = std::collections::HashMap::new();
    let mut name = String::new();
    for line in lock.lines() {
        if let Some(rest) = line.strip_prefix("name = ") {
            name = rest.trim().trim_matches('"').to_string();
        } else if let Some(rest) = line.strip_prefix("version = ") {
            versions.insert(name.clone(), rest.trim().trim_matches('"').to_string());
        }
    }
    for (crate_name, short) in [
        ("tree-sitter-python", "py"),
        ("tree-sitter-typescript", "ts"),
        ("tree-sitter-javascript", "js"),
        ("tree-sitter-rust", "rs"),
    ] {
        let expected = format!("{short}-{}", versions[crate_name]);
        assert!(
            agent_weave::db::GRAMMAR_VERSION.contains(&expected),
            "GRAMMAR_VERSION {} missing {expected}",
            agent_weave::db::GRAMMAR_VERSION
        );
    }
}

#[test]
fn tracked_but_gitignored_sources_are_indexed() {
    // R6d A9: a force-added file covered by .gitignore is git-visible and
    // must be indexed.
    let repo = temp_repo("tracked-ignored");
    write(&repo, ".gitignore", "generated/\n");
    write(&repo, "generated/g.py", "def hidden_fn():\n    return 1\n");
    let out = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(out.status.success());
    let out = std::process::Command::new("git")
        .args(["add", "-f", "generated/g.py"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(out.status.success());
    build(&repo);
    let out = aw(&repo).args(["stats"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("files=1"),
        "tracked-ignored file indexed: {stdout}"
    );
    let out = aw(&repo).args(["callers", "hidden_fn"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "defined but uncalled");
    let out = aw(&repo)
        .args(["symbols", "generated/g.py"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "file indexed");
}

#[test]
fn parallel_builds_produce_identical_tables() {
    // G12 (R6 round-2): fresh builds must be byte-identical regardless of
    // worker scheduling. Dump canonical rows twice and compare.
    let repo = temp_repo("parallel-det");
    for i in 0..40 {
        write(
            &repo,
            &format!("f{i:02}.py"),
            &format!("def a{i}():\n    return b{i}()\n\ndef b{i}():\n    return {i}\n"),
        );
    }
    let dump = |tag: &str| -> String {
        let db = repo.join(".aw/index.db");
        let out = Command::new("python3")
            .arg("-c")
            .arg(format!(
                "import sqlite3,hashlib\nc=sqlite3.connect('file:{}?mode=ro',uri=True)\n\
                 rows=[tuple(str(x) for x in r) for q in [\
                 'SELECT path,size,parse_status,error_nodes,unresolved,loc,cyclomatic,generated FROM files ORDER BY path',\
                 'SELECT qualname,kind,line_start,line_end,signature,cyclomatic,branches,nesting,effective_loc,exported,file_path FROM symbols ORDER BY qualname,file_path,line_start',\
                 'SELECT src_id,dst_id,dst_name,kind,confidence,site_line FROM edges ORDER BY src_id,dst_id,kind,site_line',\
                 'SELECT file_id,dst_name,kind,site_line FROM unresolved_calls ORDER BY file_id,kind,dst_name,site_line']\
                 for r in c.execute(q)]\nprint(hashlib.sha256(repr(rows).encode()).hexdigest())",
                db.display()
            ))
            .output()
            .unwrap();
        assert!(out.status.success(), "{tag}: dump failed");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    build(&repo);
    let first = dump("first");
    let (code, stdout, stderr) = build(&repo); // fast path
    assert_eq!(code, 0, "{stdout}{stderr}");
    let second = dump("second");
    let out = aw(&repo).args(["build", "--force"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let third = dump("force");
    assert_eq!(first, second, "fast-path rebuild changed table content");
    assert_eq!(first, third, "forced rebuild changed table content");
}

#[test]
fn doctor_invalidate_removes_the_index() {
    let repo = fixture("invalidate");
    build(&repo);
    assert!(repo.join(".aw/index.db").exists());
    let out = aw(&repo).args(["doctor", "--invalidate"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(!repo.join(".aw/index.db").exists(), "index removed");
    let out = aw(&repo).args(["callers", "normalize"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "no index after invalidate");
}

#[test]
fn truncated_index_exits_3() {
    // CUJ3: a truncated index file is the corrupt-index class (exit 3).
    let repo = fixture("truncated");
    build(&repo);
    let db = repo.join(".aw/index.db");
    let mut bytes = fs::read(&db).unwrap();
    bytes.truncate(bytes.len() / 3);
    fs::write(&db, bytes).unwrap();
    let out = aw(&repo).args(["callers", "normalize"]).output().unwrap();
    assert_eq!(out.status.code(), Some(3), "truncated index is corrupt");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("corrupt-index"), "{stderr}");
}

#[test]
fn context_shows_definition_callers_and_callees() {
    // Amendment 17: one call returns the definition + signature, direct
    // callers, and direct callees with exact section headers.
    let repo = temp_repo("context");
    write(&repo, "a.py", "def target():\n    return 1\n");
    write(
        &repo,
        "b.py",
        "from a import target\n\ndef caller():\n    return target()\n",
    );
    write(
        &repo,
        "c.py",
        "from b import caller\n\ndef top():\n    return caller()\n",
    );
    build(&repo);
    let out = aw(&repo).args(["context", "caller"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let expected =
        "aw: context — resolved static edges only; dynamic dispatch may hide callers or callees\n\
b.py:3-4 function b.caller def caller()\n\
callers (showing 1 of 1):\n\
c.py:4 c.top\n\
callees (showing 1 of 1):\n\
a.py:1 a.target\n";
    assert_eq!(stdout, expected, "exact bytes");
    // G11: payload pointers are byte-identical to the stdout data lines.
    let payload = fs::read_to_string(repo.join(".aw/usage-results.jsonl")).unwrap();
    let last = payload.lines().last().unwrap();
    assert_eq!(
        payload_pointers(last),
        vec!["c.py:4 c.top".to_string(), "a.py:1 a.target".to_string()]
    );

    // Cap per section: two callers, --limit 1.
    write(
        &repo,
        "d.py",
        "from b import caller\n\ndef other():\n    return caller()\n",
    );
    build(&repo);
    let out = aw(&repo)
        .args(["context", "caller", "--limit", "1"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("callers (showing 1 of 2):"), "{stdout}");

    // Missing symbol: exit 1.
    let out = aw(&repo).args(["context", "nope_xyz"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not in index"), "{stderr}");

    // Ambiguous name: exit 2 with candidates.
    write(&repo, "e.py", "class A1:\n    def dup(self):\n        return 1\n\nclass B1:\n    def dup(self):\n        return 2\n");
    build(&repo);
    let out = aw(&repo).args(["context", "dup"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn empty_dead_and_clones_carry_their_caveat_on_stdout() {
    // Amendment 5/12d + R6 round-2: the caveat is the first stdout line even
    // for an empty result (exit 1 reason stays on stderr).
    let repo = temp_repo("empty-caveat");
    // No indexable symbols => dead has zero candidates (module-only file).
    write(&repo, "only.py", "x = 1\n");
    build(&repo);
    let out = aw(&repo).args(["dead"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("aw: dead-code candidates —"), "{stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("0 dead-code candidates"), "{stderr}");

    let out = aw(&repo).args(["clones"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("aw: clone candidates —"), "{stdout}");
}

#[test]
fn serial_and_parallel_builds_are_identical() {
    // G12: the worker count must not change the resulting tables.
    let mk = |name: &str| {
        let repo = temp_repo(name);
        for i in 0..30 {
            write(
                &repo,
                &format!("f{i:02}.py"),
                &format!("def a{i}():\n    return b{i}()\n\ndef b{i}():\n    return {i}\n"),
            );
        }
        repo
    };
    let serial = mk("serial");
    let parallel = mk("parallel");
    for (repo, workers) in [(&serial, "1"), (&parallel, "4")] {
        let out = Command::new(env!("CARGO_BIN_EXE_aw"))
            .arg("--root")
            .arg(repo)
            .arg("build")
            .env("AW_WORKERS", workers)
            .output()
            .unwrap();
        assert!(out.status.success());
    }
    let dump = |repo: &std::path::Path| -> String {
        let out = Command::new("python3")
            .arg("-c")
            .arg(format!(
                "import sqlite3,hashlib\nc=sqlite3.connect('file:{}?mode=ro',uri=True)\nrows=[]\nfor q in ['SELECT path,size,parse_status,error_nodes,unresolved,loc,cyclomatic,generated FROM files ORDER BY path','SELECT qualname,kind,line_start,line_end,signature,cyclomatic,branches,nesting,effective_loc,exported,file_path FROM symbols ORDER BY qualname,file_path,line_start','SELECT src_id,dst_id,dst_name,kind,confidence,site_line FROM edges ORDER BY src_id,dst_id,kind,site_line','SELECT file_id,dst_name,kind,site_line FROM unresolved_calls ORDER BY file_id,kind,dst_name,site_line']:\n rows += [tuple(str(x) for x in r) for r in c.execute(q)]\nprint(hashlib.sha256(repr(rows).encode()).hexdigest())",
                repo.join(".aw/index.db").display()
            ))
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    assert_eq!(
        dump(&serial),
        dump(&parallel),
        "serial vs parallel tables differ"
    );
}

#[test]
fn unresolved_names_flag_internal_misses() {
    // R7 L4: an unresolved bare name that exists in the index is a resolver
    // gap (actionable), distinct from external/builtin calls.
    let repo = temp_repo("unresolved-internal");
    write(&repo, "a.py", "def helper():\n    return 1\n");
    write(&repo, "b.py", "def go():\n    return helper()\n");
    build(&repo);
    let out = aw(&repo).args(["unresolved"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("1 distinct names (internal=1)"), "{stdout}");
    assert!(stdout.contains("helper n=1 [internal]"), "{stdout}");
}

#[test]
fn clones_show_the_enclosing_symbol() {
    // R7 L4: spans alone force a file read; the enclosing symbol names the
    // duplicated unit.
    let repo = temp_repo("clones-identity");
    let body = "def process(items):\n    a = 1\n    b = 2\n    c = 3\n    d = 4\n    e = 5\n    return a + b + c + d + e\n";
    write(
        &repo,
        "x.py",
        &format!("def x_header():\n    return 0\n\n{body}"),
    );
    write(
        &repo,
        "y.py",
        &format!("def y_header():\n    return 0\n\n{body}"),
    );
    build(&repo);
    let out = aw(&repo).args(["clones"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout
            .lines()
            .any(|l| l.contains("x.py:") && l.contains("(x.process)")),
        "enclosing symbol missing: {stdout}"
    );
    // --min-lines filters AFTER merging: the merged span (10 lines) survives
    // 8 and disappears at 12 (R7 review: only 5 and 100 were tested).
    let out = aw(&repo)
        .args(["clones", "--min-lines", "8"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    let out = aw(&repo)
        .args(["clones", "--min-lines", "12"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("0 clone candidates"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn p8a_impact_defaults_to_files_with_site_counts() {
    let repo = temp_repo("impact-sites");
    write(&repo, "a.py", "def target():\n    return 1\n");
    write(
        &repo,
        "b.py",
        "from a import target\n\ndef one():\n    return target()\n\ndef two():\n    return target()\n",
    );
    build(&repo);
    let out = aw(&repo).args(["impact", "a.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert_eq!(
        stdout.lines().count(),
        1,
        "one row per caller file: {stdout}"
    );
    assert!(
        stdout.contains("b.py") && stdout.contains("(2 sites)"),
        "{stdout}"
    );

    let out = aw(&repo)
        .args(["impact", "a.py", "--sites"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("b.py:4") && stdout.contains("b.py:7"),
        "{stdout}"
    );
}

#[test]
fn dead_exclude_tests_drops_test_paths_and_names() {
    let repo = temp_repo("dead-exclude");
    write(&repo, "src.py", "def helper():\n    return 1\n");
    write(
        &repo,
        "tests/test_x.py",
        "def test_helper():\n    return 1\n",
    );
    build(&repo);
    let out = aw(&repo).args(["dead"]).output().unwrap();
    let all = String::from_utf8_lossy(&out.stdout);
    assert!(all.contains("tests/test_x.py"), "{all}");
    let out = aw(&repo)
        .args(["dead", "--exclude-tests"])
        .output()
        .unwrap();
    let filtered = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{filtered}");
    assert!(filtered.contains("src.py"), "{filtered}");
    assert!(!filtered.contains("test_x.py"), "{filtered}");
    assert!(!filtered.contains("test_helper"), "{filtered}");
}

#[test]
fn search_stem_fallback_handles_ion_nouns() {
    let repo = temp_repo("search-ion");
    write(&repo, "a.py", "def rotate():\n    return 1\n");
    build(&repo);
    let out = aw(&repo).args(["search", "rotation"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("a.rotate"), "{stdout}");
}

/// Split a documented command line into shell-like words, honoring single and
/// double quotes. Keeps the docs test dependency-free.
fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in line.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => cur.push(ch),
            None if ch == '\'' || ch == '"' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            }
            None => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

/// Every `aw ...` invocation shown in README.md and docs/*.md must parse with
/// the real clap definition, so documentation cannot drift from `src/cli.rs`.
#[test]
fn documented_aw_commands_parse() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut docs = vec![root.join("README.md")];
    for entry in fs::read_dir(root.join("docs")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            docs.push(path);
        }
    }
    let mut checked = 0usize;
    let mut errors = Vec::new();
    for path in docs {
        let text = fs::read_to_string(&path).unwrap();
        let mut fenced = false;
        for (n, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                fenced = !fenced;
                continue;
            }
            if !fenced || !trimmed.starts_with("aw ") {
                continue;
            }
            // Skip placeholders, alternations, pipes, redirects, comments, and
            // help/version probes: these are templates, not literal invocations.
            if trimmed.contains('<')
                || trimmed.contains('[')
                || trimmed.contains('|')
                || trimmed.contains('>')
                || trimmed.contains('#')
                || trimmed.contains('$')
                || trimmed.contains('\\')
                || trimmed.contains("...")
                || trimmed.contains("--help")
                || trimmed.contains("--version")
            {
                continue;
            }
            let argv = shell_words(trimmed);
            match <agent_weave::cli::Cli as clap::Parser>::try_parse_from(&argv) {
                Ok(_) => checked += 1,
                Err(e) => errors.push(format!("{}:{}: `{trimmed}` -> {e}", path.display(), n + 1)),
            }
        }
    }
    assert!(
        errors.is_empty(),
        "documented commands failed:\n{}",
        errors.join("\n")
    );
    assert!(
        checked >= 25,
        "only {checked} literal aw invocations checked; docs test lost coverage"
    );
}
