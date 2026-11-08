//! Integration tests: robustness section.

use super::common::*;

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
