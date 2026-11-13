//! Integration tests: force_parse section.

use super::common::*;

#[test]
fn build_force_bypasses_parse_gate_and_help_says_so() {
    let repo = temp_repo("parse-force");
    write(&repo, "broken.py", "def broken(:\n");
    let strict = eg(&repo).args(["build"]).output().unwrap();
    assert_eq!(strict.status.code(), Some(7), "{strict:?}");
    let forced = eg(&repo).args(["build", "--force"]).output().unwrap();
    assert_eq!(forced.status.code(), Some(0), "{forced:?}");

    let help = eg(&repo).args(["build", "--help"]).output().unwrap();
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(text.contains("parse-error"), "{text}");
    let doctor = eg(&repo).args(["doctor", "--help"]).output().unwrap();
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
    let out = eg(&repo)
        .args(["callers", "leaf", "--depth", "2"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("sample.left [depth=1]"), "{text}");
    assert!(text.contains("sample.top [depth=1]"), "{text}");
    assert!(text.contains("sample.right [depth=2]"), "{text}");
    assert!(!text.contains("sample.top [depth=2]"), "{text}");
    let self_direct = eg(&repo).args(["callers", "self_rec"]).output().unwrap();
    assert_eq!(self_direct.status.code(), Some(0), "{self_direct:?}");
    assert!(
        String::from_utf8_lossy(&self_direct.stdout).contains("sample.self_rec"),
        "{self_direct:?}"
    );
    // The queried symbol is the closure seed: its self-call is not repeated
    // in transitive rows.
    let self_deep = eg(&repo)
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
    let callers = eg(&repo).args(["callers", "target"]).output().unwrap();
    assert_eq!(callers.status.code(), Some(0), "{callers:?}");
    assert!(
        String::from_utf8_lossy(&callers.stdout).contains("sample.go"),
        "{callers:?}"
    );
    let callees = eg(&repo).args(["callees", "go"]).output().unwrap();
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
    let a = eg(&ts).args(["callers", "A.run"]).output().unwrap();
    assert_eq!(a.status.code(), Some(1), "{a:?}");
    let b = eg(&ts).args(["callers", "B.run"]).output().unwrap();
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
    let a = eg(&rs).args(["callers", "A.render"]).output().unwrap();
    assert_eq!(a.status.code(), Some(1), "{a:?}");
    let b = eg(&rs).args(["callers", "B.render"]).output().unwrap();
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
    let out = eg(&repo).args(["callers", "target"]).output().unwrap();
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
    let out = eg(&repo).args(["callers", "target"]).output().unwrap();
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
    let out = eg(&repo).args(["callers", "target"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("sample.js:7"), "outside call lost: {text}");
    assert!(
        !text.contains("sample.js:4"),
        "nested function escaped the arrow binding: {text}"
    );
}

#[cfg(unix)]
#[cfg(unix)]
#[test]
fn hardlinked_build_lock_does_not_touch_external_file() {
    let repo = temp_repo("hardlink-lock");
    write(&repo, "a.py", "def f(): pass\n");
    let sentinel = std::env::temp_dir().join(format!("eg-sentinel-lock-{}", std::process::id()));
    fs::write(&sentinel, b"keep\n").unwrap();
    fs::create_dir_all(repo.join(".engram")).unwrap();
    fs::hard_link(&sentinel, repo.join(".engram/build.lock")).unwrap();
    let out = eg(&repo).args(["build"]).output().unwrap();
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
    fs::remove_file(repo.join(".engram/usage.log")).ok();
    let status = std::process::Command::new("mkfifo")
        .arg(repo.join(".engram/usage.log"))
        .status()
        .unwrap();
    assert!(status.success());
    let mut cmd = eg(&repo);
    cmd.args(["usage"]);
    let out = output_with_timeout(cmd, 5).expect("usage hung on a FIFO journal");
    assert_eq!(out.status.code(), Some(6), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("refusing unsafe journal"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn fifo_build_lock_does_not_hang_doctor() {
    let repo = temp_repo("fifo-doctor-lock");
    write(&repo, "a.py", "def f(): pass\n");
    assert_eq!(build(&repo).0, 0);
    let lock = repo.join(".engram/build.lock");
    fs::remove_file(&lock).unwrap();
    assert!(Command::new("mkfifo")
        .arg(&lock)
        .status()
        .unwrap()
        .success());
    let mut cmd = eg(&repo);
    cmd.arg("doctor");
    let out = output_with_timeout(cmd, 5).expect("doctor hung on a FIFO lock");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
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
    fs::copy(repo.join(".engram/index.db"), &incremental_db).unwrap();

    let force = eg(&repo).args(["build", "--force"]).output().unwrap();
    assert_eq!(force.status.code(), Some(0), "{force:?}");
    let incremental = dump_edges(&incremental_db);
    let fresh = dump_edges(&repo.join(".engram/index.db"));
    assert_eq!(incremental, fresh, "incremental and fresh edge sets differ");

    let callers = eg(&repo).args(["callers", "A.foo"]).output().unwrap();
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
    let out = eg(&repo).args(["callees", "go"]).output().unwrap();
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
    let incremental = eg(&repo).args(["callees", "go"]).output().unwrap();
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
    fs::copy(repo.join(".engram/index.db"), &incremental_db).unwrap();
    let force = eg(&repo).args(["build", "--force"]).output().unwrap();
    assert_eq!(force.status.code(), Some(0), "{force:?}");
    let fresh = eg(&repo).args(["callees", "go"]).output().unwrap();
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
    let callees = eg(&repo).args(["callees", "invoke"]).output().unwrap();
    let text = String::from_utf8_lossy(&callees.stdout);
    assert!(text.contains("pkg.foo"), "{text}");
    assert!(!text.contains("[ambiguous]"), "{text}");
}
