//! Integration tests: incremental section.

use super::common::*;

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
