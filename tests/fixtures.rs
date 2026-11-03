//! Spec-mandated fixture corpus (R1 fix): star import, getattr dispatch,
//! variable reassignment, `import x as y`, factory-nested constructor
//! (negative), and module-qualified constructor. Each asserts the exact
//! resolution outcome on a minimal fixture.

use agent_weave::extract::extract_file;
use agent_weave::extract::resolve;
use agent_weave::parse::{parse_source, Lang};

fn edges_for(src: &str) -> Vec<(String, String, String)> {
    // (kind, confidence, dst_name) of all calls edges, single-file project.
    let parsed = parse_source(src.as_bytes(), Lang::Python).unwrap();
    let fx = extract_file(0, "pkg/mod.py", src.as_bytes(), &parsed);
    let resolution = resolve(vec![fx], Default::default());
    resolution
        .edges
        .iter()
        .map(|e| {
            (
                e.kind.to_string(),
                e.confidence.to_string(),
                e.dst_name.clone(),
            )
        })
        .collect()
}

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
        e.iter()
            .any(|(k, c, n)| *k == "imports" && *c == "AMBIGUOUS" && n.ends_with(".utils")),
        "star import edge: {e:?}"
    );
    assert!(
        e.iter()
            .any(|(k, c, n)| *k == "calls" && *c == "EXTRACTED" && n.contains("helper")),
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
            e.kind == "calls" && e.confidence == "EXTRACTED" && e.dst_name == "helpers.help_me"
        }),
        "hp.help_me() must resolve to helpers.help_me: {:?}",
        resolution.edges
    );
}

