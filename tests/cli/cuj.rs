//! Integration tests: cuj section.

use super::common::*;

#[test]
fn cuj1_build_happy_path_and_readonly_tree() {
    let repo = fixture("cuj1");
    let before = snapshot(&repo);
    let (code, stdout, _stderr) = build(&repo);
    assert_eq!(code, 0);
    let line = stdout
        .lines()
        .find(|l| l.starts_with("eg: built index"))
        .unwrap();
    for key in ["files=3", "parse_errors=0", "wall="] {
        assert!(line.contains(key), "stats line missing {key}: {line}");
    }
    assert!(repo.join(".engram/index.db").exists());
    assert!(repo.join(".engram/.gitignore").exists());
    let after = snapshot(&repo);
    assert_eq!(before, after, "tree changed outside .engram/ (invariant 3)");
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

    let out = eg(&repo).args(["callers", "normalize"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    let pointers: Vec<&str> = stdout
        .lines()
        .filter(|l| l.contains(':') && !l.starts_with("eg:"))
        .collect();
    assert_eq!(pointers.len(), 50, "cap default 50");
    assert!(pointers.iter().all(|l| l.split(':').count() >= 2));
    assert!(stdout.contains("showing 50 of 60"), "stdout: {stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("[eg] capped at 50 results (60 total)"),
        "stderr: {stderr}"
    );
}

#[test]
fn cuj3_corrupt_index_exit3() {
    let repo = fixture("cuj3");
    build(&repo);
    fs::write(repo.join(".engram/index.db"), b"garbage").unwrap();
    let out = eg(&repo).args(["callers", "normalize"]).output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("eg: corrupt-index:"), "{stderr}");
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
    let out = eg(&repo).args(["callers", "process"]).output().unwrap();
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
            "import sqlite3; c = sqlite3.connect('{}/.engram/index.db'); c.execute('PRAGMA user_version = 99'); c.commit()",
            repo.display()
        ))
        .output()
        .unwrap();
    assert!(out.status.success());
    let out = eg(&repo).args(["callers", "normalize"]).output().unwrap();
    assert_eq!(out.status.code(), Some(5));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("schema v99, binary expects v10 — run eg build"),
        "{stderr}"
    );
}

#[test]
fn cuj6_concurrent_build_lock_exit4() {
    let repo = fixture("cuj6");
    // Hold the lock from this process; the child build must exit 4.
    let lock_path = repo.join(".engram/build.lock");
    fs::create_dir_all(repo.join(".engram")).unwrap();
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .read(true)
        .open(&lock_path)
        .unwrap();
    file.try_lock().unwrap();
    let out = eg(&repo).args(["build"]).output().unwrap();
    assert_eq!(out.status.code(), Some(4));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("eg: locked:"), "{stderr}");
    drop(file);
}

#[test]
fn cuj7_query_without_index_exit1() {
    let repo = temp_repo("cuj7");
    write(&repo, "a.py", "def f():\n    pass\n");
    let out = eg(&repo).args(["callers", "f"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("eg: no-results: no .engram/index.db under")
            && stderr.contains("run eg build"),
        "{stderr}"
    );
}

#[test]
fn cuj8_parse_quality_abort_exit7() {
    let repo = fixture("cuj8");
    write(&repo, "broken.py", BROKEN);
    // 1/4 files broken = 25% > default 5%.
    let out = eg(&repo).args(["build"]).output().unwrap();
    assert_eq!(out.status.code(), Some(7));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("eg: parse-quality: 25% of 4 files"),
        "{stderr}"
    );
    // --force proceeds.
    let out = eg(&repo).args(["build", "--force"]).output().unwrap();
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
    let out = eg(&repo).args(["callers", "normalize"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("eg: 1 of "),
        "staleness header first line: {stdout}"
    );
    assert!(stdout.contains("result files changed since build — run eg build"));
    // FNV-1a re-hash path: same size, different content -> still stale.
    // Nanosecond mtimes make the mtime mismatch (and thus the re-hash)
    // deterministic; both result files now report changed.
    let original = fs::read_to_string(repo.join("utils.py")).unwrap();
    let swapped: String = original.chars().rev().collect();
    fs::write(repo.join("utils.py"), &swapped).unwrap();
    let out = eg(&repo).args(["callers", "normalize"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("eg: 2 of "),
        "re-hash staleness: {stdout}"
    );
}

#[test]
fn symbols_directory_unknown_reports_zero_symbols() {
    let repo = temp_repo("symbols-unknown-dir");
    write(&repo, "pkg/a.py", "def a():\n    pass\n");
    build(&repo);

    let out = eg(&repo).args(["symbols", "nosuchdir"]).output().unwrap();
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

    let out = eg(&repo)
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
        let out = eg(&repo).args(&args).output().unwrap();
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
        let out = eg(&repo).args(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn impact_sites_empty_reason_keeps_edge_suffix() {
    let repo = temp_repo("impact-empty-sites");
    write(&repo, "a.py", "def f():\n    pass\n");
    build(&repo);
    let out = eg(&repo)
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
    let out = eg(&repo).args(["impact", "a.py"]).output().unwrap();
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
        stdout.starts_with("eg: impact — resolved static call edges only"),
        "{stdout}"
    );
    // The named relationship is real: --edges imports finds the importer.
    let out = eg(&repo)
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
    let out = eg(&repo)
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
    let out = eg(&repo)
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

    let out = eg(&repo).args(["imports", "rootidx.ts"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("rootidx.ts:1 root"), "{stdout}");
    assert!(stdout.contains("rootidx.ts:1 root.idx"), "{stdout}");
    let out = eg(&repo)
        .args(["imports", "pkg/relinner.py"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("pkg/relinner.py:1 app"), "{stdout}");
    let out = eg(&repo).args(["unresolved"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "no unresolved expected");
}

#[test]
fn response_contract_limit_clamps() {
    let repo = fixture("limit");
    build(&repo);
    let out = eg(&repo)
        .args(["callers", "normalize", "--limit", "0"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = eg(&repo)
        .args(["callers", "normalize", "--limit", "999999"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "clamped, not rejected");
}
