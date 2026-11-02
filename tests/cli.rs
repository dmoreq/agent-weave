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
    let line = stdout
        .lines()
        .find(|l| l.starts_with("kg: built index"))
        .unwrap();
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
        stderr.contains("schema v99, binary expects v5 — run kg build"),
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
    assert!(
        stderr.contains("kg: parse-quality: 25% of 4 files"),
        "{stderr}"
    );
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
    assert!(
        stdout.starts_with("kg: 2 of "),
        "re-hash staleness: {stdout}"
    );
}

#[test]
fn response_contract_limit_clamps() {
    let repo = fixture("limit");
    build(&repo);
    let out = kg(&repo)
        .args(["callers", "normalize", "--limit", "0"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = kg(&repo)
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
    let out = kg(&repo).args(["callers", "normalize"]).output().unwrap();
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
    let out = kg(&repo).args(["callers", "normalize"]).output().unwrap();
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
    let out = kg(&repo).args(["impact", "utils.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "impact must survive: {stdout}");
    assert!(
        stdout.contains("app.py:6"),
        "impact edge survived: {stdout}"
    );
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
    write(
        &repo,
        "utils.py",
        &format!("{UTILS}\n\ndef extra():\n    return 1\n"),
    );
    let (_, _, _) = build(&repo);
    let out = kg(&repo).args(["callers", "normalize"]).output().unwrap();
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
    let out = kg(&repo)
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
fn unresolved_reports_counts_and_zero_is_exit1() {
    let repo = temp_repo("unresolved");
    write(&repo, "m.py", "def go():\n    return ghost()\n");
    build(&repo);
    let out = kg(&repo).args(["unresolved"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("kg: unresolved=1 callees in 1 files"),
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
    assert!(
        stdout.contains("utils.py:1 function utils.normalize"),
        "{stdout}"
    );
    assert!(
        stdout.contains("utils.py:4 function utils.caller"),
        "{stdout}"
    );

    let out = kg(&repo).args(["symbols", "nope.py"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not indexed"), "{stderr}");
}

#[test]
fn symbols_outline_adds_signature_and_default_stays_byte_identical() {
    let repo = fixture("symbols-outline");
    build(&repo);
    let base = kg(&repo).args(["symbols", "utils.py"]).output().unwrap();
    assert_eq!(base.status.code(), Some(0));
    let plain = String::from_utf8_lossy(&base.stdout).into_owned();
    let out = kg(&repo)
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
    let out = kg(&repo).args(["search", "normalize"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("utils.py:1"), "{stdout}");
    assert!(
        !stdout.contains("[ambiguous]"),
        "search hits carry no tier tag: {stdout}"
    );
    // Amendment 14b: the hit line carries the stored signature.
    assert!(
        stdout.contains("def normalize(text):"),
        "search hit carries the signature: {stdout}"
    );
    // Amendment 14a: signature-only vocabulary is searchable (no symbol
    // name contains the `def` token; every signature does).
    let out = kg(&repo).args(["search", "def"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("utils.py:1"),
        "signature tokens reach symbols: {stdout}"
    );

    // No match: exit 1 with the documented reason line.
    let out = kg(&repo).args(["search", "zzzqqqxyz"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("0 symbols matched"), "{stderr}");

    // Empty terms: usage error, exit 2.
    let out = kg(&repo).args(["search", "   "]).output().unwrap();
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

    let out = kg(&repo)
        .args(["clones", "--limit", "10000"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let first = stdout.lines().next().unwrap().to_string();
    assert_eq!(
        first, "kg: clone candidates — normalized-text equality, not proof of semantic duplication",
        "{stdout}"
    );

    let pair_lines: Vec<&str> = stdout
        .lines()
        .filter(|l| l.contains(":") && !l.starts_with("kg:") && !l.starts_with("showing "))
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
    let out = kg(&repo)
        .args(["clones", "--min-lines", "100"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));

    // Carry-forward: no-change rebuild keeps the candidates (re-count from
    // the second run's own stdout — detection must hold independently).
    build(&repo);
    let out = kg(&repo)
        .args(["clones", "--limit", "10000"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout2 = String::from_utf8_lossy(&out.stdout);
    let pair_lines2: Vec<&str> = stdout2
        .lines()
        .filter(|l| l.contains(":") && !l.starts_with("kg:") && !l.starts_with("showing "))
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
    // Amendment 6: build + callers journal entries; kg usage aggregates them.
    let repo = fixture("journal");
    kg(&repo)
        .env_remove("KG_NO_JOURNAL")
        .args(["build"])
        .output()
        .unwrap();
    kg(&repo)
        .env_remove("KG_NO_JOURNAL")
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    let log = fs::read_to_string(repo.join(".kg/usage.log")).unwrap();
    assert_eq!(log.lines().count(), 2, "one entry per invocation: {log}");
    assert!(log.contains("\"tool\":\"build\""), "{log}");
    assert!(log.contains("\"tool\":\"callers\""), "{log}");
    assert!(log.contains("\"surface\":\"cli\""), "{log}");

    let out = kg(&repo)
        .env_remove("KG_NO_JOURNAL")
        .args(["usage"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout.contains("kg usage: 2 journaled calls"), "{stdout}");
    assert!(stdout.contains("callers: 1 calls"), "{stdout}");
    assert!(stdout.contains("build: 1 calls"), "{stdout}");
}

#[test]
fn journal_opt_out_via_env_and_flag() {
    let repo = fixture("journal-env");
    kg(&repo)
        .env("KG_NO_JOURNAL", "1")
        .args(["build"])
        .output()
        .unwrap();
    assert!(
        !repo.join(".kg/usage.log").exists(),
        "env opt-out must suppress the journal"
    );

    let repo = fixture("journal-flag");
    kg(&repo).args(["--no-journal", "build"]).output().unwrap();
    assert!(
        !repo.join(".kg/usage.log").exists(),
        "--no-journal must suppress the journal"
    );

    // usage on an empty journal: exit 1 with a reason.
    let out = kg(&repo).args(["usage"]).output().unwrap();
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
        !repo.join(".kg/usage-results.jsonl").exists(),
        "build writes no payload line"
    );

    let out = kg(&repo)
        .env_remove("KG_NO_JOURNAL")
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    let data_lines: Vec<String> = stdout
        .lines()
        .filter(|l| !l.starts_with("kg: "))
        .map(String::from)
        .collect();
    assert!(!data_lines.is_empty());

    let payload = fs::read_to_string(repo.join(".kg/usage-results.jsonl")).unwrap();
    let mut lines = payload.lines();
    let line = lines.next().unwrap();
    assert!(line.contains("\"tool\":\"callers\""), "{line}");
    assert!(line.contains("\"name\":\"normalize\""), "{line}");
    assert!(line.contains("\"surface\":\"cli\""), "{line}");
    assert!(line.contains("\"stale\":false"), "{line}");
    assert!(line.contains("\"built_at\":"), "{line}");
    assert!(line.contains("\"source_rev\":\""), "{line}");
    assert_eq!(payload_pointers(line), data_lines, "byte-equal pointers");

    kg(&repo)
        .env_remove("KG_NO_JOURNAL")
        .args(["dead"])
        .output()
        .unwrap();
    kg(&repo)
        .env_remove("KG_NO_JOURNAL")
        .args(["symbols", "utils.py"])
        .output()
        .unwrap();
    let payload = fs::read_to_string(repo.join(".kg/usage-results.jsonl")).unwrap();
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
    kg(&repo)
        .env_remove("KG_NO_JOURNAL")
        .args(["stats"])
        .output()
        .unwrap();
    let after = fs::read_to_string(repo.join(".kg/usage-results.jsonl"))
        .unwrap()
        .lines()
        .count();
    assert_eq!(before, after, "stats writes no payload line");
}

#[test]
fn session_and_arm_tags_recorded_in_both_logs() {
    // Amendment 10b: KG_SESSION/KG_ARM land in usage.log AND payload lines.
    let repo = fixture("payload-tags");
    build(&repo);
    kg(&repo)
        .env_remove("KG_NO_JOURNAL")
        .env("KG_SESSION", "  r1-task-1  ")
        .env("KG_ARM", "kg")
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    let usage = fs::read_to_string(repo.join(".kg/usage.log")).unwrap();
    assert!(usage.contains("\"session\":\"r1-task-1\""), "{usage}");
    assert!(usage.contains("\"arm\":\"kg\""), "{usage}");
    let payload = fs::read_to_string(repo.join(".kg/usage-results.jsonl")).unwrap();
    assert!(payload.contains("\"session\":\"r1-task-1\""), "{payload}");
    assert!(payload.contains("\"arm\":\"kg\""), "{payload}");
}

#[test]
fn payload_opt_out_and_empty_or_ambiguous_writes_none() {
    // Opt-out suppresses both journal files.
    let repo = fixture("payload-optout");
    kg(&repo)
        .env("KG_NO_JOURNAL", "1")
        .args(["build"])
        .output()
        .unwrap();
    kg(&repo)
        .env("KG_NO_JOURNAL", "1")
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    assert!(!repo.join(".kg/usage.log").exists());
    assert!(!repo.join(".kg/usage-results.jsonl").exists());

    // Empty result: journaled with exit 1, no payload line.
    let repo = fixture("payload-empty");
    build(&repo);
    let out = kg(&repo)
        .env_remove("KG_NO_JOURNAL")
        .args(["callers", "ghost"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        !repo.join(".kg/usage-results.jsonl").exists(),
        "empty result writes no payload"
    );

    // Ambiguous name: exit 2 with candidates on stdout, no payload line.
    let repo = temp_repo("payload-ambig");
    write(&repo, "a.py", "def dup():\n    return 1\n");
    write(&repo, "b.py", "def dup():\n    return 2\n");
    build(&repo);
    let out = kg(&repo).args(["callers", "dup"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        !repo.join(".kg/usage-results.jsonl").exists(),
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
    kg(&repo)
        .env_remove("KG_NO_JOURNAL")
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    kg(&repo)
        .env_remove("KG_NO_JOURNAL")
        .args(["dead"])
        .output()
        .unwrap();

    let out = kg(&repo).args(["usage", "--verify"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("kg usage --verify: 5 pointers sampled from 2 entries"),
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
    let out = kg(&repo).args(["usage", "--verify"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("changed=2"),
        "app.py pointers unverifiable: {stdout}"
    );
    assert!(stdout.contains("fail=0"), "changed is not fail: {stdout}");

    // Corrupt one pointer beyond bounds -> FAIL, still exit 0.
    let ppath = repo.join(".kg/usage-results.jsonl");
    let payload = fs::read_to_string(&ppath)
        .unwrap()
        .replace("utils.py:5", "utils.py:9999");
    fs::write(&ppath, payload).unwrap();
    let out = kg(&repo).args(["usage", "--verify"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "report-only");
    assert!(stdout.contains("fail=1"), "{stdout}");
    assert!(
        stdout.contains("FAIL utils.py:9999 (line beyond"),
        "{stdout}"
    );
}

#[test]
fn usage_verify_stride_sample_and_empty_payload() {
    // >limit entries sample exactly `limit` entries, deterministically.
    let repo = fixture("verify-stride");
    build(&repo);
    for _ in 0..60 {
        kg(&repo)
            .env_remove("KG_NO_JOURNAL")
            .args(["callers", "normalize"])
            .output()
            .unwrap();
    }
    let out = kg(&repo)
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
    let out = kg(&repo).args(["usage", "--verify"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no payload entries"), "{stderr}");

    // --limit 0 is a usage error (exit 2).
    let repo = fixture("verify-limit0");
    build(&repo);
    kg(&repo)
        .env_remove("KG_NO_JOURNAL")
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    let out = kg(&repo)
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
    kg(&repo)
        .env_remove("KG_NO_JOURNAL")
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    write(
        &repo,
        "app.py",
        "from utils import normalize\n\ndef run():\n    return normalize('z')\n",
    );
    build(&repo);
    let out = kg(&repo).args(["usage", "--verify"]).output().unwrap();
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
    kg(&repo)
        .env_remove("KG_NO_JOURNAL")
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    let out = kg(&repo).args(["usage"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("payload: 1 entries, 2 pointers"),
        "{stdout}"
    );
    assert!(stdout.contains("normalize: 1 calls"), "{stdout}");

    let out = kg(&repo).args(["doctor"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout.contains("payload journal: 1 lines, "), "{stdout}");

    // No pointer queries yet -> absent line, and no payload section in usage.
    let repo = fixture("usage-payload-absent");
    build(&repo);
    let out = kg(&repo).args(["usage"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("payload:"),
        "no section without payload: {stdout}"
    );
    let out = kg(&repo).args(["doctor"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("payload journal: absent"),
        "doctor states absence: {stdout}"
    );
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
    assert!(
        stdout.contains("files=3") && stdout.contains("symbols=") && stdout.contains("edges="),
        "{stdout}"
    );

    let out = kg(&repo).args(["hotspots"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("loc=") && stdout.contains("cyc="),
        "{stdout}"
    );

    let out = kg(&repo).args(["report"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
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
fn partial_v4_index_rebuilds_fresh() {
    // Amendment 12 regression: a v4-stamped index from the first landing
    // wave (symbols_fts present, clone_groups absent) must disable the fast
    // path and rebuild fully, not fail carry-forward on the missing table.
    let repo = fixture("partial-v4");
    build(&repo);
    // Demote the index to a first-wave v4: drop clone_groups in place.
    let idx = repo.join(".kg/index.db");
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
// G17 (Amendment 13): `kg path` seeded fixtures — 10 scenarios, each asserting
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
    let out = kg(&repo).args(["path", "ca", "cd"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "kg: path — resolved static edges only; dynamic dispatch may hide paths\n\
         chain.py:2\nchain.py:5\nchain.py:8\n\
         kg: chain chain.ca -> chain.cb -> chain.cc -> chain.cd (3 hops)\n",
        "scenario 1: {stdout}"
    );

    // (2) Diamond tie-break: dtop reaches dbot through BOTH dm1 and dm2 at
    // 2 hops; dm1 expands before dm2 (qualname order), so the pinned
    // first-discoverer parent rule yields dtop -> dm1 -> dbot.
    let out = kg(&repo).args(["path", "dtop", "dbot"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "kg: path — resolved static edges only; dynamic dispatch may hide paths\n\
         diamond.py:2\ndiamond.py:6\n\
         kg: chain diamond.dtop -> diamond.dm1 -> diamond.dbot (2 hops)\n",
        "scenario 2: {stdout}"
    );

    // (3) Cycle: cya reaches cyc in 2 hops and terminates.
    let out = kg(&repo).args(["path", "cya", "cyc"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "kg: path — resolved static edges only; dynamic dispatch may hide paths\n\
         cycle.py:2\ncycle.py:5\n\
         kg: chain cycle.cya -> cycle.cyb -> cycle.cyc (2 hops)\n",
        "scenario 3: {stdout}"
    );

    // (4) Disconnected pair: definitive exhaustive negative.
    let out = kg(&repo).args(["path", "iso1", "iso2"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(stdout, "", "scenario 4 stdout: {stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        stderr,
        "kg: no-results: no call path from 'disconnected.iso1' to 'disconnected.iso2' at any depth <= 10 (resolved edges only)\n",
        "scenario 4 stderr: {stderr}"
    );

    // (5) Depth-cap miss: the real path is 11 hops, beyond the 1-10 clamp,
    // so any --depth clamps to 10 and yields the cap-truncated negative.
    // Boundary: f01 -> f11 is exactly 10 hops and is found at the default;
    // --depth 11 behaves byte-identically (clamped).
    let out = kg(&repo).args(["path", "f01", "f12"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        stderr,
        "kg: no-results: no call path from 'deep.f01' to 'deep.f12' within depth 10; a longer path may exist (resolved edges only)\n",
        "scenario 5 truncated stderr: {stderr}"
    );
    let mut expected =
        String::from("kg: path — resolved static edges only; dynamic dispatch may hide paths\n");
    for i in 1..=10 {
        expected.push_str(&format!("deep.py:{}\n", 2 * i));
    }
    expected.push_str("kg: chain deep.f01 -> deep.f02 -> deep.f03 -> deep.f04 -> deep.f05 -> deep.f06 -> deep.f07 -> deep.f08 -> deep.f09 -> deep.f10 -> deep.f11 (10 hops)\n");
    let out = kg(&repo).args(["path", "f01", "f11"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(stdout, expected, "scenario 5 boundary: {stdout}");
    let out = kg(&repo)
        .args(["path", "f01", "f11", "--depth", "11"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(stdout, expected, "scenario 5 clamp: {stdout}");

    // (5b) Min-clamp boundary: --depth 0 clamps to 1; ca -> cc needs 2 hops,
    // so the frontier stops with unvisited out-neighbors (cap-truncated).
    let out = kg(&repo)
        .args(["path", "ca", "cc", "--depth", "0"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        stderr,
        "kg: no-results: no call path from 'chain.ca' to 'chain.cc' within depth 1; a longer path may exist (resolved edges only)\n",
        "scenario 5b min-clamp stderr: {stderr}"
    );

    // (6) from == to: same-symbol negative (exit 1); an ambiguous shared
    // name resolves FIRST and exits 2 (pinned order: ambiguity precedes the
    // same-symbol check).
    let out = kg(&repo).args(["path", "ca", "ca"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        stderr, "kg: no-results: 'chain.ca' and 'chain.ca' resolve to the same symbol\n",
        "scenario 6 stderr: {stderr}"
    );
    let out = kg(&repo).args(["path", "dup", "dup"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "ambiguous shared name exits 2, not 1"
    );

    // (7) Ambiguous `from` endpoint: exit 2, role in the detail, candidates
    // on stdout; the Class.method form resolves.
    let out = kg(&repo).args(["path", "dup", "ca"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout, "amb.py:2 amb.Apage.dup\namb.py:6 amb.Bpage.dup\n",
        "scenario 7 stdout: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        stderr,
        "kg: usage: ambiguous name 'dup' — 2 candidates as the from endpoint; use the Class.method qualname form\n",
        "scenario 7 stderr: {stderr}"
    );
    let out = kg(&repo)
        .args(["path", "amb.Apage.dup", "target"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "kg: path — resolved static edges only; dynamic dispatch may hide paths\n\
         amb.py:3\n\
         kg: chain amb.Apage.dup -> amb.target (1 hops)\n",
        "scenario 7 resolved form: {stdout}"
    );

    // (8) Ambiguous `to` endpoint: exit 2 with the `to` role.
    let out = kg(&repo).args(["path", "ca", "dup"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout, "amb.py:2 amb.Apage.dup\namb.py:6 amb.Bpage.dup\n",
        "scenario 8 stdout: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        stderr,
        "kg: usage: ambiguous name 'dup' — 2 candidates as the to endpoint; use the Class.method qualname form\n",
        "scenario 8 stderr: {stderr}"
    );

    // (9) Multi-edge representative: two call sites of mtarget; the pointer
    // is the smallest site_line (line 2).
    let out = kg(&repo)
        .args(["path", "multi_caller", "mtarget"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "kg: path — resolved static edges only; dynamic dispatch may hide paths\n\
         multi.py:2\n\
         kg: chain multi.multi_caller -> multi.mtarget (1 hops)\n",
        "scenario 9: {stdout}"
    );

    // (10) Stale-index header: change a displayed file post-build; the
    // staleness line follows the caveat on the displayed set.
    write(
        &repo,
        "chain.py",
        "def ca():\n    return cb()\n\ndef cb():\n    return cc()\n\ndef cc():\n    return cd()\n\ndef cd():\n    return 1\n# touched\n",
    );
    let out = kg(&repo).args(["path", "ca", "cd"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "kg: path — resolved static edges only; dynamic dispatch may hide paths\n\
         kg: 1 of 1 result files changed since build — run kg build\n\
         chain.py:2\nchain.py:5\nchain.py:8\n\
         kg: chain chain.ca -> chain.cb -> chain.cc -> chain.cd (3 hops)\n",
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
    let out = kg(&repo).args(["stats"]).output().unwrap();
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
    let out = kg(&repo).args(["clones", "--limit", "5"]).output().unwrap();
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
    let out = kg(&repo).args(["callers", "foo"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "foo does not exist yet");
    // Add the missing symbol; the next incremental build must heal a.py.
    write(
        &repo,
        "b.py",
        "def existing():\n    return 1\n\ndef foo():\n    return 2\n",
    );
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let out = kg(&repo).args(["callers", "foo"]).output().unwrap();
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
    let out = kg(&repo).args(["callers", "Widget"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "class has an inbound call edge: {:?}",
        out
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("c.py:6"), "{stdout}");
    let out = kg(&repo)
        .args(["callers", "c.Widget.__init__"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "ctor keeps its own edge");
    let out = kg(&repo).args(["dead"]).output().unwrap();
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
            kg::db::GRAMMAR_VERSION.contains(&expected),
            "GRAMMAR_VERSION {} missing {expected}",
            kg::db::GRAMMAR_VERSION
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
    let out = kg(&repo).args(["stats"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("files=1"),
        "tracked-ignored file indexed: {stdout}"
    );
    let out = kg(&repo).args(["callers", "hidden_fn"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "defined but uncalled");
    let out = kg(&repo)
        .args(["symbols", "generated/g.py"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "file indexed");
}
