//! Integration tests: invalidation section.

use super::common::*;

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
    let ambiguous = eg(&repo).args(["callees", "invoke"]).output().unwrap();
    assert_eq!(ambiguous.status.code(), Some(0), "{ambiguous:?}");
    assert!(
        String::from_utf8_lossy(&ambiguous.stdout).contains("[ambiguous] foo"),
        "{ambiguous:?}"
    );

    fs::remove_file(repo.join("pkg/__init__.py")).unwrap();
    let (code, stdout, stderr) = build(&repo);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let after = eg(&repo).args(["callees", "invoke"]).output().unwrap();
    assert_eq!(after.status.code(), Some(0), "{after:?}");
    let text = String::from_utf8_lossy(&after.stdout);
    assert!(
        !text.contains("[ambiguous]"),
        "incremental kept the ambiguous edge: {after:?}"
    );
    assert!(text.contains("pkg.foo"), "{after:?}");

    let incremental_db = repo.join("incremental.db");
    fs::copy(repo.join(".engram/index.db"), &incremental_db).unwrap();
    let force = eg(&repo).args(["build", "--force"]).output().unwrap();
    assert_eq!(force.status.code(), Some(0), "{force:?}");
    assert_eq!(
        dump_edges(&incremental_db),
        dump_edges(&repo.join(".engram/index.db")),
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
    let out = eg(&repo).args(["callers", "target"]).output().unwrap();
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
    let out = eg(&repo).args(["callers", "target"]).output().unwrap();
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
    let out = eg(&repo).args(["callers", "C.run"]).output().unwrap();
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
    let a = eg(&repo).args(["callers", "A.run"]).output().unwrap();
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
    let b = eg(&repo).args(["callers", "B.run"]).output().unwrap();
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
    let go = eg(&repo).args(["callees", "go"]).output().unwrap();
    assert_eq!(go.status.code(), Some(0), "{go:?}");
    let gtext = String::from_utf8_lossy(&go.stdout);
    assert!(gtext.contains("sample.B.run"), "{gtext}");
    assert!(
        !gtext.contains("sample.A.run"),
        "annotation overrode reassignment: {gtext}"
    );
    let keep = eg(&repo).args(["callees", "keep"]).output().unwrap();
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
    let out = eg(&repo).args(["callers", "C.run"]).output().unwrap();
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
    let a = eg(&repo).args(["callers", "A.render"]).output().unwrap();
    assert_eq!(a.status.code(), Some(0), "{a:?}");
    assert!(
        String::from_utf8_lossy(&a.stdout).contains("lib.A.go"),
        "{a:?}"
    );
    let b = eg(&repo).args(["callers", "B.render"]).output().unwrap();
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
    let run = eg(&repo).args(["callers", "helpers.run"]).output().unwrap();
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    let runtext = String::from_utf8_lossy(&run.stdout);
    assert!(runtext.contains("app.go"), "{runtext}");
    assert!(
        !runtext.contains("app.shadow"),
        "parameter shadow leaked: {runtext}"
    );
    let method = eg(&repo).args(["callers", "C.run"]).output().unwrap();
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
    let out = eg(&repo).args(["callers", "work"]).output().unwrap();
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
        let a = eg(&repo).args(["callers", "a.target"]).output().unwrap();
        assert_eq!(a.status.code(), Some(0), "{name}: {a:?}");
        let atext = String::from_utf8_lossy(&a.stdout);
        assert!(atext.contains(left_qn), "{name}: {atext}");
        assert!(!atext.contains(right_qn), "{name} left leaked: {atext}");
        let b = eg(&repo).args(["callers", "b.target"]).output().unwrap();
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
    let a = eg(&repo).args(["callers", "a.f"]).output().unwrap();
    assert_eq!(a.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&a.stdout).contains("app.left"));
    let b = eg(&repo).args(["callers", "b.f"]).output().unwrap();
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
    let out = eg(&repo).args(["callers", "target"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("sample.run2"), "{stdout}");
    assert!(!stdout.contains("sample.run "), "{stdout}");
    let unresolved = eg(&repo).args(["unresolved", "--names"]).output().unwrap();
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
    let out = eg(&repo).args(["callers", "A.m"]).output().unwrap();
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
    let out = eg(&repo).args(["callees", "run"]).output().unwrap();
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
    let out = eg(&repo).args(["callers", "helper"]).output().unwrap();
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
    let symbols = eg(&repo).args(["symbols", "sample.py"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&symbols.stdout);
    assert!(stdout.contains("sample.outer.C.m"), "{stdout}");
    assert!(stdout.contains("sample.outer.C.run"), "{stdout}");
    let callees = eg(&repo)
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
    let callers = eg(&repo)
        .args(["callers", "tools.execute"])
        .output()
        .unwrap();
    assert_eq!(callers.status.code(), Some(1), "{callers:?}");
    let callees = eg(&repo).args(["callees", "run"]).output().unwrap();
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
    let shadowed = eg(&repo)
        .args(["callees", "app.shadowed"])
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&shadowed.stdout).contains("pkg.mod.f"),
        "{shadowed:?}"
    );
    let control = eg(&repo).args(["callees", "app.control"]).output().unwrap();
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
    let out = eg(&repo).args(["callees", "run"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("A.render"));
}
