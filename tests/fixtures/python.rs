//! Python resolution fixtures: imports, receiver typing, MRO, and scoping.

use engram::extract::{extract_file, resolve, Confidence, EdgeKind};
use engram::parse::{parse_source, Lang};

use super::common::edges_for;

#[test]
fn star_import_defers_to_star_module() {
    // from utils import * ; helper() -> AMBIGUOUS import edge to the utils
    // module; the bare call defers to the star module's top-level names.
    let src1 = "def helper():\n    return 1\n";
    let src2 = "from .utils import *\n\ndef go():\n    return helper()\n";
    let parsed1 = parse_source(src1.as_bytes(), Lang::Python).unwrap();
    let parsed2 = parse_source(src2.as_bytes(), Lang::Python).unwrap();
    let fx1 = extract_file(0, "pkg/utils.py", src1.as_bytes(), &parsed1);
    let fx2 = extract_file(1, "pkg/mod.py", src2.as_bytes(), &parsed2);
    let resolution = resolve(vec![fx1, fx2], Default::default());
    let e: Vec<_> = resolution
        .edges
        .iter()
        .map(|x| (x.kind, x.confidence, x.dst_name.clone()))
        .collect();
    assert!(
        e.iter().any(|(k, c, n)| *k == EdgeKind::Imports
            && *c == Confidence::Ambiguous
            && n.ends_with(".utils")),
        "star import edge: {e:?}"
    );
    assert!(
        e.iter().any(|(k, c, n)| *k == EdgeKind::Calls
            && *c == Confidence::Extracted
            && n.contains("helper")),
        "bare call under star import resolves to utils.helper: {e:?}"
    );
}

#[test]
fn getattr_dispatch_is_dropped() {
    let src = "def go(obj):\n    return getattr(obj, 'run')()\n";
    let e = edges_for(src);
    assert!(
        !e.iter().any(|(k, _, _)| k == "calls"),
        "getattr dispatch must not produce edges: {e:?}"
    );
}

#[test]
fn variable_reassignment_last_binding_wins() {
    let src = "class A:\n    def m(self):\n        return 1\n\nclass B:\n    def m(self):\n        return 2\n\ndef go(flag):\n    obj = A()\n    if flag:\n        obj = B()\n    return obj.m()\n";
    let e = edges_for(src);
    assert!(
        e.iter()
            .any(|(k, c, n)| k == "calls" && c == "EXTRACTED" && n.contains("B.m")),
        "reassignment -> B.m expected: {e:?}"
    );
}

#[test]
fn import_x_as_y_binds_module_root() {
    // R1: `import helpers as hp; hp.help_me()` must resolve to the module's
    // function (positive assertion; the old test only asserted an absence).
    let src1 = "def help_me():\n    return 1\n";
    let src2 = "import helpers as hp\n\ndef go():\n    return hp.help_me()\n";
    let p1 = parse_source(src1.as_bytes(), Lang::Python).unwrap();
    let p2 = parse_source(src2.as_bytes(), Lang::Python).unwrap();
    let fx1 = extract_file(0, "helpers.py", src1.as_bytes(), &p1);
    let fx2 = extract_file(1, "mod.py", src2.as_bytes(), &p2);
    let resolution = resolve(vec![fx1, fx2], Default::default());
    assert!(
        resolution.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls
                && e.confidence == Confidence::Extracted
                && e.dst_name == "helpers.help_me"
        }),
        "hp.help_me() must resolve to helpers.help_me: {:?}",
        resolution.edges
    );
}

#[test]
fn long_chain_respects_local_module_shadowing() {
    let module = "def f(): pass\n";
    let app =
        "import pkg.mod\ndef shadowed(pkg):\n    pkg.mod.f()\ndef control():\n    pkg.mod.f()\n";
    let files = [("pkg/mod.py", module), ("app.py", app)]
        .into_iter()
        .enumerate()
        .map(|(index, (path, source))| {
            let parsed = parse_source(source.as_bytes(), Lang::Python).unwrap();
            extract_file(index, path, source.as_bytes(), &parsed)
        })
        .collect();
    let resolution = resolve(files, Default::default());
    let sites: Vec<_> = resolution
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Calls && edge.dst_name == "pkg.mod.f")
        .map(|edge| edge.site_line)
        .collect();
    assert_eq!(sites, vec![5]);
}

