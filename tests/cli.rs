//! Integration tests: CUJs 1-8, G4 staleness, and the read-only tree
//! snapshot invariant, exercised against a throwaway fixture repo.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static TEMP_SEQ: AtomicUsize = AtomicUsize::new(0);

fn temp_repo(name: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("aw-it-{}-{}-{seq}", name, std::process::id()));
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
    c.env("AW_GLOBAL_DIR", repo.join(".test_global"));
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

/// Run one git command in a temp repo; fail the test on non-zero exit.
fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
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
        stderr.contains("schema v99, binary expects v10 — run aw build"),
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
        vec!["tests", "f"],
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
fn impact_default_empty_names_importers() {
    // Default calls-only impact on a file with importers but no inbound calls
    // must not be a bare zero: the detail names the importer count and the
    // rerun hint, while the caveat header states the calls-only shape.
    let repo = temp_repo("impact-importer");
    write(&repo, "a.py", "def only():\n    return 1\n");
    write(&repo, "b.py", "import a\n");
    build(&repo);
    let out = aw(&repo).args(["impact", "a.py"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("1 file(s) import and 0 file(s) inherit from this file"),
        "{stderr}"
    );
    assert!(
        stderr.contains("rerun with --edges imports,inherits"),
        "{stderr}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("aw: impact — resolved static call edges only"),
        "{stdout}"
    );
    // The named relationship is real: --edges imports finds the importer.
    let out = aw(&repo)
        .args(["impact", "a.py", "--edges", "imports"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("b.py"), "{stdout}");
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
    let out = aw(&repo)
        .args(["impact", "src/lib.rs", "--edges", "inherits"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("src/user.rs"), "{stdout}");

    write(
        &repo,
        "src/qualified.rs",
        "use crate::Greet;\npub struct Other;\nimpl crate::Greet for Other {\n    fn hello(&self) {}\n}\n",
    );
    build(&repo);
    let out = aw(&repo)
        .args(["impact", "src/lib.rs", "--edges", "inherits"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("src/qualified.rs"), "{stdout}");
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
fn older_extraction_revision_rebuilds_once() {
    let repo = fixture("extraction-upgrade");
    assert_eq!(build(&repo).0, 0);
    let db = repo.join(".aw/index.db");
    let status = Command::new("python3")
        .arg("-c")
        .arg("import sqlite3, sys; c=sqlite3.connect(sys.argv[1]); c.execute(\"UPDATE meta SET value='2' WHERE key='signature_version'\"); c.commit()")
        .arg(db)
        .status()
        .unwrap();
    assert!(status.success());
    let (code, first, stderr) = build(&repo);
    assert_eq!(code, 0, "{stderr}");
    assert!(first.contains("carried=0"), "{first}");
    let (code, second, stderr) = build(&repo);
    assert_eq!(code, 0, "{stderr}");
    assert!(second.contains("carried=3"), "{second}");
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
fn unresolved_path_scope_binds_params() {
    let repo = temp_repo("unresolved-path-scope");
    write(
        &repo,
        "src/sample.py",
        "def task(receiver):\n    receiver.unknown_method()\n",
    );
    build(&repo);

    let out = aw(&repo)
        .args(["unresolved", "--path", "src"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("src/sample.py unresolved=1"), "{stdout}");

    let names = aw(&repo)
        .args(["unresolved", "--names", "--path", "src"])
        .output()
        .unwrap();
    let nstdout = String::from_utf8_lossy(&names.stdout);
    assert_eq!(names.status.code(), Some(0), "{nstdout}");
    assert!(
        nstdout.contains("src/sample.py:2 call receiver.unknown_method"),
        "{nstdout}"
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
fn callees_empty_names_unresolved_sites() {
    let repo = temp_repo("callees-empty-unresolved");
    write(&repo, "a.py", "def entry():\n    return missing.run()\n");
    assert_eq!(build(&repo).0, 0);
    let out = aw(&repo).args(["callees", "entry"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(combined.contains("unresolved call site"), "{combined}");
    assert!(combined.contains("missing.run"), "{combined}");
    assert!(combined.contains("aw unresolved --names"), "{combined}");
}

#[test]
fn callees_empty_without_unresolved_stays_plain() {
    let repo = temp_repo("callees-empty-plain");
    write(&repo, "a.py", "def empty():\n    return 1\n");
    assert_eq!(build(&repo).0, 0);
    let out = aw(&repo).args(["callees", "empty"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(combined.contains("0 callees found"), "{combined}");
    assert!(!combined.contains("unresolved call site"), "{combined}");
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
fn callers_depth_two_reaches_transitive_callers() {
    let repo = temp_repo("callers-depth");
    write(&repo, "c.py", "def h():\n    pass\n");
    write(&repo, "b.py", "from c import h\n\ndef g():\n    h()\n");
    write(&repo, "a.py", "from b import g\n\ndef f():\n    g()\n");
    build(&repo);

    // Depth 1 keeps the existing direct-caller format and does not add tags.
    let direct = aw(&repo).args(["callers", "h"]).output().unwrap();
    let d1 = String::from_utf8_lossy(&direct.stdout);
    assert_eq!(direct.status.code(), Some(0), "{d1}");
    assert!(d1.contains("b.g"), "{d1}");
    assert!(!d1.contains("a.f"), "{d1}");
    assert!(!d1.contains("[depth="), "{d1}");

    let out = aw(&repo)
        .args(["callers", "h", "--depth", "2"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("transitive callers"), "{stdout}");
    assert!(stdout.contains("b.g [depth=1]"), "{stdout}");
    assert!(stdout.contains("a.f [depth=2]"), "{stdout}");
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

    let out = aw(&repo)
        .args(["impact", "lib.rs", "--edges", "inherits"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
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
fn search_limit_counts_union_by_identity() {
    let repo = temp_repo("search-overcount");
    write(&repo, "a.py", "def run(): pass\n");
    write(&repo, "b.py", "def run(): pass\n");
    write(&repo, "c.py", "def run(): pass\ndef run_build(): pass\n");
    build(&repo);
    let out = aw(&repo)
        .args(["search", "run", "--limit", "1"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("showing 1 of 4"), "{text}");
}

#[test]
fn search_exact_first_merge_and_union_count_characterization() {
    let repo = temp_repo("search-characterize");
    write(&repo, "a.py", "def run(): pass\n");
    write(&repo, "b.py", "def run(): pass\n");
    write(&repo, "c.py", "def run_build(): pass\n");
    build(&repo);
    let out = aw(&repo)
        .args(["search", "run", "--limit", "10"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.lines().collect();
    let exact = lines
        .iter()
        .position(|line| line.contains(" a.run ") || line.contains(" b.run "))
        .expect("exact run row");
    let prefix = lines
        .iter()
        .position(|line| line.contains(" c.run_build "))
        .expect("prefix run_build row");
    assert!(exact < prefix, "exact-first merge order broken: {lines:?}");

    let capped = aw(&repo)
        .args(["search", "run", "--limit", "1"])
        .output()
        .unwrap();
    assert_eq!(capped.status.code(), Some(0), "{capped:?}");
    assert!(
        String::from_utf8_lossy(&capped.stdout).contains("showing 1 of 3"),
        "union count changed: {capped:?}"
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
fn usage_quality_adds_empty_split_and_keeps_default_summary() {
    let repo = fixture("usage-quality");
    aw(&repo).args(["build"]).output().unwrap();
    aw(&repo).args(["callers", "normalize"]).output().unwrap();

    let default = aw(&repo).args(["usage"]).output().unwrap();
    assert_eq!(default.status.code(), Some(0));
    let default_stdout = String::from_utf8_lossy(&default.stdout);
    assert!(
        default_stdout.contains("callers: 1 calls"),
        "{default_stdout}"
    );
    assert!(!default_stdout.contains("empty_ok="), "{default_stdout}");

    let quality = aw(&repo).args(["usage", "--quality"]).output().unwrap();
    assert_eq!(quality.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&quality.stdout);
    assert!(stdout.contains("empty_ok="), "{stdout}");
    assert!(stdout.contains("empty_error="), "{stdout}");
    assert!(stdout.contains("capped="), "{stdout}");
    assert!(stdout.contains('%'), "{stdout}");
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

#[test]
fn usage_journal_creates_gitignore_marker() {
    // The journal self-creates `.aw` on the first write; the tool-owned
    // gitignore marker must exist so the directory never leaks into git status.
    let repo = temp_repo("journal-gitignore");
    write(&repo, "a.py", "def f(): pass\n");
    // Baseline commit: `git status --porcelain` must be empty before the
    // journal writes anything, so only `.aw` leakage could dirty it.
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "-A"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-qm",
            "init",
        ],
    );

    // `callers` fails with no index (exit 1) but still journals.
    let out = aw(&repo).args(["callers", "f"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(repo.join(".aw/usage.log").exists(), "journal file written");
    let marker = fs::read_to_string(repo.join(".aw/.gitignore")).unwrap();
    assert_eq!(marker, "*\n", "tool-owned gitignore marker");
    let status = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&status.stdout).trim().is_empty(),
        ".aw must be invisible to git: {:?}",
        String::from_utf8_lossy(&status.stdout)
    );
}

#[cfg(unix)]
#[test]
fn fifo_gitignore_marker_fails_closed_without_hanging() {
    // A FIFO at .aw/.gitignore is fail-closed: rejected before any open (no
    // hang), left in place, and never written through.
    use std::os::unix::fs::FileTypeExt;
    let repo = temp_repo("fifo-gitignore");
    write(&repo, "a.py", "def f(): pass\n");
    fs::create_dir_all(repo.join(".aw")).unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(repo.join(".aw/.gitignore"))
            .status()
            .unwrap()
            .success(),
        "mkfifo failed"
    );

    let mut cmd = aw(&repo);
    cmd.arg("build");
    // The budget only bounds the wait; a true FIFO hang is killed and still
    // fails via None. Sized for a loaded host, where a sub-second fail-closed
    // exit can inflate past a 20 s wall clock.
    let out = output_with_timeout(cmd, 60).expect("build must not hang on a FIFO .aw/.gitignore");
    assert_eq!(
        out.status.code(),
        Some(6),
        "FIFO marker must fail closed: {out:?}"
    );
    let meta = fs::symlink_metadata(repo.join(".aw/.gitignore")).unwrap();
    assert!(
        meta.file_type().is_fifo(),
        "marker must be left untouched, not replaced"
    );
}

#[cfg(unix)]
#[test]
fn fifo_indexed_file_does_not_hang_doctor() {
    // A FIFO replacing an indexed file must not block the staleness re-hash:
    // file_changed reads through the same bounded guard as the walk
    // (regular-file check + 1 MiB cap) and treats the oddity as changed. The
    // indexed file is empty so its stored size matches the FIFO's stat size
    // and the probe reaches the read path instead of short-circuiting on
    // size.
    use std::os::unix::fs::FileTypeExt;
    let repo = temp_repo("fifo-doctor");
    write(&repo, "empty.py", "");
    build(&repo);
    let fifo = repo.join("empty.py");
    fs::remove_file(&fifo).unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success(),
        "mkfifo failed"
    );
    let meta = fs::symlink_metadata(&fifo).unwrap();
    assert!(meta.file_type().is_fifo(), "fixture must be a FIFO");

    let mut cmd = aw(&repo);
    cmd.args(["doctor"]);
    let out = output_with_timeout(cmd, 10)
        .expect("doctor must not hang on an indexed file replaced by a FIFO");
    assert_eq!(out.status.code(), Some(0), "doctor must succeed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("stale ratio: 100.0%"),
        "the FIFO must count as changed (1 of 1 files): {stdout}"
    );
}

#[test]
fn session_and_arm_tags_recorded_in_usage_log() {
    // AW_SESSION/AW_ARM tags, sanitized, land in the usage log.
    let repo = fixture("session-tags");
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
    let mut pgrep_ok = 0usize;
    loop {
        match Command::new("pgrep").args(["-P", &pid, "-l"]).output() {
            Ok(out) if out.status.code() == Some(0) => {
                pgrep_ok += 1;
                for line in String::from_utf8_lossy(&out.stdout).lines() {
                    if let Some((_, comm)) = line.split_once(char::is_whitespace) {
                        let comm = comm.trim();
                        if !comm.is_empty() {
                            seen.insert(comm.to_string());
                        }
                    }
                }
            }
            Ok(out) if out.status.code() == Some(1) => {
                pgrep_ok += 1;
            }
            Ok(out) => {
                if child.try_wait().unwrap().is_none() {
                    panic!("pgrep failed while build was alive: {out:?}");
                }
            }
            Err(e) => panic!("pgrep inspection failed: {e}"),
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let status = child.wait().unwrap();
    assert!(status.success());
    assert!(pgrep_ok > 0, "process inspection never succeeded");
    // A non-git fixture may have no child at all; the seeded-child test below
    // separately proves the inspection mechanism is not vacuous.
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
    assert!(
        probe.status.code() == Some(1),
        "pgrep inspection did not report a clean no-match: {probe:?}"
    );
    assert!(alive.trim().is_empty(), "processes still alive: {alive}");
}

#[test]
fn g5_process_inspection_detects_seeded_child() {
    // Guard the guard: pgrep must see a live descendant whose command line
    // carries the marker. Without this, an empty result could pass vacuously.
    let marker = format!("aw-seed-{}", std::process::id());
    let script = format!("sleep 30; : {marker}");
    let mut child = Command::new("sh").args(["-c", &script]).spawn().unwrap();
    let mut found = false;
    for _ in 0..100 {
        let out = Command::new("pgrep")
            .args(["-f", &marker])
            .output()
            .unwrap();
        assert!(out.status.code() == Some(0) || out.status.code() == Some(1));
        if out.status.code() == Some(0) {
            found = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(found, "pgrep did not detect the seeded child");
}

#[test]
fn p15_stats_line() {
    let repo = fixture("p15");
    build(&repo);
    let out = aw(&repo).args(["stats"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("files=3") && stdout.contains("symbols=") && stdout.contains("edges="),
        "{stdout}"
    );
}

#[test]
fn stats_path_scopes_counts() {
    // `--path` scopes every counter; the global line keeps its byte shape
    // (no `scope=` segment), and a scope with no indexed files is a
    // no-results, not an empty success line.
    let repo = temp_repo("stats-scope");
    write(
        &repo,
        "a.py",
        "def a1():\n    return a2()\n\ndef a2():\n    return 1\n",
    );
    write(
        &repo,
        "sub/c.py",
        "def c1():\n    return c2()\n\ndef c2():\n    return 1\n",
    );
    write(&repo, "sub/e.py", "def e1():\n    return 1\n");
    build(&repo);

    let out = aw(&repo).args(["stats"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let global = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        global.starts_with("aw stats: files=3 symbols=8 edges="),
        "global stats: {global}"
    );
    assert!(
        !global.contains("scope="),
        "global stats must not carry a scope: {global}"
    );

    let out = aw(&repo).args(["stats", "--path", "sub"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let scoped = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        scoped.starts_with("aw stats: scope=sub files=2 symbols=5 edges="),
        "scoped stats: {scoped}"
    );
    assert!(
        scoped.contains(" parse_errors=0 unresolved=0 "),
        "scoped stats: {scoped}"
    );

    // Trailing-slash and ./-prefixed spellings normalize to the same scope.
    let out = aw(&repo)
        .args(["stats", "--path", "./sub/"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        scoped,
        "./sub/ must normalize to sub"
    );

    // Scoped totals shrink with the scope on this fixture. Edge-kind totals
    // are not pinned: new kinds extend both lines.
    let field = |line: &str, key: &str| -> i64 {
        let start = line.find(&format!("{key}=")).unwrap() + key.len() + 1;
        line[start..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap()
    };
    assert!(
        field(&scoped, "calls") < field(&global, "calls"),
        "{scoped} vs {global}"
    );
    assert!(
        field(&scoped, "edges") < field(&global, "edges"),
        "{scoped} vs {global}"
    );

    let out = aw(&repo)
        .args(["stats", "--path", "nope"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "aw: no-results: no indexed files in 'nope'\n"
    );

    let out = aw(&repo).args(["stats", "--path", "."]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("invalid scope '.'"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn partial_secondary_index_rebuilds_fresh() {
    let repo = fixture("partial-index");
    build(&repo);
    let idx = repo.join(".aw/index.db");
    let status = std::process::Command::new("sqlite3")
        .args([
            idx.to_string_lossy().as_ref(),
            "DROP INDEX symbols_file_path;",
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
    let repo = fixture("partial-file-path");
    build(&repo);
    let idx = repo.join(".aw/index.db");
    let status = std::process::Command::new("sqlite3")
        .args([
            idx.to_string_lossy().as_ref(),
            "DROP INDEX symbols_file_path;
             ALTER TABLE symbols DROP COLUMN file_path;",
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
    let out = aw(&repo).args(["search", "normalize"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("utils.py"),
        "fresh index lost symbols after v10 rebuild"
    );
}

#[test]
fn old_version_database_rebuilds_to_v10() {
    // A v6 user_version disables the fast path; build rewrites the index at
    // the current schema version instead of carrying v6 rows.
    let repo = fixture("v6-to-v8");
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
        stderr.contains("schema v6, binary expects v10 — run aw build"),
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
        "10",
        "build did not stamp v10"
    );
}

#[test]
fn partial_v10_index_rebuilds_fresh() {
    let repo = fixture("partial-v4");
    build(&repo);
    let idx = repo.join(".aw/index.db");
    let status = std::process::Command::new("sqlite3")
        .args([
            idx.to_string_lossy().as_ref(),
            "ALTER TABLE symbols DROP COLUMN signature;",
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
    assert!(!stderr.contains("signature"), "{stdout}{stderr}");
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
fn gap_healing_reapplies_inbound_closure() {
    // ANEW1: a gap-healed file is re-extracted after the initial dirty
    // closure. Its inbound caller stays carried and points at the old symbol
    // ids, so the build must re-apply the inbound closure to the healed file.
    let repo = temp_repo("gap-inbound");
    write(
        &repo,
        "a.py",
        "import missing\n\ndef entry():\n    return missing.foo()\n",
    );
    write(
        &repo,
        "b.py",
        "import a\n\ndef invoke():\n    return a.entry()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let before = aw(&repo).args(["callees", "invoke"]).output().unwrap();
    assert_eq!(before.status.code(), Some(0), "{before:?}");
    assert!(
        String::from_utf8_lossy(&before.stdout).contains("a.entry"),
        "{before:?}"
    );

    // The new symbol heals a.py after the dirty closure; b.py must be
    // re-extracted so its copied edge survives the fresh a.entry ids.
    write(&repo, "missing.py", "def foo():\n    pass\n");
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let after = aw(&repo).args(["callees", "invoke"]).output().unwrap();
    assert_eq!(after.status.code(), Some(0), "{after:?}");
    assert!(
        String::from_utf8_lossy(&after.stdout).contains("a.entry"),
        "incremental lost the inbound edge: {after:?}"
    );

    let incremental_db = repo.join("incremental.db");
    fs::copy(repo.join(".aw/index.db"), &incremental_db).unwrap();
    let force = aw(&repo).args(["build", "--force"]).output().unwrap();
    assert_eq!(force.status.code(), Some(0), "{force:?}");
    assert_eq!(
        dump_edges(&incremental_db),
        dump_edges(&repo.join(".aw/index.db")),
        "incremental and fresh edge sets differ"
    );
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
                 'SELECT path,size,parse_status,error_nodes,unresolved FROM files ORDER BY path',\
                 'SELECT qualname,kind,line_start,line_end,signature,is_test,file_path FROM symbols ORDER BY qualname,file_path,line_start',\
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
        "aw: context — resolved static edges plus tagged [ambiguous] candidates; dynamic dispatch may hide callers or callees\n\
b.py:3-4 function b.caller def caller()\n\
callers (showing 1 of 1):\n\
c.py:4 c.top\n\
callees (showing 1 of 1):\n\
a.py:1 a.target\n";
    assert_eq!(stdout, expected, "exact bytes");

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
        let out = aw(repo)
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
                "import sqlite3,hashlib\nc=sqlite3.connect('file:{}?mode=ro',uri=True)\nrows=[]\nfor q in ['SELECT path,size,parse_status,error_nodes,unresolved FROM files ORDER BY path','SELECT qualname,kind,line_start,line_end,signature,is_test,file_path FROM symbols ORDER BY qualname,file_path,line_start','SELECT src_id,dst_id,dst_name,kind,confidence,site_line FROM edges ORDER BY src_id,dst_id,kind,site_line','SELECT file_id,dst_name,kind,site_line FROM unresolved_calls ORDER BY file_id,kind,dst_name,site_line']:\n rows += [tuple(str(x) for x in r) for r in c.execute(q)]\nprint(hashlib.sha256(repr(rows).encode()).hexdigest())",
                repo.join(".aw/index.db").display()
            ))
            .output()
            .unwrap();
        assert!(out.status.success(), "dump failed");
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

/// A gitignored tree can be huge and unreadable. `build` must not descend it:
/// the git-visible path list is enough to find the indexed files.
#[cfg(unix)]
#[test]
fn build_does_not_descend_unreadable_gitignored_tree() {
    use std::os::unix::fs::PermissionsExt;
    let repo = temp_repo("walk-ignored");
    write(&repo, ".gitignore", "generated/\n");
    write(&repo, "a.py", "def a():\n    return 1\n");
    let ignored = repo.join("generated");
    fs::create_dir_all(&ignored).unwrap();
    for i in 0..3000 {
        fs::write(ignored.join(format!("dummy_{i}.py")), "x = 1\n").unwrap();
    }
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "-A"]);
    fs::set_permissions(&ignored, fs::Permissions::from_mode(0o000)).unwrap();

    let started = std::time::Instant::now();
    let out = aw(&repo).args(["build"]).output().unwrap();
    let elapsed = started.elapsed();
    fs::set_permissions(&ignored, fs::Permissions::from_mode(0o755)).unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout={stdout} stderr={stderr}"
    );
    assert!(stdout.contains("files=1"), "{stdout}");
    // A healthy build of this fixture takes well under a second; the budget
    // guards against pathological slowness, not exact latency, so it is sized
    // for a loaded host where scheduling can inflate wall clock ~10x.
    assert!(
        elapsed < std::time::Duration::from_secs(60),
        "build took {elapsed:?}"
    );
}

#[test]
fn class_method_suffix_lookup_finds_callers_and_ambiguity() {
    let repo = temp_repo("suffix");
    write(
        &repo,
        "a.py",
        "class Pointer:\n    def render(self):\n        return 1\n\ndef go(p: Pointer):\n    return p.render()\n\ndef make():\n    return Pointer().render()\n",
    );
    build(&repo);
    let out = aw(&repo)
        .args(["callers", "Pointer.render"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("a.go"), "{stdout}");
    assert!(stdout.contains("a.make"), "{stdout}");

    // Two same-suffix methods stay ambiguous (exit 2 + candidates).
    let repo2 = temp_repo("suffix-ambiguous");
    write(
        &repo2,
        "m1.py",
        "class Pointer:\n    def render(self):\n        return 1\n",
    );
    write(
        &repo2,
        "m2.py",
        "class Pointer:\n    def render(self):\n        return 2\n",
    );
    build(&repo2);
    let out = aw(&repo2)
        .args(["callers", "Pointer.render"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stdout} {stderr}");
    assert!(stdout.contains("m1.Pointer.render"), "{stdout}");
    assert!(stdout.contains("m2.Pointer.render"), "{stdout}");
}

#[test]
fn search_keeps_prefix_matches_after_an_exact_symbol() {
    let repo = temp_repo("search-prefix");
    write(
        &repo,
        "a.py",
        "def run():\n    return 1\n\ndef run_build():\n    return 2\n",
    );
    build(&repo);
    let out = aw(&repo).args(["search", "run"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    let exact = stdout.find("a.run ").or_else(|| stdout.find("a.run\n"));
    let prefix = stdout.find("a.run_build");
    assert!(exact.is_some(), "exact run missing: {stdout}");
    assert!(prefix.is_some(), "run_build dropped: {stdout}");
    assert!(
        exact.unwrap() < prefix.unwrap(),
        "exact must rank first: {stdout}"
    );
}

#[test]
fn ambiguous_pointer_keeps_space_before_the_tag() {
    let repo = temp_repo("ambiguous-render");
    write(
        &repo,
        "a.py",
        "class A:\n    def m(self):\n        return 1\n\nclass B(A):\n    def m(self):\n        return 2\n\nclass C(A):\n    def m(self):\n        return 3\n\nclass D(B, C):\n    def go(self):\n        return self.m()\n",
    );
    build(&repo);
    let out = aw(&repo).args(["callees", "D.go"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains(" [ambiguous] a.D.m"), "{stdout}");
    assert!(!stdout.contains(":15[ambiguous]"), "{stdout}");
}

#[test]
fn root_symbols_work() {
    let repo = temp_repo("root-symbols");
    write(&repo, "a.py", "def orphan():\n    return 1\n");
    build(&repo);

    let root = aw(&repo).args(["symbols", "."]).output().unwrap();
    let root_stdout = String::from_utf8_lossy(&root.stdout);
    assert_eq!(root.status.code(), Some(0), "{root_stdout}");
    assert!(root_stdout.contains("aw: dir . —"), "{root_stdout}");
    let dotted = aw(&repo).args(["symbols", "./"]).output().unwrap();
    assert_eq!(dotted.status.code(), Some(0));
}

#[test]
fn stats_is_not_journaled_as_empty() {
    let repo = temp_repo("journal-results");
    write(&repo, "a.py", "def a():\n    return 1\n");
    build(&repo);
    let out = aw(&repo)
        .arg("stats")
        .env_remove("AW_NO_JOURNAL")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "stats");
    let usage = aw(&repo).args(["usage"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&usage.stdout);
    assert_eq!(usage.status.code(), Some(0), "{stdout}");
    let line = stdout
        .lines()
        .find(|l| l.trim_start().starts_with("stats:"))
        .unwrap_or_else(|| panic!("stats missing from journal: {stdout}"));
    assert!(!line.contains("empty=1"), "stats misreported empty: {line}");
}

#[test]
fn doctor_does_not_warn_about_a_released_build_lock() {
    let repo = temp_repo("doctor-lock");
    write(&repo, "a.py", "def a():\n    return 1\n");
    build(&repo);
    let out = aw(&repo).args(["doctor"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(!stdout.contains("lock file:"), "{stdout}");
}

#[test]
fn path_endpoints_accept_class_method_suffixes() {
    let repo = temp_repo("path-suffix");
    write(
        &repo,
        "a.py",
        "class A:\n    def m(self):\n        return done()\n\ndef done():\n    return 1\n\ndef start(a: A):\n    return a.m()\n",
    );
    build(&repo);
    let out = aw(&repo).args(["path", "start", "A.m"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stdout}{stderr}");
    assert!(stdout.contains("a.A.m"), "{stdout}");
}

#[cfg(unix)]
fn external_symlink_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aw-ext-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(unix)]
#[test]
fn symlinked_aw_directory_is_rejected_without_external_writes() {
    use std::os::unix::fs::symlink;
    let repo = temp_repo("symlink-aw-dir");
    write(&repo, "sample.py", "def task():\n    pass\n");
    let ext = external_symlink_dir("aw-dir");
    let sentinel = ext.join("sentinel.txt");
    fs::write(&sentinel, "keep").unwrap();
    let _ = fs::remove_dir_all(repo.join(".aw"));
    symlink(&ext, repo.join(".aw")).unwrap();

    let out = aw(&repo).args(["build"]).output().unwrap();
    assert_ne!(out.status.code(), Some(0), "symlinked .aw must be refused");
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "keep");
    assert!(!ext.join("index.db").exists());
    let _ = fs::remove_dir_all(&ext);
}

#[cfg(unix)]
#[test]
fn symlinked_aw_directory_to_missing_target_is_not_created() {
    use std::os::unix::fs::symlink;
    let repo = temp_repo("symlink-aw-missing");
    write(&repo, "sample.py", "def task():\n    pass\n");
    let target = std::env::temp_dir().join(format!("aw-ext-aw-missing-{}", std::process::id()));
    let _ = fs::remove_dir_all(&target);
    let _ = fs::remove_dir_all(repo.join(".aw"));
    symlink(&target, repo.join(".aw")).unwrap();

    let out = aw(&repo).args(["build"]).output().unwrap();
    assert_ne!(out.status.code(), Some(0));
    assert!(!target.exists(), "symlink target must not be created");
    let _ = fs::remove_file(repo.join(".aw"));
}

#[cfg(unix)]
#[test]
fn symlinked_build_lock_and_gitignore_are_rejected() {
    use std::os::unix::fs::symlink;
    let repo = temp_repo("symlink-lock");
    write(&repo, "sample.py", "def task():\n    pass\n");
    let ext = external_symlink_dir("lock");
    let sentinel = ext.join("sentinel.txt");
    fs::write(&sentinel, "keep").unwrap();
    fs::create_dir_all(repo.join(".aw")).unwrap();
    symlink(&sentinel, repo.join(".aw/build.lock")).unwrap();
    let lock = aw(&repo).args(["build"]).output().unwrap();
    assert_ne!(lock.status.code(), Some(0));
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "keep");

    fs::remove_file(repo.join(".aw/build.lock")).unwrap();
    symlink(&sentinel, repo.join(".aw/.gitignore")).unwrap();
    let ignore = aw(&repo).args(["build"]).output().unwrap();
    assert_ne!(ignore.status.code(), Some(0));
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "keep");
    let _ = fs::remove_file(repo.join(".aw/.gitignore"));
    let _ = fs::remove_dir_all(&ext);
}

#[cfg(unix)]
#[test]
fn invalidate_refuses_while_build_lock_is_held() {
    let repo = temp_repo("invalidate-lock");
    write(&repo, "sample.py", "def task():\n    pass\n");
    let _ = build(&repo);
    let sentinel = repo.join(".aw/sentinel");
    fs::write(&sentinel, "keep").unwrap();
    let lock = agent_weave::db::BuildLock::acquire(&repo).unwrap();

    let blocked = aw(&repo).args(["doctor", "--invalidate"]).output().unwrap();
    assert_eq!(blocked.status.code(), Some(4), "{blocked:?}");
    assert!(sentinel.exists(), "invalidate ran while root lock was held");

    drop(lock);
    let after = aw(&repo).args(["doctor", "--invalidate"]).output().unwrap();
    assert_eq!(after.status.code(), Some(0), "{after:?}");
    assert!(!repo.join(".aw").exists());
}

#[test]
fn removed_inheritance_does_not_survive_incremental_build() {
    let repo = temp_repo("removed-base");
    write(
        &repo,
        "sample.py",
        "class A:\n    def foo(self):\n        pass\n\nclass B(A):\n    pass\n\ndef invoke(b: B):\n    b.foo()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let before = aw(&repo).args(["callers", "A.foo"]).output().unwrap();
    assert_eq!(before.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&before.stdout).contains("sample.invoke"));

    write(
        &repo,
        "sample.py",
        "class A:\n    def foo(self):\n        pass\n\nclass B:\n    pass\n\ndef invoke(b: B):\n    b.foo()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let after = aw(&repo).args(["callers", "A.foo"]).output().unwrap();
    assert_eq!(after.status.code(), Some(1), "{after:?}");

    let force = aw(&repo).args(["build", "--force"]).output().unwrap();
    assert_eq!(force.status.code(), Some(0));
    let forced = aw(&repo).args(["callers", "A.foo"]).output().unwrap();
    assert_eq!(forced.status.code(), Some(1), "{forced:?}");
}

#[test]
fn added_duplicate_candidate_invalidates_resolved_edges() {
    let repo = temp_repo("added-candidate");
    write(
        &repo,
        "caller.py",
        "from pkg import foo\n\ndef invoke():\n    foo()\n",
    );
    write(&repo, "pkg.py", "def foo():\n    pass\n");
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let before = aw(&repo).args(["callees", "invoke"]).output().unwrap();
    assert_eq!(before.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&before.stdout).contains("pkg.foo"),
        "{before:?}"
    );

    write(&repo, "pkg/__init__.py", "def foo():\n    pass\n");
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let after = aw(&repo).args(["callees", "invoke"]).output().unwrap();
    assert_eq!(after.status.code(), Some(0), "{after:?}");
    assert!(
        String::from_utf8_lossy(&after.stdout).contains("[ambiguous]"),
        "incremental build kept a stale resolved edge: {after:?}"
    );

    let incremental_db = repo.join("incremental.db");
    fs::copy(repo.join(".aw/index.db"), &incremental_db).unwrap();
    let force = aw(&repo).args(["build", "--force"]).output().unwrap();
    assert_eq!(force.status.code(), Some(0));
    let forced = aw(&repo).args(["callees", "invoke"]).output().unwrap();
    assert_eq!(after.stdout, forced.stdout);
    assert_eq!(
        dump_edges(&incremental_db),
        dump_edges(&repo.join(".aw/index.db")),
        "incremental and fresh edge sets differ"
    );

    let ctx = aw(&repo).args(["context", "invoke"]).output().unwrap();
    assert_eq!(ctx.status.code(), Some(0), "{ctx:?}");
    let text = String::from_utf8_lossy(&ctx.stdout);
    assert!(
        text.starts_with("aw: context — resolved static edges plus tagged [ambiguous] candidates;"),
        "{text}"
    );
    assert!(text.contains("[ambiguous] foo"), "{text}");
}

#[test]
fn removed_duplicate_candidate_invalidates_ambiguous_sites() {
    // ANEW2: removing one of two same-qualname candidates must re-extract
    // carried callers with ambiguous edges; a null-destination edge is not in
    // the resolved-edge dirty closure.
    let repo = temp_repo("removed-candidate");
    write(
        &repo,
        "caller.py",
        "from pkg import foo\n\ndef invoke():\n    foo()\n",
    );
    write(&repo, "pkg.py", "def foo():\n    pass\n");
    write(&repo, "pkg/__init__.py", "def foo():\n    pass\n");
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let ambiguous = aw(&repo).args(["callees", "invoke"]).output().unwrap();
    assert_eq!(ambiguous.status.code(), Some(0), "{ambiguous:?}");
    assert!(
        String::from_utf8_lossy(&ambiguous.stdout).contains("[ambiguous] foo"),
        "{ambiguous:?}"
    );

    fs::remove_file(repo.join("pkg/__init__.py")).unwrap();
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let after = aw(&repo).args(["callees", "invoke"]).output().unwrap();
    assert_eq!(after.status.code(), Some(0), "{after:?}");
    let text = String::from_utf8_lossy(&after.stdout);
    assert!(
        !text.contains("[ambiguous]"),
        "incremental kept the ambiguous edge: {after:?}"
    );
    assert!(text.contains("pkg.foo"), "{after:?}");

    let incremental_db = repo.join("incremental.db");
    fs::copy(repo.join(".aw/index.db"), &incremental_db).unwrap();
    let force = aw(&repo).args(["build", "--force"]).output().unwrap();
    assert_eq!(force.status.code(), Some(0), "{force:?}");
    assert_eq!(
        dump_edges(&incremental_db),
        dump_edges(&repo.join(".aw/index.db")),
        "incremental and fresh edge sets differ"
    );
}

#[test]
fn assignment_shadows_bare_callable_name() {
    let repo = temp_repo("assignment-shadow");
    write(
        &repo,
        "sample.py",
        "def target():\n    pass\n\ndef go():\n    target = None\n    target()\n\ndef control():\n    target()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let out = aw(&repo).args(["callers", "target"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("sample.control"), "{text}");
    assert!(
        !text.contains("sample.go"),
        "assignment-shadowed call resolved: {text}"
    );
}

#[test]
fn arrow_parameter_shadows_global_function() {
    let repo = temp_repo("arrow-parameter");
    write(
        &repo,
        "sample.js",
        "function target() {}\nconst go = (target) => target();\nfunction control() { target(); }\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let out = aw(&repo).args(["callers", "target"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("sample.control"), "{text}");
    assert!(
        !text.contains("sample.go"),
        "arrow parameter did not shadow: {text}"
    );
}

#[test]
fn nested_self_parameter_shadows_receiver() {
    let repo = temp_repo("nested-self-shadow");
    write(
        &repo,
        "sample.py",
        "class C:\n    def run(self):\n        pass\n\n    def outer(self):\n        def inner(self):\n            self.run()\n        inner(self)\n\n    def ok(self):\n        self.run()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let out = aw(&repo).args(["callers", "C.run"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("sample.C.ok"), "{text}");
    assert!(
        !text.contains("sample.C.outer.inner"),
        "nested self shadow resolved: {text}"
    );
}

#[test]
fn ordered_bindings_resolve_at_call_site() {
    let repo = temp_repo("ordered-bindings");
    write(
        &repo,
        "sample.py",
        "class A:\n    def run(self): pass\n\nclass B:\n    def run(self): pass\n\ndef go():\n    value = A()\n    value.run()\n    value = B()\n    value.run()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let a = aw(&repo).args(["callers", "A.run"]).output().unwrap();
    assert_eq!(a.status.code(), Some(0), "{a:?}");
    let atext = String::from_utf8_lossy(&a.stdout);
    assert!(
        atext.contains("sample.py:9"),
        "first call must use A: {atext}"
    );
    assert!(
        !atext.contains("sample.py:11"),
        "later assignment changed the first call: {atext}"
    );
    let b = aw(&repo).args(["callers", "B.run"]).output().unwrap();
    assert_eq!(b.status.code(), Some(0), "{b:?}");
    let btext = String::from_utf8_lossy(&b.stdout);
    assert!(
        btext.contains("sample.py:11"),
        "second call must use B: {btext}"
    );
    assert!(
        !btext.contains("sample.py:9"),
        "earlier assignment leaked: {btext}"
    );
}

#[test]
fn annotation_is_initial_binding_not_override() {
    let repo = temp_repo("annotation-reassign");
    write(
        &repo,
        "sample.py",
        "class A:\n    def run(self): pass\n\nclass B:\n    def run(self): pass\n\ndef go(value: A):\n    value = B()\n    value.run()\n\ndef keep(value: A):\n    value.run()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let go = aw(&repo).args(["callees", "go"]).output().unwrap();
    assert_eq!(go.status.code(), Some(0), "{go:?}");
    let gtext = String::from_utf8_lossy(&go.stdout);
    assert!(gtext.contains("sample.B.run"), "{gtext}");
    assert!(
        !gtext.contains("sample.A.run"),
        "annotation overrode reassignment: {gtext}"
    );
    let keep = aw(&repo).args(["callees", "keep"]).output().unwrap();
    assert_eq!(keep.status.code(), Some(0), "{keep:?}");
    assert!(
        String::from_utf8_lossy(&keep.stdout).contains("sample.A.run"),
        "{keep:?}"
    );
}

#[test]
fn typescript_receiver_annotations_and_construction() {
    let repo = temp_repo("typescript-receivers");
    write(
        &repo,
        "sample.ts",
        "class C { run() {} }\nfunction typed(value: C) { value.run(); }\nfunction constructed() {\n  const value = new C();\n  value.run();\n}\nfunction make(): C { return new C(); }\nfunction annotated() {\n  const value: C = make();\n  value.run();\n}\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let out = aw(&repo).args(["callers", "C.run"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("sample.typed"), "{text}");
    assert!(text.contains("sample.constructed"), "{text}");
    assert!(text.contains("sample.annotated"), "{text}");
}

#[test]
fn rust_struct_literal_and_self_new_type_receiver() {
    let repo = temp_repo("rust-constructors");
    write(
        &repo,
        "lib.rs",
        "struct A { value: i32 }\nstruct B;\nimpl A {\n    fn new() -> Self { Self { value: 0 } }\n    fn render(&self) {}\n    fn go() {\n        let literal = A { value: 1 };\n        literal.render();\n        let constructed = Self::new();\n        constructed.render();\n    }\n}\nimpl B { fn render(&self) {} }\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let a = aw(&repo).args(["callers", "A.render"]).output().unwrap();
    assert_eq!(a.status.code(), Some(0), "{a:?}");
    assert!(
        String::from_utf8_lossy(&a.stdout).contains("lib.A.go"),
        "{a:?}"
    );
    let b = aw(&repo).args(["callers", "B.render"]).output().unwrap();
    assert_eq!(b.status.code(), Some(1), "{b:?}");
}

#[test]
fn local_qualified_import_and_class_receiver() {
    let repo = temp_repo("local-qualified-import");
    write(
        &repo,
        "helpers.py",
        "def run(): pass\n\nclass C:\n    def run(self): pass\n",
    );
    write(
        &repo,
        "app.py",
        "def go():\n    import helpers as h\n    h.run()\n\ndef constructed():\n    from helpers import C\n    value = C()\n    value.run()\n\ndef shadow(h):\n    h.run()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let run = aw(&repo).args(["callers", "helpers.run"]).output().unwrap();
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    let runtext = String::from_utf8_lossy(&run.stdout);
    assert!(runtext.contains("app.go"), "{runtext}");
    assert!(
        !runtext.contains("app.shadow"),
        "parameter shadow leaked: {runtext}"
    );
    let method = aw(&repo).args(["callers", "C.run"]).output().unwrap();
    assert_eq!(method.status.code(), Some(0), "{method:?}");
    assert!(
        String::from_utf8_lossy(&method.stdout).contains("app.constructed"),
        "{method:?}"
    );
}

#[test]
fn typescript_default_export_binds_default_import() {
    let repo = temp_repo("typescript-default");
    write(&repo, "worker.ts", "export default function work() {}\n");
    write(
        &repo,
        "app.ts",
        "import execute from \"./worker\";\nexport function go() { execute(); }\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let out = aw(&repo).args(["callers", "work"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("app.go"), "{text}");
}

#[test]
fn rust_inline_module_aliases_do_not_leak() {
    let src = "mod a { pub fn target() {} }\nmod b { pub fn target() {} }\nmod left {\n    use crate::a::target;\n    pub fn go() { target(); }\n}\nmod right {\n    use crate::b::target;\n    pub fn go() { target(); }\n}\n";
    for (name, file, left_qn, right_qn) in [
        (
            "rust-inline-src",
            "src/lib.rs",
            "src.left.go",
            "src.right.go",
        ),
        ("rust-inline-root", "lib.rs", "left.go", "right.go"),
    ] {
        let repo = temp_repo(name);
        write(&repo, file, src);
        let (code, stdout, stderr) = build(&repo);
        assert_eq!(code, 0, "{name}: {stdout}{stderr}");
        let a = aw(&repo).args(["callers", "a.target"]).output().unwrap();
        assert_eq!(a.status.code(), Some(0), "{name}: {a:?}");
        let atext = String::from_utf8_lossy(&a.stdout);
        assert!(atext.contains(left_qn), "{name}: {atext}");
        assert!(!atext.contains(right_qn), "{name} left leaked: {atext}");
        let b = aw(&repo).args(["callers", "b.target"]).output().unwrap();
        assert_eq!(b.status.code(), Some(0), "{name}: {b:?}");
        let btext = String::from_utf8_lossy(&b.stdout);
        assert!(btext.contains(right_qn), "{name}: {btext}");
        assert!(!btext.contains(left_qn), "{name} right leaked: {btext}");
    }
}

#[test]
fn local_imports_do_not_cross_bindings() {
    let repo = temp_repo("local-imports");
    write(&repo, "a.py", "def f(): pass\n");
    write(&repo, "b.py", "def f(): pass\n");
    write(
        &repo,
        "app.py",
        "def left():\n    from a import f\n    f()\n\ndef right():\n    from b import f\n    f()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let a = aw(&repo).args(["callers", "a.f"]).output().unwrap();
    assert_eq!(a.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&a.stdout).contains("app.left"));
    let b = aw(&repo).args(["callers", "b.f"]).output().unwrap();
    assert_eq!(b.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&b.stdout).contains("app.right"));
}

#[test]
fn parameter_binding_shadows_module_function() {
    let repo = temp_repo("parameter-shadow");
    write(
        &repo,
        "sample.py",
        "def target(): pass\n\ndef run(target):\n    target()\n\ndef run2():\n    target()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let out = aw(&repo).args(["callers", "target"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("sample.run2"), "{stdout}");
    assert!(!stdout.contains("sample.run "), "{stdout}");
    let unresolved = aw(&repo).args(["unresolved", "--names"]).output().unwrap();
    assert!(
        String::from_utf8_lossy(&unresolved.stdout).contains("sample.py:4 call target"),
        "{unresolved:?}"
    );
}

#[test]
fn unknown_reassignment_clears_receiver_type() {
    let repo = temp_repo("receiver-reassign");
    write(
        &repo,
        "sample.py",
        "class A:\n    def m(self): pass\n\ndef run(other):\n    obj = A()\n    obj = other\n    obj.m()\n\ndef run2():\n    obj = A()\n    obj.m()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let out = aw(&repo).args(["callers", "A.m"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("sample.run2"), "{stdout}");
    assert!(!stdout.contains("sample.run "), "{stdout}");
}

#[test]
fn rust_factory_does_not_type_receiver() {
    let repo = temp_repo("rust-factory");
    write(
        &repo,
        "lib.rs",
        "struct A;\nstruct B;\nimpl A {\n    fn make() -> B { B }\n    fn render(&self) {}\n}\nimpl B { fn render(&self) {} }\nfn run() {\n    let value = A::make();\n    value.render();\n}\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let out = aw(&repo).args(["callees", "run"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("lib.A.make"), "{stdout}");
    assert!(!stdout.contains("lib.A.render"), "{stdout}");
}

#[test]
fn arrow_initializer_has_one_owner() {
    let repo = temp_repo("arrow-owner");
    write(
        &repo,
        "sample.ts",
        "function helper() {}\nconst run = () => helper();\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let out = aw(&repo).args(["callers", "helper"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("sample.run"), "{stdout}");
    assert!(
        !stdout.lines().any(|line| line.ends_with(" sample")),
        "outer walk emitted a second owner: {stdout}"
    );
}

#[test]
fn nested_class_methods_keep_class_qualname() {
    let repo = temp_repo("nested-class");
    write(
        &repo,
        "sample.py",
        "def outer():\n    class C:\n        def m(self): pass\n        def run(self):\n            self.m()\n    return C()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let symbols = aw(&repo).args(["symbols", "sample.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&symbols.stdout);
    assert!(stdout.contains("sample.outer.C.m"), "{stdout}");
    assert!(stdout.contains("sample.outer.C.run"), "{stdout}");
    let callees = aw(&repo)
        .args(["callees", "sample.outer.C.run"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&callees.stdout).contains("sample.outer.C.m"),
        "{callees:?}"
    );
}

#[test]
fn local_receiver_blocks_shadowed_module() {
    let repo = temp_repo("receiver-module-shadow");
    write(&repo, "tools.py", "def execute(): pass\n");
    write(
        &repo,
        "app.py",
        "import tools\nfrom external import External\n\nclass C(External):\n    pass\n\ndef run():\n    tools = C()\n    tools.execute()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let callers = aw(&repo)
        .args(["callers", "tools.execute"])
        .output()
        .unwrap();
    assert_eq!(callers.status.code(), Some(1), "{callers:?}");
    let callees = aw(&repo).args(["callees", "run"]).output().unwrap();
    assert!(
        !String::from_utf8_lossy(&callees.stdout).contains("tools.execute"),
        "{callees:?}"
    );
}

#[test]
fn local_binding_blocks_long_module_chain() {
    let repo = temp_repo("long-module-shadow");
    write(&repo, "pkg/mod.py", "def f(): pass\n");
    write(
        &repo,
        "app.py",
        "import pkg.mod\ndef shadowed(pkg):\n    pkg.mod.f()\ndef control():\n    pkg.mod.f()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let shadowed = aw(&repo)
        .args(["callees", "app.shadowed"])
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&shadowed.stdout).contains("pkg.mod.f"),
        "{shadowed:?}"
    );
    let control = aw(&repo).args(["callees", "app.control"]).output().unwrap();
    assert!(
        String::from_utf8_lossy(&control.stdout).contains("pkg.mod.f"),
        "{control:?}"
    );
}

#[test]
fn ambiguous_class_does_not_pick_one_method() {
    let repo = temp_repo("class-ambiguity");
    write(
        &repo,
        "sample.py",
        "if flag:\n    class A:\n        def render(self): pass\nelse:\n    class A:\n        pass\n\ndef run(value: A):\n    value.render()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let out = aw(&repo).args(["callees", "run"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("A.render"));
}

#[test]
fn build_force_bypasses_parse_gate_and_help_says_so() {
    let repo = temp_repo("parse-force");
    write(&repo, "broken.py", "def broken(:\n");
    let strict = aw(&repo).args(["build"]).output().unwrap();
    assert_eq!(strict.status.code(), Some(7), "{strict:?}");
    let forced = aw(&repo).args(["build", "--force"]).output().unwrap();
    assert_eq!(forced.status.code(), Some(0), "{forced:?}");

    let help = aw(&repo).args(["build", "--help"]).output().unwrap();
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(text.contains("parse-error"), "{text}");
    let doctor = aw(&repo).args(["doctor", "--help"]).output().unwrap();
    let text = String::from_utf8_lossy(&doctor.stdout);
    // Contract bump (memory retrieval design): --invalidate resets the
    // derived state and keeps the primary memory.db store.
    assert!(
        text.contains("primary data") && text.contains("memory.db"),
        "{text}"
    );
}

#[test]
fn transitive_callers_use_shortest_depth_per_symbol() {
    let repo = temp_repo("callers-closure");
    write(
        &repo,
        "sample.py",
        "def leaf(): pass\n\ndef self_rec():\n    self_rec()\n\ndef left():\n    leaf()\n\ndef right():\n    left()\n\ndef top():\n    right()\n    leaf()\n",
    );
    build(&repo);
    let out = aw(&repo)
        .args(["callers", "leaf", "--depth", "2"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("sample.left [depth=1]"), "{text}");
    assert!(text.contains("sample.top [depth=1]"), "{text}");
    assert!(text.contains("sample.right [depth=2]"), "{text}");
    assert!(!text.contains("sample.top [depth=2]"), "{text}");
    let self_direct = aw(&repo).args(["callers", "self_rec"]).output().unwrap();
    assert_eq!(self_direct.status.code(), Some(0), "{self_direct:?}");
    assert!(
        String::from_utf8_lossy(&self_direct.stdout).contains("sample.self_rec"),
        "{self_direct:?}"
    );
    // The queried symbol is the closure seed: its self-call is not repeated
    // in transitive rows.
    let self_deep = aw(&repo)
        .args(["callers", "self_rec", "--depth", "2"])
        .output()
        .unwrap();
    assert_eq!(self_deep.status.code(), Some(1), "{self_deep:?}");
    assert!(
        String::from_utf8_lossy(&self_deep.stderr).contains("0 callers found"),
        "{self_deep:?}"
    );
}

#[test]
fn later_assignment_does_not_shadow_earlier_call() {
    let repo = temp_repo("assignment-after-call");
    write(
        &repo,
        "sample.py",
        "def target(): pass\n\ndef go():\n    target()\n    target = None\n",
    );
    build(&repo);
    let callers = aw(&repo).args(["callers", "target"]).output().unwrap();
    assert_eq!(callers.status.code(), Some(0), "{callers:?}");
    assert!(
        String::from_utf8_lossy(&callers.stdout).contains("sample.go"),
        "{callers:?}"
    );
    let callees = aw(&repo).args(["callees", "go"]).output().unwrap();
    assert!(
        String::from_utf8_lossy(&callees.stdout).contains("sample.target"),
        "{callees:?}"
    );
}

#[test]
fn reassignment_updates_receiver_type_ts_and_rust() {
    let ts = temp_repo("ts-reassignment");
    write(
        &ts,
        "sample.ts",
        "class A { run() {} }\nclass B { run() {} }\nfunction go() {\n  let value = new A();\n  value = new B();\n  value.run();\n}\n",
    );
    build(&ts);
    let a = aw(&ts).args(["callers", "A.run"]).output().unwrap();
    assert_eq!(a.status.code(), Some(1), "{a:?}");
    let b = aw(&ts).args(["callers", "B.run"]).output().unwrap();
    assert_eq!(b.status.code(), Some(0), "{b:?}");
    assert!(
        String::from_utf8_lossy(&b.stdout).contains("sample.go"),
        "{b:?}"
    );

    let rs = temp_repo("rust-reassignment");
    write(
        &rs,
        "lib.rs",
        "struct A;\nstruct B;\nimpl A { fn new() -> Self { A } fn render(&self) {} }\nimpl B { fn new() -> Self { B } fn render(&self) {} }\nfn go() {\n    let mut value = A::new();\n    value = B::new();\n    value.render();\n}\n",
    );
    build(&rs);
    let a = aw(&rs).args(["callers", "A.render"]).output().unwrap();
    assert_eq!(a.status.code(), Some(1), "{a:?}");
    let b = aw(&rs).args(["callers", "B.render"]).output().unwrap();
    assert_eq!(b.status.code(), Some(0), "{b:?}");
    assert!(
        String::from_utf8_lossy(&b.stdout).contains("lib.go"),
        "{b:?}"
    );
}

#[test]
fn tuple_and_augmented_assignments_shadow_bare_names() {
    let repo = temp_repo("pattern-shadow");
    write(
        &repo,
        "sample.py",
        "def target(): pass\n\ndef go():\n    target += 1\n    target()\n\ndef go2():\n    target, other = None, None\n    target()\n",
    );
    build(&repo);
    let out = aw(&repo).args(["callers", "target"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "tuple/augmented assignment did not shadow: {out:?}"
    );
}

#[test]
fn nested_arrow_parameter_shadows_only_inside_callback() {
    let repo = temp_repo("nested-arrow");
    write(
        &repo,
        "sample.js",
        "function target() {}\nfunction go() {\n  const cb = (target) => target();\n  target();\n}\n",
    );
    build(&repo);
    let out = aw(&repo).args(["callers", "target"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("sample.js:4"), "outside call lost: {text}");
    assert!(!text.contains("sample.js:3"), "arrow shadow leaked: {text}");
}

#[test]
fn nested_function_inside_arrow_uses_arrow_binding() {
    let repo = temp_repo("nested-arrow-function");
    write(
        &repo,
        "sample.js",
        "function target() {}\nfunction go() {\n  const cb = (target) => {\n    function inner() { target(); }\n    inner();\n  };\n  target();\n}\n",
    );
    build(&repo);
    let out = aw(&repo).args(["callers", "target"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("sample.js:7"), "outside call lost: {text}");
    assert!(
        !text.contains("sample.js:4"),
        "nested function escaped the arrow binding: {text}"
    );
}

#[cfg(unix)]
fn output_with_timeout(mut cmd: std::process::Command, secs: u64) -> Option<std::process::Output> {
    use std::io::Read;
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut out) = child.stdout.take() {
                    out.read_to_string(&mut stdout).ok();
                }
                if let Some(mut err) = child.stderr.take() {
                    err.read_to_string(&mut stderr).ok();
                }
                return Some(std::process::Output {
                    status,
                    stdout: stdout.into_bytes(),
                    stderr: stderr.into_bytes(),
                });
            }
            Ok(None) => {}
            Err(_) => return None,
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(unix)]
#[test]
fn hardlinked_build_lock_does_not_touch_external_file() {
    let repo = temp_repo("hardlink-lock");
    write(&repo, "a.py", "def f(): pass\n");
    let sentinel = std::env::temp_dir().join(format!("aw-sentinel-lock-{}", std::process::id()));
    fs::write(&sentinel, b"keep\n").unwrap();
    fs::create_dir_all(repo.join(".aw")).unwrap();
    fs::hard_link(&sentinel, repo.join(".aw/build.lock")).unwrap();
    let out = aw(&repo).args(["build"]).output().unwrap();
    assert_ne!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fs::read(&sentinel).unwrap(), b"keep\n");
    let _ = fs::remove_file(&sentinel);
}

#[cfg(unix)]
#[test]
fn fifo_journal_reader_does_not_hang() {
    let repo = temp_repo("fifo-reader");
    write(&repo, "a.py", "def f(): pass\n");
    build(&repo);
    fs::remove_file(repo.join(".aw/usage.log")).ok();
    let status = std::process::Command::new("mkfifo")
        .arg(repo.join(".aw/usage.log"))
        .status()
        .unwrap();
    assert!(status.success());
    let mut cmd = aw(&repo);
    cmd.args(["usage"]);
    let out = output_with_timeout(cmd, 5).expect("usage hung on a FIFO journal");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
}

#[cfg(unix)]
#[test]
fn fifo_build_lock_does_not_hang_doctor() {
    let repo = temp_repo("fifo-doctor-lock");
    write(&repo, "a.py", "def f(): pass\n");
    assert_eq!(build(&repo).0, 0);
    let lock = repo.join(".aw/build.lock");
    fs::remove_file(&lock).unwrap();
    assert!(Command::new("mkfifo")
        .arg(&lock)
        .status()
        .unwrap()
        .success());
    let mut cmd = aw(&repo);
    cmd.arg("doctor");
    let out = output_with_timeout(cmd, 5).expect("doctor hung on a FIFO lock");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
}

fn dump_edges(db: &Path) -> String {
    let sql = "SELECT e.kind, e.confidence, e.site_line,
                      sf.path, ss.qualname, ss.line_start,
                      COALESCE(df.path, ''), COALESCE(ds.qualname, e.dst_name),
                      COALESCE(ds.line_start, 0)
               FROM edges e
               JOIN symbols ss ON ss.id = e.src_id
               JOIN files sf ON sf.id = ss.file_id
               LEFT JOIN symbols ds ON ds.id = e.dst_id
               LEFT JOIN files df ON df.id = ds.file_id
               ORDER BY 1, 2, 3, 4, 5, 6, 7, 8, 9";
    let out = Command::new("sqlite3").arg(db).arg(sql).output().unwrap();
    assert!(out.status.success(), "sqlite3 failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn incremental_and_fresh_edge_sets_match_after_removed_inheritance() {
    let repo = temp_repo("removed-base-full-edges");
    write(
        &repo,
        "sample.py",
        "class A:\n    def foo(self):\n        pass\n\nclass B(A):\n    pass\n\ndef invoke(b: B):\n    b.foo()\n\ndef keep(a: A):\n    a.foo()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");

    write(
        &repo,
        "sample.py",
        "class A:\n    def foo(self):\n        pass\n\nclass B:\n    pass\n\ndef invoke(b: B):\n    b.foo()\n\ndef keep(a: A):\n    a.foo()\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let incremental_db = repo.join("incremental.db");
    fs::copy(repo.join(".aw/index.db"), &incremental_db).unwrap();

    let force = aw(&repo).args(["build", "--force"]).output().unwrap();
    assert_eq!(force.status.code(), Some(0), "{force:?}");
    let incremental = dump_edges(&incremental_db);
    let fresh = dump_edges(&repo.join(".aw/index.db"));
    assert_eq!(incremental, fresh, "incremental and fresh edge sets differ");

    let callers = aw(&repo).args(["callers", "A.foo"]).output().unwrap();
    let text = String::from_utf8_lossy(&callers.stdout);
    assert!(text.contains("sample.keep"), "{text}");
    assert!(!text.contains("sample.invoke"), "{text}");
}

#[test]
fn incremental_and_force_agree_on_typed_receiver() {
    // arch-5: a typed receiver call `x.foo()` with `x: b.B` resolves through
    // the MRO. Adding `Mid.foo` moves the answer from `a.A.foo` to
    // `mid.Mid.foo`; the fast path must re-extract caller.py (its type_ref
    // edge into b.py pulls b.py's change into the dirty closure) and agree
    // with `build --force`. No edge-count hardcoding: the type_ref kind adds
    // one edge per typed call site.
    let repo = temp_repo("typed-receiver");
    write(
        &repo,
        "a.py",
        "class A:\n    def foo(self):\n        return 1\n",
    );
    write(
        &repo,
        "mid.py",
        "from a import A\n\nclass Mid(A):\n    pass\n",
    );
    write(
        &repo,
        "b.py",
        "from mid import Mid\n\nclass B(Mid):\n    pass\n",
    );
    write(
        &repo,
        "caller.py",
        "from __future__ import annotations\n\ndef go(x: b.B):\n    return x.foo()\n",
    );
    build(&repo);
    let out = aw(&repo).args(["callees", "go"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("a.A.foo"),
        "MRO hit before Mid.foo exists: {out:?}"
    );

    // Add Mid.foo: the MRO answer changes mid.py -> b.py (inherits) ->
    // caller.py (type_ref) must all re-extract.
    write(
        &repo,
        "mid.py",
        "from a import A\n\nclass Mid(A):\n    def foo(self):\n        return 2\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let incremental = aw(&repo).args(["callees", "go"]).output().unwrap();
    assert_eq!(incremental.status.code(), Some(0), "{incremental:?}");
    let inc_stdout = String::from_utf8_lossy(&incremental.stdout);
    assert!(
        inc_stdout.contains("mid.py:4") && inc_stdout.contains("mid.Mid.foo"),
        "incremental must heal the MRO answer: {inc_stdout}"
    );
    assert!(
        !inc_stdout.contains("a.A.foo"),
        "stale a.A.foo must be gone: {inc_stdout}"
    );

    let incremental_db = repo.join("incremental.db");
    fs::copy(repo.join(".aw/index.db"), &incremental_db).unwrap();
    let force = aw(&repo).args(["build", "--force"]).output().unwrap();
    assert_eq!(force.status.code(), Some(0), "{force:?}");
    let fresh = aw(&repo).args(["callees", "go"]).output().unwrap();
    assert_eq!(fresh.status.code(), Some(0), "{fresh:?}");
    let fresh_stdout = String::from_utf8_lossy(&fresh.stdout);
    assert!(
        fresh_stdout.contains("mid.py:4") && fresh_stdout.contains("mid.Mid.foo"),
        "fresh build answer: {fresh_stdout}"
    );
}

#[test]
fn unrelated_addition_does_not_invalidate_resolved_edges() {
    let repo = temp_repo("added-unrelated");
    write(
        &repo,
        "caller.py",
        "from pkg import foo\n\ndef invoke():\n    foo()\n",
    );
    write(&repo, "pkg.py", "def foo():\n    pass\n");
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");

    write(&repo, "unrelated.py", "def bar():\n    pass\n");
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains("carried=2"),
        "unrelated addition re-extracted carried files: {stdout}"
    );
    let callees = aw(&repo).args(["callees", "invoke"]).output().unwrap();
    let text = String::from_utf8_lossy(&callees.stdout);
    assert!(text.contains("pkg.foo"), "{text}");
    assert!(!text.contains("[ambiguous]"), "{text}");
}

// ---------------------------------------------------------------------------
// Project memory (companion design: 2026-11-05_agent-weave-memory-retrieval).
// Phase 1 acceptance: byte-exact shapes, exit contracts, idempotent replay,
// revision supersession, version/corruption gates, doctor disposition,
// concurrent writers, export round-trip, journal privacy, recall smoke.
// ---------------------------------------------------------------------------

fn mem_write(repo: &Path, key: &str, body: &str, kind: &str) -> (i32, String, String) {
    let out = aw(repo)
        .args(["mem", "write", key, "--body", body, "--kind", kind])
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn mem_out(repo: &Path, args: &[&str]) -> (i32, String, String) {
    let out = aw(repo).args(["mem"]).args(args).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn mem_write_query_get_shapes_and_idempotent_replay() {
    let repo = temp_repo("mem-shapes");
    let (code, stdout, stderr) = mem_write(
        &repo,
        "resolver-tiers",
        "the resolver keeps confidence tiers; AMBIGUOUS edges keep dst_id NULL",
        "decision",
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.starts_with(
            "aw: memory — agent-authored notes, candidates only, never proof of current behavior\n"
        ),
        "caveat must be the stdout first line: {stdout:?}"
    );
    assert_eq!(
        stdout.lines().nth(1),
        Some("ok: 'resolver-tiers' rev 1 written"),
        "{stdout:?}"
    );
    // Idempotent replay: identical content is a successful no-op naming the rev.
    let (code, stdout, _stderr) = mem_write(
        &repo,
        "resolver-tiers",
        "the resolver keeps confidence tiers; AMBIGUOUS edges keep dst_id NULL",
        "decision",
    );
    assert_eq!(code, 0);
    assert_eq!(
        stdout.lines().nth(1),
        Some("ok: 'resolver-tiers' unchanged at rev 1 (identical content)"),
        "{stdout:?}"
    );
    // Ranked query hit.
    let (code, stdout, stderr) = mem_out(&repo, &["query", "confidence", "tiers"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines[0],
        "aw: memory — agent-authored notes, candidates only, never proof of current behavior"
    );
    assert!(
        lines[1].starts_with("resolver-tiers r1 decision durable "),
        "row header is `key rev kind scope created_at`: {:?}",
        lines[1]
    );
    assert_eq!(
        lines[2],
        "  the resolver keeps confidence tiers; AMBIGUOUS edges keep dst_id NULL"
    );
    // Exact-key get with revision count.
    let (code, stdout, _stderr) = mem_out(&repo, &["get", "resolver-tiers"]);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("resolver-tiers r1 decision durable "),
        "{stdout}"
    );
    assert!(stdout.ends_with("revs: 1\n"), "{stdout:?}");
    // Unknown key: exit 1, caveat on stdout, diagnostic on stderr.
    let (code, stdout, stderr) = mem_out(&repo, &["get", "missing-key"]);
    assert_eq!(code, 1);
    assert!(stdout.starts_with("aw: memory — "), "{stdout:?}");
    assert_eq!(
        stderr.trim(),
        "aw: no-results: unknown memory key 'missing-key'",
        "{stderr}"
    );
}

#[test]
fn mem_query_cap_showing_line_and_stderr_notice() {
    let repo = temp_repo("mem-cap");
    for i in 0..15 {
        let (code, stdout, stderr) = mem_write(
            &repo,
            &format!("cap-note-{i:02}"),
            &format!("cap filler note number {i} shares the token filler"),
            "fact",
        );
        assert_eq!(code, 0, "{stdout}{stderr}");
    }
    let (code, stdout, stderr) = mem_out(&repo, &["query", "filler"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    // Caveat + 10 rows + `showing 10 of 15`.
    assert_eq!(
        lines[0],
        "aw: memory — agent-authored notes, candidates only, never proof of current behavior"
    );
    assert_eq!(lines.last(), Some(&"showing 10 of 15"), "{stdout:?}");
    assert_eq!(
        stderr.trim(),
        "[aw] capped at 10 results (15 total)",
        "{stderr}"
    );
    // --limit raises the cap deterministically.
    let (code, stdout, stderr) = mem_out(&repo, &["query", "filler", "--limit", "12"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.ends_with("showing 12 of 15\n"), "{stdout:?}");
    assert_eq!(
        stderr.trim(),
        "[aw] capped at 12 results (15 total)",
        "{stderr}"
    );
    // --all lists by recency with the same cap contract.
    let (code, stdout, _stderr) = mem_out(&repo, &["query", "--all"]);
    assert_eq!(code, 0);
    assert!(stdout.ends_with("showing 10 of 15\n"), "{stdout:?}");
}

#[test]
fn mem_empty_and_missing_store_are_exit1() {
    let repo = temp_repo("mem-empty");
    // No .aw/memory.db at all: the read contract exits 1 with the no-memory class.
    let (code, stdout, stderr) = mem_out(&repo, &["query", "anything"]);
    assert_eq!(code, 1);
    // House convention for a missing store: stdout stays empty, the
    // diagnostic carries the no-memory class on stderr.
    assert!(stdout.is_empty(), "{stdout:?}");
    assert_eq!(
        stderr.trim(),
        format!(
            "aw: no-memory: no .aw/memory.db under {} — write with aw mem write",
            repo.display()
        ),
        "{stderr}"
    );
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "--all"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("no-memory"), "{stderr}");
    // With a store present, a non-matching query is a plain empty result.
    let _ = mem_write(
        &repo,
        "solo",
        "the only note here talks about orbit values",
        "fact",
    );
    let (code, stdout, stderr) = mem_out(&repo, &["query", "zzz-unmatchable"]);
    assert_eq!(code, 1);
    assert!(stdout.starts_with("aw: memory — "), "{stdout:?}");
    assert_eq!(
        stderr.trim(),
        "aw: no-results: no memory matches 'zzz-unmatchable'",
        "{stderr}"
    );
    // Usage errors: --all with terms, whitespace-only terms.
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "--all", "terms"]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("mutually exclusive"), "{stderr}");
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "   "]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("provide terms or --all"), "{stderr}");
    // Bare query without terms defaults to --all listing.
    let (code, stdout, stderr) = mem_out(&repo, &["query"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("solo"), "{stdout}");
}

#[test]
fn mem_revision_supersession_and_delete() {
    let repo = temp_repo("mem-revs");
    let _ = mem_write(&repo, "k", "body one", "decision");
    let (code, stdout, _stderr) = mem_write(&repo, "k", "body two", "decision");
    assert_eq!(code, 0);
    assert_eq!(
        stdout.lines().nth(1),
        Some("ok: 'k' rev 2 written (supersedes rev 1)"),
        "{stdout:?}"
    );
    let (code, stdout, _stderr) = mem_out(&repo, &["get", "k"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("k r2 decision durable "), "{stdout}");
    assert!(stdout.contains("  body two"), "{stdout}");
    assert!(stdout.ends_with("revs: 2\n"), "{stdout:?}");
    // Delete removes every revision; the FTS rows go with it (query misses).
    let (code, stdout, _stderr) = mem_out(&repo, &["delete", "k"]);
    assert_eq!(code, 0);
    assert_eq!(
        stdout.lines().nth(1),
        Some("deleted 'k' revs=2"),
        "{stdout:?}"
    );
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "body"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("no memory matches"), "{stderr}");
    let (code, _stdout, stderr) = mem_out(&repo, &["delete", "k"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("unknown memory key 'k'"), "{stderr}");
    // Deleting with no store at all must not conjure one.
    let empty = temp_repo("mem-delete-empty");
    let (code, _stdout, stderr) = mem_out(&empty, &["delete", "k"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("no-memory"), "{stderr}");
    assert!(!empty.join(".aw/memory.db").exists());
}

#[test]
fn mem_write_usage_rejections_are_exit2() {
    let repo = temp_repo("mem-usage");
    // Unknown kind (closed taxonomy).
    let (code, _stdout, stderr) = mem_write(&repo, "k", "body", "mood");
    assert_eq!(code, 2);
    assert!(
        stderr.contains("kind must be one of decision|gotcha|fact|task"),
        "{stderr}"
    );
    // Phase 2: session scope is now accepted; unknown scope still exits 2.
    let out = aw(&repo)
        .args([
            "mem", "write", "k", "--body", "b", "--kind", "fact", "--scope", "orbit",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("scope must be 'durable' or 'session'"));
    // TTL without the session lane.
    let out = aw(&repo)
        .args([
            "mem", "write", "k", "--body", "b", "--kind", "fact", "--ttl", "60",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--ttl is only valid with --scope session"),
    );
    // Body cap.
    let big = "x".repeat(8193);
    let (code, _stdout, stderr) = mem_write(&repo, "k", &big, "fact");
    assert_eq!(code, 2);
    assert!(stderr.contains("body exceeds 8192 bytes"), "{stderr}");
    // Key cap and control characters.
    let (code, _stdout, stderr) = mem_write(&repo, &"k".repeat(201), "b", "fact");
    assert_eq!(code, 2);
    assert!(stderr.contains("key exceeds 200 bytes"), "{stderr}");
    let (code, _stdout, stderr) = mem_write(&repo, "bad\nkey", "b", "fact");
    assert_eq!(code, 2);
    assert!(stderr.contains("control characters"), "{stderr}");
    // Empty body.
    let (code, _stdout, stderr) = mem_write(&repo, "k", "   ", "fact");
    assert_eq!(code, 2);
    assert!(stderr.contains("body must not be empty"), "{stderr}");
    // Nothing was written by the rejected calls.
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "--all"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("no .aw/memory.db"), "{stderr}");
}

#[test]
fn mem_schema_gates_newer_incomplete_and_corrupt() {
    use rusqlite::Connection;
    // Newer than the binary -> exit 5, export-backup message.
    let repo = temp_repo("mem-newer");
    let _ = mem_write(&repo, "k", "b", "fact");
    let conn = Connection::open(repo.join(".aw/memory.db")).unwrap();
    conn.execute_batch("PRAGMA user_version = 99").unwrap();
    drop(conn);
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "--all"]);
    assert_eq!(code, 5);
    assert!(
        stderr.contains("memory.db schema v99 is newer than the binary expects (v1)"),
        "{stderr}"
    );
    assert!(
        stderr.contains("restore from an aw mem export backup"),
        "{stderr}"
    );

    // Older but failing the compiled manifest -> the other exit-5 message.
    let repo = temp_repo("mem-incomplete");
    let _ = mem_write(&repo, "k", "b", "fact");
    let conn = Connection::open(repo.join(".aw/memory.db")).unwrap();
    conn.execute_batch("PRAGMA user_version = 0; DROP TABLE memories_fts;")
        .unwrap();
    drop(conn);
    let (code, _stdout, stderr) = mem_out(&repo, &["get", "k"]);
    assert_eq!(code, 5);
    assert!(
        stderr.contains("is missing required structure for this binary"),
        "{stderr}"
    );

    // Truncated / non-SQLite file -> exit 3.
    let repo = temp_repo("mem-corrupt");
    std::fs::create_dir_all(repo.join(".aw")).unwrap();
    std::fs::write(repo.join(".aw/memory.db"), b"this is not sqlite at all").unwrap();
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "--all"]);
    assert_eq!(code, 3);
    assert!(stderr.contains("corrupt-memory"), "{stderr}");
}

#[test]
fn mem_works_without_index_db_and_survives_rebuilds() {
    let repo = temp_repo("mem-no-index");
    // No build ever ran: mem write + query work alone.
    let (code, stdout, stderr) =
        mem_write(&repo, "standalone", "memory outlives the index", "fact");
    assert_eq!(code, 0, "{stdout}{stderr}");
    let (code, stdout, _stderr) = mem_out(&repo, &["query", "memory outlives"]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("standalone r1"), "{stdout}");
    assert!(
        !repo.join(".aw/index.db").exists(),
        "no index.db was created"
    );
    // And it still answers after a full build + invalidate cycle.
    let (code, _stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stderr}");
    let (code, _stdout, stderr) = mem_out(&repo, &["get", "standalone"]);
    assert_eq!(code, 0, "{stderr}");
}

#[test]
fn mem_doctor_invalidate_preserves_memory() {
    let repo = temp_repo("mem-invalidate");
    let _ = mem_write(&repo, "keeper", "memory survives the reset", "decision");
    let (code, _stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stderr}");
    let out = aw(&repo).args(["doctor", "--invalidate"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("kept primary memory.db"), "{text}");
    assert!(
        repo.join(".aw/memory.db").exists(),
        "memory.db must survive --invalidate"
    );
    assert!(
        !repo.join(".aw/index.db").exists(),
        "the derived index is reset"
    );
    // --help documents the new reset scope (contract bump).
    let help = aw(&repo).args(["doctor", "--help"]).output().unwrap();
    let help_text = String::from_utf8_lossy(&help.stdout);
    assert!(
        help_text.contains("memory.db") && help_text.contains("primary data"),
        "--help must document the kept store: {help_text}"
    );
    // Memory still answers; journal/report rows are gone with the index.
    let (code, stdout, stderr) = mem_out(&repo, &["query", "memory survives"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("keeper"), "{stdout}");
    // Doctor report carries the memory line (the index was reset above, so
    // rebuild first — the report opens the index).
    let (code, _stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stderr}");
    let out = aw(&repo).args(["doctor"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("memory: integrity ok, 1 notes"), "{text}");
}

#[test]
fn mem_concurrent_writers_serialize_or_exit4_never_unique() {
    // (a) Same key, different bodies: the loser either lands as the next rev
    // (busy_timeout serialization) or exits 4 — never a UNIQUE(key, rev) hit.
    let repo = temp_repo("mem-race-same");
    let _ = mem_write(&repo, "shared", "the original body", "fact");
    let ra_repo = repo.clone();
    let a = std::thread::spawn(move || mem_write(&ra_repo, "shared", "writer A body", "fact"));
    let rb_repo = repo.clone();
    let b = std::thread::spawn(move || mem_write(&rb_repo, "shared", "writer B body", "fact"));
    let (ra, rb) = (a.join().unwrap(), b.join().unwrap());
    for (code, _stdout, stderr) in [&ra, &rb] {
        assert!(
            *code == 0 || *code == 4,
            "loser must serialize (0) or exit 4, got {code}: {stderr}"
        );
        assert!(
            !stderr.contains("UNIQUE"),
            "constraint leaks are defects: {stderr}"
        );
    }
    let (code, stdout, _stderr) = mem_out(&repo, &["get", "shared"]);
    assert_eq!(code, 0);
    let revs: usize = stdout
        .lines()
        .last()
        .and_then(|l| l.strip_prefix("revs: "))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let expected = 1 + [&ra, &rb].iter().filter(|r| r.0 == 0).count();
    assert_eq!(
        revs, expected,
        "every successful write is one rev: {stdout}"
    );

    // (b) Different keys share SQLite's single writer, so under load one may
    // still serialize (0) or hit the busy timeout (4). What must never happen
    // is a UNIQUE leak, and at least one write must land.
    let repo = temp_repo("mem-race-distinct");
    let ra = {
        let repo = repo.clone();
        std::thread::spawn(move || mem_write(&repo, "left", "left body", "fact"))
    };
    let rb = {
        let repo = repo.clone();
        std::thread::spawn(move || mem_write(&repo, "right", "right body", "fact"))
    };
    let (ra, rb) = (ra.join().unwrap(), rb.join().unwrap());
    for (code, _stdout, stderr) in [&ra, &rb] {
        assert!(
            *code == 0 || *code == 4,
            "distinct-key writer must serialize (0) or exit 4, got {code}: {stderr}"
        );
        assert!(
            !stderr.contains("UNIQUE"),
            "constraint leaks are defects: {stderr}"
        );
    }
    // A writer that hit the busy timeout left no row; retry it once so the
    // end state does not depend on machine load. The concurrency invariant
    // (no UNIQUE leak) was already checked on the racing attempts.
    if ra.0 != 0 {
        let retry = mem_write(&repo, "left", "left body", "fact");
        assert_eq!(retry.0, 0, "retry left: {}{}", retry.1, retry.2);
    }
    if rb.0 != 0 {
        let retry = mem_write(&repo, "right", "right body", "fact");
        assert_eq!(retry.0, 0, "retry right: {}{}", retry.1, retry.2);
    }
    for key in ["left", "right"] {
        let (code, _stdout, stderr) = mem_out(&repo, &["get", key]);
        assert_eq!(code, 0, "{stderr}");
    }
}

#[test]
fn mem_export_round_trip_restores_the_note_set() {
    let repo = temp_repo("mem-export-src");
    let notes = [
        (
            "alpha-decision",
            "decision",
            "settled: alpha path",
            "tags alpha",
            "src/one.rs",
        ),
        (
            "beta-gotcha",
            "gotcha",
            "never do beta; it exits 5",
            "",
            "src/two.rs src/three.rs",
        ),
        (
            "gamma-fact",
            "fact",
            "gamma notes with a much longer body about retrieval behavior",
            "tags gamma",
            "",
        ),
    ];
    for (key, kind, body, tags, refs) in notes {
        let mut args = vec!["write", key, "--body", body, "--kind", kind];
        if !tags.is_empty() {
            args.push("--tags");
            args.push(tags);
        }
        if !refs.is_empty() {
            args.push("--refs");
            args.push(refs);
        }
        let (code, _stdout, stderr) = mem_out(&repo, &args);
        assert_eq!(code, 0, "{stderr}");
    }
    // Supersede one note so the export carries only current revs.
    let _ = mem_write(
        &repo,
        "alpha-decision",
        "settled: alpha path v2",
        "decision",
    );

    let (code, stdout, stderr) = mem_out(&repo, &["export"]);
    assert_eq!(code, 0, "{stderr}");
    // stdout is PURE JSONL: the caveat travels on stderr.
    assert!(
        !stdout.contains("aw: memory"),
        "export stdout must not carry the caveat: {stdout:?}"
    );
    assert!(stderr.starts_with("aw: memory — "), "{stderr:?}");
    let lines: Vec<serde_json::Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).expect("pure JSONL"))
        .collect();
    assert_eq!(lines.len(), 3, "{stdout}");
    let fingerprint = |rows: &[serde_json::Value]| -> Vec<String> {
        let mut v: Vec<String> = rows
            .iter()
            .map(|r| {
                format!(
                    "{}|{}|{}|{}|{}|{}",
                    r["key"].as_str().unwrap(),
                    r["kind"].as_str().unwrap(),
                    r["scope"].as_str().unwrap(),
                    r["body"].as_str().unwrap(),
                    r["tags"].as_str().unwrap(),
                    r["refs"].as_str().unwrap(),
                )
            })
            .collect();
        v.sort();
        v
    };
    let original = fingerprint(&lines);

    // Restore into a fresh store by replaying mem write per line.
    let fresh = temp_repo("mem-export-fresh");
    for r in &lines {
        let mut args = vec![
            "write",
            r["key"].as_str().unwrap(),
            "--body",
            r["body"].as_str().unwrap(),
            "--kind",
            r["kind"].as_str().unwrap(),
        ];
        let tags = r["tags"].as_str().unwrap();
        let refs = r["refs"].as_str().unwrap();
        if !tags.is_empty() {
            args.push("--tags");
            args.push(tags);
        }
        if !refs.is_empty() {
            args.push("--refs");
            args.push(refs);
        }
        let (code, _stdout, stderr) = mem_out(&fresh, &args);
        assert_eq!(code, 0, "{stderr}");
    }
    let (code, stdout2, stderr) = mem_out(&fresh, &["export"]);
    assert_eq!(code, 0, "{stderr}");
    let restored: Vec<serde_json::Value> = stdout2
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(original, fingerprint(&restored), "restore is content-exact");
    // --limit takes the N most recent, deterministically.
    let (code, stdout3, _stderr) = mem_out(&repo, &["export", "--limit", "1"]);
    assert_eq!(code, 0);
    let limited: Vec<serde_json::Value> = stdout3
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(limited.len(), 1);
    // Empty store: exit 1, nothing on stdout.
    let bare = temp_repo("mem-export-empty");
    let _ = mem_write(&bare, "temp", "note", "fact");
    let _ = mem_out(&bare, &["delete", "temp"]);
    let (code, stdout4, stderr) = mem_out(&bare, &["export"]);
    assert_eq!(code, 1);
    assert!(stdout4.is_empty(), "{stdout4:?}");
    assert!(stderr.contains("no current notes to export"), "{stderr}");
}

#[test]
fn mem_journal_records_len_never_body() {
    let repo = temp_repo("mem-journal");
    let secret_body = "the resolver keeps confidence tiers; AMBIGUOUS edges keep dst_id NULL";
    let (code, _stdout, stderr) = mem_write(&repo, "secret-check", secret_body, "decision");
    assert_eq!(code, 0, "{stderr}");
    let _ = mem_out(&repo, &["query", "confidence"]);
    let log = std::fs::read_to_string(repo.join(".aw/usage.log")).unwrap();
    assert!(log.contains("\"tool\":\"mem_write\""), "{log}");
    assert!(log.contains("\"tool\":\"mem_query\""), "{log}");
    assert!(
        log.contains("body_len"),
        "journal args carry body_len: {log}"
    );
    assert!(
        !log.contains("confidence tiers"),
        "journal must never carry the body: {log}"
    );
    // usage --quality shows the mem tools and the query empty rate.
    let out = aw(&repo).args(["usage", "--quality"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("mem_write:"), "{text}");
    assert!(text.contains("mem_query:"), "{text}");
}

#[test]
fn mem_recall_smoke_oracle_reaches_threshold() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/memory/oracle.json")).unwrap();
    let repo = temp_repo("mem-oracle");
    for note in fixture["notes"].as_array().unwrap() {
        let (code, _stdout, stderr) = mem_out(
            &repo,
            &[
                "write",
                note["key"].as_str().unwrap(),
                "--body",
                note["body"].as_str().unwrap(),
                "--kind",
                note["kind"].as_str().unwrap(),
                "--tags",
                note["tags"].as_str().unwrap(),
                "--refs",
                note["refs"].as_str().unwrap(),
            ],
        );
        assert_eq!(code, 0, "{stderr}");
    }
    let queries = fixture["queries"].as_array().unwrap();
    let mut hits = 0usize;
    for q in queries {
        let (code, stdout, stderr) = mem_out(
            &repo,
            &["query", q["terms"].as_str().unwrap(), "--limit", "3"],
        );
        assert_eq!(code, 0, "{} {stderr}", q["terms"].as_str().unwrap());
        let expected = q["expect"].as_str().unwrap();
        if stdout.contains(expected) {
            hits += 1;
        }
    }
    let recall = hits as f64 / queries.len() as f64;
    assert!(
        recall >= 0.8,
        "recall@3 = {recall} ({hits}/{}), below the 0.8 smoke threshold",
        queries.len()
    );
}

// ---------------------------------------------------------------------------
// Phase 2 — lifecycle: session lane + TTL, gc purge, --supersedes,
// --include-superseded, determinism under replay.
// ---------------------------------------------------------------------------

#[test]
fn mem_session_ttl_default_and_durable_ttl_rejection() {
    let repo = temp_repo("mem-session-ttl");
    // Session without --ttl: expires_at = created_at + 604800 (OQ2 default).
    let (code, _stdout, stderr) = mem_out(
        &repo,
        &[
            "write",
            "session-note",
            "--body",
            "transient scratch work",
            "--kind",
            "fact",
            "--scope",
            "session",
        ],
    );
    assert_eq!(code, 0, "{stderr}");
    let (code, stdout, _stderr) = mem_out(&repo, &["export"]);
    assert_eq!(code, 0);
    let row: serde_json::Value = serde_json::from_str(stdout.lines().next().unwrap()).unwrap();
    assert_eq!(row["scope"], "session");
    let ttl = row["expires_at"].as_i64().unwrap() - row["created_at"].as_i64().unwrap();
    assert_eq!(ttl, 604800, "OQ2 default session TTL is 7 days: {row}");
    // Explicit --ttl wins.
    let _ = mem_out(
        &repo,
        &[
            "write",
            "session-short",
            "--body",
            "short lived",
            "--kind",
            "fact",
            "--scope",
            "session",
            "--ttl",
            "60",
        ],
    );
    let (code, stdout, _stderr) = mem_out(&repo, &["export"]);
    assert_eq!(code, 0);
    let rows: Vec<serde_json::Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let short = rows.iter().find(|r| r["key"] == "session-short").unwrap();
    assert_eq!(
        short["expires_at"].as_i64().unwrap() - short["created_at"].as_i64().unwrap(),
        60
    );
    // Durable + --ttl is a usage error; durable rows carry expires_at = 0.
    let (code, _stdout, stderr) = mem_out(
        &repo,
        &[
            "write",
            "durable-ttl",
            "--body",
            "b",
            "--kind",
            "fact",
            "--ttl",
            "60",
        ],
    );
    assert_eq!(code, 2);
    assert!(
        stderr.contains("--ttl is only valid with --scope session"),
        "{stderr}"
    );
    let durable = rows.iter().find(|r| r["key"] == "session-note").unwrap();
    assert!(durable["scope"] == "session");
}

#[test]
fn mem_gc_cli_purges_seeded_expired_and_reports_counts() {
    use rusqlite::Connection;
    let repo = temp_repo("mem-gc-cli");
    let _ = mem_write(&repo, "keeper", "durable note", "fact");
    let _ = mem_out(
        &repo,
        &[
            "write",
            "session-note",
            "--body",
            "expired scratch",
            "--kind",
            "fact",
            "--scope",
            "session",
            "--ttl",
            "60",
        ],
    );
    // Inject an expired deadline for the session note (injected clock).
    let conn = Connection::open(repo.join(".aw/memory.db")).unwrap();
    conn.execute(
        "UPDATE memories SET expires_at = 1 WHERE key = 'session-note'",
        [],
    )
    .unwrap();
    drop(conn);
    let (code, stdout, stderr) = mem_out(&repo, &["gc"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        stdout.lines().nth(1),
        Some("expired=1 removed, superseded=0 removed, durable=1 rows"),
        "{stdout:?}"
    );
    // The expired row and its FTS entry are gone: the query misses it.
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "expired scratch"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("no memory matches"), "{stderr}");
    // gc with no store at all must not conjure one.
    let bare = temp_repo("mem-gc-missing");
    let (code, _stdout, stderr) = mem_out(&bare, &["gc"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("no-memory"), "{stderr}");
    assert!(!bare.join(".aw/memory.db").exists());
}

#[test]
fn mem_supersedes_chain_cli_contract() {
    let repo = temp_repo("mem-supersedes-cli");
    let _ = mem_write(
        &repo,
        "old-conclusion",
        "the old way of doing things",
        "decision",
    );
    let (code, stdout, stderr) = mem_out(
        &repo,
        &[
            "write",
            "new-conclusion",
            "--body",
            "the new way, measured",
            "--kind",
            "decision",
            "--supersedes",
            "old-conclusion",
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert_eq!(
        stdout.lines().nth(1),
        Some("ok: 'new-conclusion' rev 1 written"),
        "{stdout:?}"
    );
    // Default listing hides the superseded note; --include-superseded shows it tagged.
    let (code, stdout, _stderr) = mem_out(&repo, &["query", "--all"]);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("new-conclusion") && !stdout.contains("old-conclusion"),
        "{stdout}"
    );
    let (code, stdout, _stderr) = mem_out(&repo, &["query", "--all", "--include-superseded"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("old-conclusion"), "{stdout}");
    let superseded_line = stdout
        .lines()
        .find(|l| l.starts_with("old-conclusion"))
        .unwrap();
    assert!(
        superseded_line.ends_with(" [superseded]"),
        "{superseded_line}"
    );
    // --include-superseded without --all is a usage error.
    let (code, _stdout, stderr) = mem_out(&repo, &["query", "measured", "--include-superseded"]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains("--include-superseded requires --all"),
        "{stderr}"
    );
    // Unknown target: exit 1 and no new note.
    let (code, _stdout, stderr) = mem_out(
        &repo,
        &[
            "write",
            "never-lands",
            "--body",
            "x",
            "--kind",
            "fact",
            "--supersedes",
            "missing",
        ],
    );
    assert_eq!(code, 1);
    assert!(stderr.contains("unknown memory key 'missing'"), "{stderr}");
    let (code, _stdout, _stderr) = mem_out(&repo, &["get", "never-lands"]);
    assert_eq!(code, 1, "the failed supersedes write must leave no row");
    // Same key as the new note: exit 2 (same-key updates go through revs).
    let (code, _stdout, stderr) = mem_out(
        &repo,
        &[
            "write",
            "self",
            "--body",
            "x",
            "--kind",
            "fact",
            "--supersedes",
            "self",
        ],
    );
    assert_eq!(code, 2);
    assert!(stderr.contains("different key"), "{stderr}");
}

#[test]
fn mem_query_output_is_deterministic_under_replay() {
    let repo = temp_repo("mem-determinism");
    let writes: [(&str, &str, &str, &str); 3] = [
        (
            "alpha",
            "alpha body about caching rows",
            "fact",
            "cache rows",
        ),
        (
            "beta",
            "beta body about resolver passes",
            "gotcha",
            "resolver passes",
        ),
        (
            "gamma",
            "gamma body about build timing",
            "decision",
            "build timing",
        ),
    ];
    let run_queries = || {
        let mut outs = Vec::new();
        for (terms, kind) in [
            ("caching rows", "--kind fact"),
            ("resolver passes", "--kind gotcha"),
            ("build timing", "--kind decision"),
        ] {
            let kind_flag = if kind == "--kind fact" {
                vec!["--kind", "fact"]
            } else if kind == "--kind gotcha" {
                vec!["--kind", "gotcha"]
            } else {
                vec!["--kind", "decision"]
            };
            let (code, stdout, stderr) =
                mem_out(&repo, &[&["query", terms][..], &kind_flag[..]].concat());
            assert_eq!(code, 0, "{stdout}{stderr}");
            outs.push(stdout);
        }
        outs
    };
    for (key, body, kind, tags) in writes {
        let (code, _stdout, stderr) = mem_out(
            &repo,
            &["write", key, "--body", body, "--kind", kind, "--tags", tags],
        );
        assert_eq!(code, 0, "{stderr}");
    }
    let first = run_queries();
    // Replay the exact write sequence: every write is an idempotent no-op,
    // and the query output is byte-identical.
    for (key, body, kind, tags) in writes {
        let (code, stdout, _stderr) = mem_out(
            &repo,
            &["write", key, "--body", body, "--kind", kind, "--tags", tags],
        );
        assert_eq!(code, 0);
        assert!(
            stdout.contains("unchanged at rev 1"),
            "replay must be a no-op: {stdout}"
        );
    }
    let second = run_queries();
    assert_eq!(first, second, "same DB + same queries -> same bytes");
}

#[test]
fn reuse_empty_reports_zero_candidates() {
    let repo = temp_repo("reuse-empty");
    write(&repo, "a.py", "def unique_one():\n    return 1\n");
    build(&repo);
    let out = aw(&repo).args(["reuse"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("0 reuse candidates found"), "{stdout}");
}

#[test]
fn reuse_detects_candidate_duplicates() {
    let repo = temp_repo("reuse-dupes");
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
    let out = aw(&repo).args(["reuse"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("duplication candidate(s) found"),
        "{stdout}"
    );
    assert!(stdout.contains("parse_token"), "{stdout}");
    // Verify telemetry recorded the duplication warning
    let usage = aw(&repo).args(["usage", "--quality"]).output().unwrap();
    let usage_stdout = String::from_utf8_lossy(&usage.stdout);
    assert!(
        usage_stdout.contains("Anti-Reinvention: 1 duplication warnings caught"),
        "{usage_stdout}"
    );
}

#[test]
fn search_outputs_interface_cards_and_callers_ranking() {
    let repo = temp_repo("search-cards");
    write(
        &repo,
        "util.py",
        "def popular_helper(x: int) -> int:\n    return x + 1\n\ndef lone_helper(x: int) -> int:\n    return x + 2\n",
    );
    write(
        &repo,
        "main.py",
        "from util import popular_helper\ndef run():\n    return popular_helper(1) + popular_helper(2)\n",
    );
    build(&repo);
    let out = aw(&repo).args(["search", "helper"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains(
            "util.py:1 function util.popular_helper def popular_helper(x: int) -> int [callers=2]"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "util.py:4 function util.lone_helper def lone_helper(x: int) -> int [callers=0]"
        ),
        "{stdout}"
    );
    let pop_idx = stdout.find("util.popular_helper").unwrap();
    let lone_idx = stdout.find("util.lone_helper").unwrap();
    assert!(
        pop_idx < lone_idx,
        "popular helper must rank ahead: {stdout}"
    );
}

#[test]
fn incremental_build_emits_reuse_hint_on_stderr() {
    let repo = temp_repo("build-reuse-hint");
    write(
        &repo,
        "core.py",
        "def transform_data(data: str):\n    return data.strip().lower()\n",
    );
    build(&repo);

    write(
        &repo,
        "extra.py",
        "def transform_data(data: str):\n    return data.strip().lower()\n",
    );
    let (code, _stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stderr.contains("aw: reuse-hint:"),
        "expected reuse hint on stderr: {stderr}"
    );
    assert!(stderr.contains("transform_data"), "{stderr}");
}

#[test]
fn reuse_scoped_path_filters_targets_and_no_duplicate_pairs() {
    let repo = temp_repo("reuse-scope-dedup");
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

    // Unscoped: exactly 1 candidate found (not duplicated as A->B and B->A)
    let out = aw(&repo).args(["reuse"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("aw: reuse — 1 duplication candidate(s) found"),
        "must report exactly 1 candidate pair: {stdout}"
    );

    // Scoped to duplicate file b.py: reports candidate
    let out_b = aw(&repo).args(["reuse", "b.py"]).output().unwrap();
    let stdout_b = String::from_utf8_lossy(&out_b.stdout);
    assert_eq!(out_b.status.code(), Some(0), "{stdout_b}");
    assert!(
        stdout_b.contains("aw: reuse — 1 duplication candidate(s) found"),
        "must report b.py duplicate: {stdout_b}"
    );

    // Scoped to canonical file a.py: zero candidates (a.py is not duplicating anything)
    let out_a = aw(&repo).args(["reuse", "a.py"]).output().unwrap();
    let stdout_a = String::from_utf8_lossy(&out_a.stdout);
    assert_eq!(out_a.status.code(), Some(0), "{stdout_a}");
    assert!(
        stdout_a.contains("0 reuse candidates found"),
        "canonical a.py must not be reported as a duplicate: {stdout_a}"
    );
}

#[test]
fn reuse_d1_callee_footprint_requires_resolved_callees() {
    let repo = temp_repo("reuse-d1");
    write(
        &repo,
        "core.py",
        "def helper1():\n    return 1\ndef helper2():\n    return 2\ndef helper3():\n    return 3\ndef canonical_pipeline():\n    return helper1() + helper2() + helper3()\n",
    );
    write(
        &repo,
        "main.py",
        "from core import canonical_pipeline\ndef run():\n    return canonical_pipeline() + canonical_pipeline()\n",
    );
    write(
        &repo,
        "dup.py",
        "from core import helper1, helper2, helper3\ndef duplicate_pipeline():\n    return helper1() + helper2() + helper3()\n",
    );
    build(&repo);

    let out = aw(&repo).args(["reuse"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("duplicate_pipeline"),
        "must identify duplicate_pipeline: {stdout}"
    );
    assert!(
        stdout.contains("callee footprint"),
        "must be detected via D1 callee footprint: {stdout}"
    );
    assert!(
        stdout.contains("shared callees: [helper1, helper2, helper3]"),
        "must list shared resolved callees: {stdout}"
    );
}

fn mem_out_global(repo: &Path, global_dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = aw(repo)
        .env("AW_GLOBAL_DIR", global_dir)
        .args(["mem"])
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn global_memory_write_and_local_layered_query_with_global_tag() {
    let repo = temp_repo("mem-global-layered");
    let global_dir = temp_repo("global-aw-store");

    // 1. Write a global note
    let (code, stdout, stderr) = mem_out_global(
        &repo,
        &global_dir,
        &[
            "write",
            "developer_profile",
            "--body",
            "Preferred stack: Rust. Style: Minimal.",
            "--kind",
            "decision",
            "--global",
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("ok: 'developer_profile' rev 1 written"));

    // Verify .aw/memory.db was NOT created in repo root, but memory.db WAS created in global_dir
    assert!(!repo.join(".aw/memory.db").exists());
    assert!(global_dir.join("memory.db").exists());

    // 2. Query from inside repo without flags: layered search finds global note and tags it [global]
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["query", "stack"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("developer_profile r1 decision durable "));
    assert!(stdout.contains(" [global]"));
    assert!(stdout.contains("Preferred stack: Rust. Style: Minimal."));

    // 3. Query --all also lists the global note with [global] tag
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["query", "--all"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("developer_profile r1 decision durable "));
    assert!(stdout.contains(" [global]"));

    // 4. Get from repo without flags falls back to global and tags with [global]
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["get", "developer_profile"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("developer_profile r1 decision durable "));
    assert!(stdout.contains(" [global]"));
    assert!(stdout.contains("revs: 1"));

    // 5. Write a local note with the SAME key
    let (code, stdout, stderr) = mem_out_global(
        &repo,
        &global_dir,
        &[
            "write",
            "developer_profile",
            "--body",
            "Repo override: Python stack.",
            "--kind",
            "decision",
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(repo.join(".aw/memory.db").exists());

    // 6. Query from repo without flags: local shadows global!
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["query", "--all"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("Repo override: Python stack."));
    assert!(!stdout.contains("[global]"));
    assert!(!stdout.contains("Preferred stack: Rust."));

    // 7. Ranked query also shows local note shadowing global note
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["query", "developer"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("Repo override: Python stack."));
    assert!(!stdout.contains("[global]"));

    // 8. Get from repo returns the local note
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["get", "developer_profile"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("Repo override: Python stack."));
    assert!(!stdout.contains("[global]"));

    // 9. Query with --global explicitly targets global store, returning the global note
    let (code, stdout, stderr) =
        mem_out_global(&repo, &global_dir, &["query", "--all", "--global"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("Preferred stack: Rust. Style: Minimal."));
    assert!(stdout.contains(" [global]"));
    assert!(!stdout.contains("Repo override"));

    // 10. Delete --global deletes from global store
    let (code, stdout, stderr) = mem_out_global(
        &repo,
        &global_dir,
        &["delete", "developer_profile", "--global"],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("deleted 'developer_profile' revs=1"));

    // Global query now returns empty/no-results
    let (code, _, _) = mem_out_global(&repo, &global_dir, &["query", "--all", "--global"]);
    assert_eq!(code, 1);

    // Local note is untouched!
    let (code, stdout, _) = mem_out_global(&repo, &global_dir, &["get", "developer_profile"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("Repo override: Python stack."));
}

#[test]
fn global_memory_export_and_gc() {
    let repo = temp_repo("mem-global-export-gc");
    let global_dir = temp_repo("global-aw-store-export");

    let (code, stdout, stderr) = mem_out_global(
        &repo,
        &global_dir,
        &[
            "write",
            "global_item",
            "--body",
            "item body for export",
            "--kind",
            "fact",
            "--global",
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");

    // Export with --global
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["export", "--global"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("\"key\":\"global_item\""), "{stdout}");
    assert!(
        stdout.contains("\"body\":\"item body for export\""),
        "{stdout}"
    );

    // GC with --global
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["gc", "--global"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains("expired=0 removed, superseded=0 removed, durable=1 rows"),
        "{stdout}"
    );
}

#[test]
fn global_memory_read_never_creates_missing_directory() {
    let repo = temp_repo("mem-no-create-read");
    let non_existent_global = repo.join("definitely-does-not-exist-global-dir");
    assert!(!non_existent_global.exists());

    // A read-only query must exit 1 and never create the directory on disk
    let (code, _, _) = mem_out_global(&repo, &non_existent_global, &["query", "test"]);
    assert_eq!(code, 1);
    assert!(
        !non_existent_global.exists(),
        "query must not create missing global directory"
    );

    let (code, _, _) = mem_out_global(&repo, &non_existent_global, &["get", "test"]);
    assert_eq!(code, 1);
    assert!(
        !non_existent_global.exists(),
        "get must not create missing global directory"
    );
}

#[test]
fn global_memory_expired_local_does_not_shadow_global() {
    let repo = temp_repo("mem-expired-shadow");
    let global_dir = temp_repo("global-aw-expired-shadow");

    // Global note
    let (code, _, _) = mem_out_global(
        &repo,
        &global_dir,
        &[
            "write",
            "active_topic",
            "--body",
            "durable global knowledge",
            "--kind",
            "decision",
            "--global",
        ],
    );
    assert_eq!(code, 0);

    // Local note with short TTL (1 second)
    let (code, _, _) = mem_out_global(
        &repo,
        &global_dir,
        &[
            "write",
            "active_topic",
            "--body",
            "transient local note",
            "--kind",
            "decision",
            "--scope",
            "session",
            "--ttl",
            "1",
        ],
    );
    assert_eq!(code, 0);

    // Sleep until local note is expired
    std::thread::sleep(std::time::Duration::from_millis(1500));

    // Query must return the global note tagged [global], not be shadowed by dead local session note
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["query", "knowledge"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("active_topic r1 decision durable "));
    assert!(stdout.contains(" [global]"));
    assert!(stdout.contains("durable global knowledge"));

    // Get must also return the active global note
    let (code, stdout, stderr) = mem_out_global(&repo, &global_dir, &["get", "active_topic"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("active_topic r1 decision durable "));
    assert!(stdout.contains(" [global]"));
    assert!(stdout.contains("durable global knowledge"));
}

#[test]
fn global_usage_dual_writes_and_summarizes_cross_repo() {
    let repo1 = temp_repo("global-usage-repo1");
    let repo2 = temp_repo("global-usage-repo2");
    let global_dir = temp_repo("global-usage-store");

    write(&repo1, "a.py", "def func_a(): pass\n");
    write(&repo2, "b.py", "def func_b(): pass\n");

    // Run build and search in repo1 with AW_GLOBAL_DIR
    aw(&repo1)
        .env("AW_GLOBAL_DIR", &global_dir)
        .env_remove("AW_NO_JOURNAL")
        .args(["build"])
        .output()
        .unwrap();
    aw(&repo1)
        .env("AW_GLOBAL_DIR", &global_dir)
        .env_remove("AW_NO_JOURNAL")
        .args(["search", "func_a"])
        .output()
        .unwrap();

    // Run build in repo2 with AW_GLOBAL_DIR
    aw(&repo2)
        .env("AW_GLOBAL_DIR", &global_dir)
        .env_remove("AW_NO_JOURNAL")
        .args(["build"])
        .output()
        .unwrap();

    // 1. Verify local journals exist and are isolated
    assert!(repo1.join(".aw/usage.log").exists());
    assert!(repo2.join(".aw/usage.log").exists());
    let log1 = fs::read_to_string(repo1.join(".aw/usage.log")).unwrap();
    assert_eq!(log1.lines().count(), 2);
    let log2 = fs::read_to_string(repo2.join(".aw/usage.log")).unwrap();
    assert_eq!(log2.lines().count(), 1);

    // 2. Verify global journal in global_dir has all 3 entries with repo attribution
    let global_log_path = global_dir.join("usage.log");
    assert!(global_log_path.exists());
    let g_log = fs::read_to_string(&global_log_path).unwrap();
    assert_eq!(g_log.lines().count(), 3);
    assert!(g_log.contains("\"repo_name\":"));
    assert!(g_log.contains("\"worktree\":false"));

    // 3. Local usage in repo1 only shows repo1
    let out1 = aw(&repo1)
        .env("AW_GLOBAL_DIR", &global_dir)
        .args(["usage"])
        .output()
        .unwrap();
    assert_eq!(out1.status.code(), Some(0));
    let stdout1 = String::from_utf8_lossy(&out1.stdout);
    assert!(stdout1.contains("aw usage: 2 journaled calls"));

    // 4. Global usage aggregates across both repositories
    let out_g = aw(&repo1)
        .env("AW_GLOBAL_DIR", &global_dir)
        .args(["usage", "--global"])
        .output()
        .unwrap();
    assert_eq!(out_g.status.code(), Some(0));
    let stdout_g = String::from_utf8_lossy(&out_g.stdout);
    assert!(stdout_g.contains("aw global usage: 3 journaled calls across 2 repositories"));
    assert!(stdout_g.contains("Active Repositories:"));
    assert!(stdout_g.contains("Recent Errors: None (100% clean)"));

    // 5. Global usage works from an empty external directory without git
    let empty_dir = temp_repo("global-usage-empty-non-git");
    let out_ext = std::process::Command::new(env!("CARGO_BIN_EXE_aw"))
        .current_dir(&empty_dir)
        .env("AW_GLOBAL_DIR", &global_dir)
        .args(["usage", "--global"])
        .output()
        .unwrap();
    assert_eq!(out_ext.status.code(), Some(0));
    let stdout_ext = String::from_utf8_lossy(&out_ext.stdout);
    assert!(stdout_ext.contains("aw global usage: 3 journaled calls across 2 repositories"));
}

#[test]
fn mem_query_finds_note_via_refs_when_terms_absent_from_body() {
    let repo = temp_repo("mem-refs-query");
    let (code, stdout, stderr) = mem_out(
        &repo,
        &[
            "write",
            "resolver-gotcha",
            "--body",
            "ambiguous edges keep target null",
            "--kind",
            "gotcha",
            "--refs",
            "src/extract/resolve.rs",
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");

    // Query for 'resolve' which is ONLY present in --refs (not key, tags, or body)
    let (code, stdout, stderr) = mem_out(&repo, &["query", "resolve"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains("resolver-gotcha"),
        "must find note via refs: {stdout}"
    );
}

#[test]
fn mem_stale_ref_flagged_when_anchor_modified_or_missing() {
    let repo = temp_repo("mem-stale-ref");
    write(&repo, "app.py", "def run():\n    return 42\n");
    build(&repo);

    let (code, stdout, stderr) = mem_out(
        &repo,
        &[
            "write",
            "app-note",
            "--body",
            "settled architecture for run function",
            "--kind",
            "decision",
            "--refs",
            "app.py",
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");

    // Right after build: app.py mtime <= note created_at, ref is valid
    let (code, stdout, stderr) = mem_out(&repo, &["query", "architecture"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("app-note"));
    assert!(
        !stdout.contains("[stale-ref]"),
        "must not be stale initially: {stdout}"
    );

    // Now sleep 1.1s and modify app.py, re-index so files.mtime > created_at
    std::thread::sleep(std::time::Duration::from_millis(1100));
    write(&repo, "app.py", "def run():\n    return 43\n");
    build(&repo);

    // Stale check in mem query
    let (code, stdout, stderr) = mem_out(&repo, &["query", "architecture"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("app-note"));
    assert!(
        stdout.contains("[stale-ref]"),
        "must flag [stale-ref] after file modification: {stdout}"
    );

    // Stale check in mem get
    let (code, stdout, stderr) = mem_out(&repo, &["get", "app-note"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains("[stale-ref]"),
        "mem get must also flag [stale-ref]: {stdout}"
    );
}

#[test]
fn reuse_detects_intra_file_duplication() {
    let repo = temp_repo("reuse-intra-file");
    write(
        &repo,
        "helpers.py",
        "def parse_token(token: str):\n    return token.strip()\n\ndef parse_token_v2(token: str):\n    return token.strip()\n",
    );
    build(&repo);

    let out = aw(&repo).args(["reuse"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("duplication candidate(s) found"),
        "must detect intra-file duplicate: {stdout}"
    );
    assert!(stdout.contains("parse_token"), "{stdout}");
}