#[test]
fn factory_nested_constructor_types_nothing() {
    // R1: obj = factory(A()) must NOT type obj as A.
    let src = "\
class A:\n\
    def m(self):\n\
        return 1\n\
\n\
def factory(x):\n\
    return x\n\
\n\
def go():\n\
    obj = factory(A())\n\
    return obj.m()\n\
";
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
    let hit = resolution
        .edges
        .iter()
        .any(|e| e.kind == "calls" && e.confidence == "EXTRACTED" && e.dst_name.contains("mod.K"));
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
        resolution.edges.iter().any(|e| e.kind == "calls"
            && e.confidence == "EXTRACTED"
            && e.dst_name == "widget.Widget"),
        "W() must resolve to widget.Widget: {:?}",
        resolution.edges
    );
    assert!(
        resolution.edges.iter().any(|e| e.kind == "calls"
            && e.confidence == "EXTRACTED"
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
fn ts_class_method_and_this_call() {
    // R2: this.m() must resolve like Python self.m() — assert the EDGE.
    let src = "export class Ledger {\n  balance = 0\n  add(n: number): number {\n    return this.normalize(n)\n  }\n  private normalize(n: number): number {\n    return Math.abs(n)\n  }\n}\n";
    let parsed =
        agent_weave::parse::parse_source(src.as_bytes(), agent_weave::parse::Lang::TypeScript)
            .unwrap();
    let fx = agent_weave::extract::extract_file(0, "apps/core/ledger.ts", src.as_bytes(), &parsed);
    let names: Vec<&str> = fx.symbols.iter().map(|s| s.qualname.as_str()).collect();
    assert!(names.contains(&"apps.core.ledger.Ledger"), "{names:?}");
    assert!(names.contains(&"apps.core.ledger.Ledger.add"), "{names:?}");
    assert!(
        names.contains(&"apps.core.ledger.Ledger.normalize"),
        "{names:?}"
    );
    let resolution = agent_weave::extract::resolve(vec![fx], Default::default());
    let hit = resolution.edges.iter().any(|e| {
        e.kind == "calls" && e.confidence == "EXTRACTED" && e.dst_name.contains("Ledger.normalize")
    });
    assert!(
        hit,
        "this.normalize(n) must resolve: {:?}",
        resolution.edges
    );
}

#[test]
fn ts_new_expression_resolves_to_constructor() {
    let src1 = "export class Widget {}\n";
    let src2 = "import { Widget } from './widget'\n\nexport function make(): Widget {\n  const w = new Widget()\n  return w\n}\n";
    let p1 =
        agent_weave::parse::parse_source(src1.as_bytes(), agent_weave::parse::Lang::TypeScript)
            .unwrap();
    let p2 =
        agent_weave::parse::parse_source(src2.as_bytes(), agent_weave::parse::Lang::TypeScript)
            .unwrap();
    let fx1 = agent_weave::extract::extract_file(0, "apps/core/widget.ts", src1.as_bytes(), &p1);
    let fx2 = agent_weave::extract::extract_file(1, "apps/core/factory.ts", src2.as_bytes(), &p2);
    let resolution = agent_weave::extract::resolve(vec![fx1, fx2], Default::default());
    let hit = resolution
        .edges
        .iter()
        .any(|e| e.kind == "calls" && e.confidence == "EXTRACTED" && e.dst_name.contains("Widget"));
    assert!(hit, "new Widget() must resolve: {:?}", resolution.edges);
}

#[test]
fn ts_relative_import_resolves_by_path() {
    let src1 = "export function format(n: number): string {\n  return String(n)\n}\n";
    let src2 = "import { format } from '../lib/format'\n\nexport function run(n: number): string {\n  return format(n)\n}\n";
    let p1 =
        agent_weave::parse::parse_source(src1.as_bytes(), agent_weave::parse::Lang::TypeScript)
            .unwrap();
    let p2 =
        agent_weave::parse::parse_source(src2.as_bytes(), agent_weave::parse::Lang::TypeScript)
            .unwrap();
    let fx1 =
        agent_weave::extract::extract_file(0, "apps/web/src/lib/format.ts", src1.as_bytes(), &p1);
    let fx2 =
        agent_weave::extract::extract_file(1, "apps/web/src/features/run.ts", src2.as_bytes(), &p2);
    let resolution = agent_weave::extract::resolve(vec![fx1, fx2], Default::default());
    let hit = resolution
        .edges
        .iter()
        .any(|e| e.kind == "calls" && e.confidence == "EXTRACTED" && e.dst_name.contains("format"));
    assert!(
        hit,
        "../lib/format import must resolve: {:?}",
        resolution.edges
    );
}

#[test]
fn ts_external_import_calls_drop() {
    let src = "import { useState } from 'react'\n\nexport function useCount() {\n  const [n] = useState(0)\n  return n\n}\n";
    let parsed =
        agent_weave::parse::parse_source(src.as_bytes(), agent_weave::parse::Lang::TypeScript)
            .unwrap();
    let fx =
        agent_weave::extract::extract_file(0, "apps/web/src/useCount.ts", src.as_bytes(), &parsed);
    let resolution = agent_weave::extract::resolve(vec![fx], Default::default());
    assert!(
        !resolution
            .edges
            .iter()
            .any(|e| e.kind == "calls" && e.dst_name.contains("useState")),
        "no useState edge"
    );
    assert!(
        resolution.stats.builtin_dropped >= 1,
        "external call counted as dropped: {:?}",
        resolution.stats
    );
}

#[test]
fn rust_crate_three_segment_call_resolves() {
    // M2/Amendment 7a: crate::a::b must fall back to a direct qualname lookup
    // under the crate root when the intermediate segments are not indexed
    // modules (src/ layout without mod.rs) — the G2-self G01 gap.
    let src1 = "pub fn file_changed() -> bool {\n    true\n}\n";
    let src2 = "pub fn check() -> bool {\n    crate::query::file_changed()\n}\n";
    let parsed1 =
        agent_weave::parse::parse_source(src1.as_bytes(), agent_weave::parse::Lang::Rust).unwrap();
    let parsed2 =
        agent_weave::parse::parse_source(src2.as_bytes(), agent_weave::parse::Lang::Rust).unwrap();
    let fx1 = agent_weave::extract::extract_file(0, "src/query.rs", src1.as_bytes(), &parsed1);
    let fx2 = agent_weave::extract::extract_file(1, "src/doctor.rs", src2.as_bytes(), &parsed2);
    let resolution = resolve(vec![fx1, fx2], Default::default());
    assert!(
        resolution.edges.iter().any(|e| {
            e.kind == "calls"
                && e.confidence == "EXTRACTED"
                && e.dst_name == "src.query.file_changed"
        }),
        "crate::query::file_changed must resolve: {:?}",
        resolution.edges
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
        resolution.edges.iter().any(|e| e.kind == "calls"
            && e.confidence == "EXTRACTED"
            && e.dst_name.contains("D.m")),
        "local-var receiver typing must shadow the module name: {:?}",
        resolution.edges
    );
}

#[test]
fn fts5_is_compiled_into_bundled_sqlite() {
    // Amendment 12b smoke: the search surface needs FTS5; fail fast here if a
    // future rusqlite bundled build drops it.
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    let on: i64 = conn
        .query_row("SELECT sqlite_compileoption_used('ENABLE_FTS5')", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(on, 1, "FTS5 missing from the bundled SQLite build");
}

#[test]
fn split_ident_splits_camel_snake_and_dotted_idents() {
    assert_eq!(
        agent_weave::extract::split_ident("HierarchicalAddressParser.parse"),
        "hierarchical address parser parse"
    );
    assert_eq!(
        agent_weave::extract::split_ident("get_knowledge_base"),
        "get knowledge base"
    );
    assert_eq!(agent_weave::extract::split_ident("FNV-1a"), "fnv 1a");
    assert_eq!(
        agent_weave::extract::split_ident("normalize_ascii"),
        "normalize ascii"
    );
    assert_eq!(agent_weave::extract::split_ident(""), "");
}

#[test]
fn rust_impl_methods_scope_to_the_impl_type() {
    // R6d A5: `impl Foo { fn make() -> Bar }` must yield m.Foo.make, not
    // m.Bar.make; `impl Bar { fn go(&self, other: Foo) -> Foo }` must yield
    // m.Bar.go, not m.Foo.go.
    let src = "struct Foo;\nstruct Bar;\n\nimpl Foo {\n    fn make() -> Bar {\n        Bar\n    }\n    fn plain() {}\n}\n\nimpl Bar {\n    fn go(&self, other: Foo) -> Foo {\n        other\n    }\n}\n";
    let parsed =
        agent_weave::parse::parse_source(src.as_bytes(), agent_weave::parse::Lang::Rust).unwrap();
    let fx = agent_weave::extract::extract_file(0, "src/m.rs", src.as_bytes(), &parsed);
    let quals: Vec<&str> = fx.symbols.iter().map(|s| s.qualname.as_str()).collect();
    assert!(quals.contains(&"src.m.Foo.make"), "{quals:?}");
    assert!(quals.contains(&"src.m.Foo.plain"), "{quals:?}");
    assert!(quals.contains(&"src.m.Bar.go"), "{quals:?}");
    assert!(!quals.contains(&"src.m.Bar.make"), "{quals:?}");
    assert!(!quals.contains(&"src.m.Foo.go"), "{quals:?}");
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
        resolution
            .edges
            .iter()
            .any(|e| e.kind == "calls" && e.confidence == "EXTRACTED" && e.dst_name == "pkg.mod.f"),
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
    let names: Vec<(&str, &str)> = res
        .edges
        .iter()
        .filter(|e| e.kind == "calls")
        .map(|e| (e.confidence, e.dst_name.as_str()))
        .collect();
    let sites: Vec<usize> = res
        .edges
        .iter()
        .filter(|e| e.kind == "calls" && e.confidence == "EXTRACTED" && e.dst_name.ends_with("A.m"))
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
        .filter(|e| e.kind == "calls" && e.confidence == "EXTRACTED" && e.dst_name.ends_with("B.m"))
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
            .any(|e| e.kind == "inherits" && e.dst_name == "pkg.base.Base"),
        "inherits edge through re-export: {:?}",
        res.edges
            .iter()
            .filter(|e| e.kind == "inherits")
            .collect::<Vec<_>>()
    );
    assert!(
        res.edges.iter().any(|e| e.kind == "calls"
            && e.confidence == "EXTRACTED"
            && e.dst_name == "pkg.base.Base.helper"),
        "self.helper() must resolve through the re-exported base: {:?}",
        res.edges
            .iter()
            .filter(|e| e.kind == "calls")
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

#[test]
fn rust_inline_mods_are_scopes() {
    // R7 F1: inline `mod alpha { fn shared }` must qualify as
    // `pkg.mod.alpha.shared`; two mods may hold same-named items.
    let src = "mod alpha {\n    fn shared() {}\n}\nmod beta {\n    fn shared() {}\n}\n";
    let parsed = parse_source(src.as_bytes(), Lang::Rust).unwrap();
    let fx = extract_file(0, "pkg/mod.rs", src.as_bytes(), &parsed);
    let qns: Vec<&str> = fx.symbols.iter().map(|s| s.qualname.as_str()).collect();
    assert!(qns.iter().any(|q| q.ends_with(".alpha.shared")), "{qns:?}");
    assert!(qns.iter().any(|q| q.ends_with(".beta.shared")), "{qns:?}");
    assert!(
        !qns.contains(&"pkg.mod.shared"),
        "shared leaked to module scope: {qns:?}"
    );
    let mut uniq = qns.clone();
    uniq.sort_unstable();
    uniq.dedup();
    assert_eq!(uniq.len(), qns.len(), "duplicate qualnames: {qns:?}");
}

#[test]
fn typescript_namespaces_are_scopes() {
    let src = "namespace A { export function shared() {} }\nnamespace B { export function shared() {} }\n";
    let parsed = parse_source(src.as_bytes(), Lang::TypeScript).unwrap();
    let fx = extract_file(0, "pkg/mod.ts", src.as_bytes(), &parsed);
    let qns: Vec<&str> = fx.symbols.iter().map(|s| s.qualname.as_str()).collect();
    assert!(qns.iter().any(|q| q.ends_with(".A.shared")), "{qns:?}");
    assert!(qns.iter().any(|q| q.ends_with(".B.shared")), "{qns:?}");
}

#[test]
fn signatures_span_multiline_parameters_and_stop_before_body() {
    // R7 L3: 16.5% of Python signatures ended at `(` because only the first
    // line was stored; the whole parameter list must survive, without the
    // body or a trailing colon/brace.
    let py = "def render(\n    self,\n    item,\n    width=10,\n):\n    return item\n";
    let parsed = parse_source(py.as_bytes(), Lang::Python).unwrap();
    let fx = extract_file(0, "pkg/mod.py", py.as_bytes(), &parsed);
    let sig = &fx
        .symbols
        .iter()
        .find(|s| s.name == "render")
        .unwrap()
        .signature;
    assert!(sig.contains("width=10"), "signature truncated: {sig:?}");
    assert!(
        !sig.contains("return"),
        "body leaked into signature: {sig:?}"
    );
    assert!(!sig.ends_with(':'), "colon retained: {sig:?}");

    let rust = "fn render(\n    item: Item,\n    width: usize,\n) -> String {\n    item.name\n}\n";
    let parsed = parse_source(rust.as_bytes(), Lang::Rust).unwrap();
    let fx = extract_file(0, "pkg/mod.rs", rust.as_bytes(), &parsed);
    let sig = &fx
        .symbols
        .iter()
        .find(|s| s.name == "render")
        .unwrap()
        .signature;
    assert!(sig.contains("width: usize"), "signature truncated: {sig:?}");
    assert!(sig.contains("-> String"), "return type lost: {sig:?}");
    assert!(!sig.contains("item.name"), "body leaked: {sig:?}");
}

#[test]
fn acronym_identifiers_split_at_the_case_boundary() {
    use agent_weave::extract::split_ident;
    assert_eq!(split_ident("HTTPServer"), "http server");
    assert_eq!(split_ident("getHTTPResponse"), "get http response");
    assert_eq!(split_ident("parseS3Uploader"), "parse s3 uploader");
}