#[test]
fn factory_nested_constructor_types_nothing() {
    // R1: obj = factory(A()) must NOT type obj as A.
    let src = r#"class A:
    def m(self):
        return 1

def factory(x):
    return x

def go():
    obj = factory(A())
    return obj.m()
"#;
    let e = edges_for(src);
    assert!(
        !e.iter()
            .any(|(k, c, n)| k == "calls" && c == "EXTRACTED" && n.contains("A.m")),
        "factory(A()) must not type obj as A: {e:?}"
    );
    // A() itself has no explicit __init__ -> the class symbol is the target.
    assert!(
        e.iter()
            .any(|(k, c, n)| k == "calls" && c == "EXTRACTED" && n.ends_with(".A")),
        "A() constructor edge: {e:?}"
    );
}

#[test]
fn module_qualified_constructor_resolves() {
    // R1: from pkg2 import mod; mod.K() must resolve via the module path.
    let src1 = "class K:\n    def m(self):\n        return 1\n";
    let src2 = "from pkg.pkg2 import mod\n\ndef go():\n    return mod.K()\n";
    let parsed1 = parse_source(src1.as_bytes(), Lang::Python).unwrap();
    let parsed2 = parse_source(src2.as_bytes(), Lang::Python).unwrap();
    let fx1 = extract_file(0, "pkg/pkg2/mod.py", src1.as_bytes(), &parsed1);
    let fx2 = extract_file(1, "pkg/other.py", src2.as_bytes(), &parsed2);
    let resolution = resolve(vec![fx1, fx2], Default::default());
    let hit = resolution.edges.iter().any(|e| {
        e.kind == EdgeKind::Calls
            && e.confidence == Confidence::Extracted
            && e.dst_name.contains("mod.K")
    });
    assert!(hit, "mod.K() must resolve: {:?}", resolution.edges);
}

#[test]
fn or_fallback_construction_types_receiver() {
    // R1 cycle 2: `x = a or B()` (the fallback-construction idiom) must type
    // x as B at the RECEIVER level — the B.m edge, not just the ctor edge.
    let src = "class B:\n    def m(self):\n        return 1\n\ndef go(a):\n    x = a or B()\n    return x.m()\n";
    let e = edges_for(src);
    assert!(
        e.iter()
            .any(|(k, c, n)| k == "calls" && c == "EXTRACTED" && n.contains("B.m")),
        "or-idiom receiver typing -> B.m: {e:?}"
    );
}

#[test]
fn typed_receiver_emits_type_ref_edge() {
    // Type-only reference (arch-5): `x.foo()` with `x: b.B` resolves through
    // the MRO, but the incremental dirty closure also needs an edge to the
    // receiver's class itself — a method added on an ancestor must re-extract
    // the caller instead of hiding behind a carried edge. The edge is emitted
    // regardless of the call outcome (Python only).
    let sources = [
        ("a.py", "class A:\n    def foo(self):\n        return 1\n"),
        ("mid.py", "from a import A\n\nclass Mid(A):\n    pass\n"),
        ("b.py", "from mid import Mid\n\nclass B(Mid):\n    pass\n"),
        (
            "caller.py",
            "from __future__ import annotations\n\ndef go(x: b.B):\n    return x.foo()\n",
        ),
    ];
    let files = sources
        .into_iter()
        .enumerate()
        .map(|(index, (path, source))| {
            let parsed = parse_source(source.as_bytes(), Lang::Python).unwrap();
            extract_file(index, path, source.as_bytes(), &parsed)
        })
        .collect();
    let resolution = resolve(files, Default::default());
    let type_refs: Vec<_> = resolution
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::TypeRef)
        .collect();
    assert_eq!(
        type_refs.len(),
        1,
        "exactly one type_ref edge: {type_refs:?}"
    );
    assert_eq!(type_refs[0].dst_name, "b.B", "{type_refs:?}");
    assert_eq!(
        type_refs[0].confidence,
        Confidence::Extracted,
        "{type_refs:?}"
    );
    assert!(
        resolution.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls
                && e.confidence == Confidence::Extracted
                && e.dst_name == "a.A.foo"
        }),
        "MRO hit before Mid.foo exists: {:?}",
        resolution.edges
    );
}

#[test]
fn diamond_mro_multi_match_is_ambiguous() {
    // Post-R2 review: two same-level MRO matches (diamond) must emit an
    // AMBIGUOUS calls edge, never silently drop the call site.
    let src = "class A:\n    def m(self):\n        return 1\n\nclass B(A):\n    def m(self):\n        return 2\n\nclass C(A):\n    def m(self):\n        return 3\n\nclass D(B, C):\n    pass\n\ndef go():\n    d = D()\n    return d.m()\n";
    let e = edges_for(src);
    assert!(
        e.iter()
            .any(|(k, c, n)| k == "calls" && c == "AMBIGUOUS" && n.contains("D.m")),
        "diamond multi-match must be AMBIGUOUS: {e:?}"
    );
}

#[test]
fn root_level_relative_import_resolves_without_stray_dot() {
    // M1 finding: `from .widget import Widget as W` inside a repo-root plain
    // module resolved to ".widget" (stray leading dot) and the W() call was
    // misclassified as dropped-external instead of resolving to the class.
    let src1 = "class Widget:\n    def render(self):\n        return 1\n";
    let src2 =
        "from .widget import Widget as W\n\ndef run():\n    w = W()\n    return w.render()\n";
    let parsed1 = parse_source(src1.as_bytes(), Lang::Python).unwrap();
    let parsed2 = parse_source(src2.as_bytes(), Lang::Python).unwrap();
    let fx1 = extract_file(0, "widget.py", src1.as_bytes(), &parsed1);
    let fx2 = extract_file(1, "app.py", src2.as_bytes(), &parsed2);
    let resolution = resolve(vec![fx1, fx2], Default::default());
    assert!(
        resolution.edges.iter().any(|e| e.kind == EdgeKind::Calls
            && e.confidence == Confidence::Extracted
            && e.dst_name == "widget.Widget"),
        "W() must resolve to widget.Widget: {:?}",
        resolution.edges
    );
    assert!(
        resolution.edges.iter().any(|e| e.kind == EdgeKind::Calls
            && e.confidence == Confidence::Extracted
            && e.dst_name == "widget.Widget.render"),
        "w.render() must resolve: {:?}",
        resolution.edges
    );
    assert_eq!(
        resolution.stats.unresolved_callees, 0,
        "no unresolved left: {:?}",
        resolution.stats
    );
}

#[test]
fn local_variable_shadows_same_named_module() {
    // M2/Amendment 7b: in d.py (module "d"), a local `d = D()` types the
    // receiver — the same-named module interpretation must not eat the call.
    let src = "class D:\n    def m(self):\n        return 1\n\ndef go():\n    d = D()\n    return d.m()\n";
    let parsed = parse_source(src.as_bytes(), Lang::Python).unwrap();
    let fx = extract_file(0, "d.py", src.as_bytes(), &parsed);
    let resolution = resolve(vec![fx], Default::default());
    assert!(
        resolution.edges.iter().any(|e| e.kind == EdgeKind::Calls
            && e.confidence == Confidence::Extracted
            && e.dst_name.contains("D.m")),
        "local-var receiver typing must shadow the module name: {:?}",
        resolution.edges
    );
}

#[test]
fn pep420_namespace_package_import_resolves() {
    // R6d I13: no __init__.py; `import pkg.mod` + `pkg.mod.f()` must resolve
    // through the namespace root to pkg.mod.f.
    let src1 = "def f():\n    return 1\n";
    let src2 = "import pkg.mod\n\ndef go():\n    return pkg.mod.f()\n";
    let p1 = parse_source(src1.as_bytes(), Lang::Python).unwrap();
    let p2 = parse_source(src2.as_bytes(), Lang::Python).unwrap();
    let fx1 = extract_file(0, "pkg/mod.py", src1.as_bytes(), &p1);
    let fx2 = extract_file(1, "use.py", src2.as_bytes(), &p2);
    let resolution = resolve(vec![fx1, fx2], Default::default());
    assert!(
        resolution.edges.iter().any(|e| e.kind == EdgeKind::Calls
            && e.confidence == Confidence::Extracted
            && e.dst_name == "pkg.mod.f"),
        "namespace import must resolve: {:?}",
        resolution.edges
    );
}

#[test]
fn super_calls_resolve_to_base_methods() {
    // Amendment 18: `super().m()`, `super(C, self).m()`, and `Base().m()`
    // must resolve (previously dropped with no edge and no unresolved row).
    let src = "class A:\n    def m(self):\n        return 1\n\nclass B(A):\n    def m(self):\n        return super().m()\n\nclass C(A):\n    def n(self):\n        return super(C, self).m()\n\ndef use():\n    return B().m()\n";
    let parsed = parse_source(src.as_bytes(), Lang::Python).unwrap();
    let fx = extract_file(0, "s.py", src.as_bytes(), &parsed);
    let res = resolve(vec![fx], Default::default());
    let names: Vec<(Confidence, &str)> = res
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Calls)
        .map(|e| (e.confidence, e.dst_name.as_str()))
        .collect();
    let sites: Vec<usize> = res
        .edges
        .iter()
        .filter(|e| {
            e.kind == EdgeKind::Calls
                && e.confidence == Confidence::Extracted
                && e.dst_name.ends_with("A.m")
        })
        .map(|e| e.site_line)
        .collect();
    for line in [7, 11] {
        assert!(
            sites.contains(&line),
            "super call at line {line} must reach A.m: {names:?}"
        );
    }
    // B().m() lands on B's override, not the base method.
    let b_sites: Vec<usize> = res
        .edges
        .iter()
        .filter(|e| {
            e.kind == EdgeKind::Calls
                && e.confidence == Confidence::Extracted
                && e.dst_name.ends_with("B.m")
        })
        .map(|e| e.site_line)
        .collect();
    assert!(
        b_sites.contains(&14),
        "constructor receiver at line 14 must reach B.m: {names:?}"
    );
}

#[test]
fn class_re_export_base_resolves() {
    // Amendment 18: `from pkg import Base` where pkg re-exports Base from
    // pkg.base must produce the inherits edge and resolve self.helper().
    let base = "class Base:\n    def helper(self):\n        return 1\n";
    let init = "from .base import Base\n";
    let app = "from pkg import Base\n\nclass Sub(Base):\n    def run(self):\n        return self.helper()\n";
    let p1 = parse_source(base.as_bytes(), Lang::Python).unwrap();
    let p2 = parse_source(init.as_bytes(), Lang::Python).unwrap();
    let p3 = parse_source(app.as_bytes(), Lang::Python).unwrap();
    let fx1 = extract_file(0, "pkg/base.py", base.as_bytes(), &p1);
    let fx2 = extract_file(1, "pkg/__init__.py", init.as_bytes(), &p2);
    let fx3 = extract_file(2, "app.py", app.as_bytes(), &p3);
    let res = resolve(vec![fx1, fx2, fx3], Default::default());
    assert!(
        res.edges
            .iter()
            .any(|e| e.kind == EdgeKind::Inherits && e.dst_name == "pkg.base.Base"),
        "inherits edge through re-export: {:?}",
        res.edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Inherits)
            .collect::<Vec<_>>()
    );
    assert!(
        res.edges.iter().any(|e| e.kind == EdgeKind::Calls
            && e.confidence == Confidence::Extracted
            && e.dst_name == "pkg.base.Base.helper"),
        "self.helper() must resolve through the re-exported base: {:?}",
        res.edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect::<Vec<_>>()
    );
}

#[test]
fn python_nested_def_scopes_to_enclosing_function() {
    // R7: a nested def belongs to its enclosing function, not the class
    // (the old class-first parent order emitted `pkg.mod.outer.inner` as
    // `pkg.mod.inner` when outer was a method).
    let src = "class C:\n    def outer(self):\n        def inner():\n            return 1\n        return inner()\n";
    let parsed = parse_source(src.as_bytes(), Lang::Python).unwrap();
    let fx = extract_file(0, "pkg/mod.py", src.as_bytes(), &parsed);
    let qns: Vec<&str> = fx.symbols.iter().map(|s| s.qualname.as_str()).collect();
    assert!(qns.contains(&"pkg.mod.C.outer.inner"), "{qns:?}");
}
