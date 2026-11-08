//! Rust resolution fixtures: impl methods, inline mods, and path resolution.

use agent_weave::extract::{extract_file, resolve, Confidence, EdgeKind, SymbolKind};
use agent_weave::parse::{parse_source, Lang};

use super::common::rust_call_edges;

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
            e.kind == EdgeKind::Calls
                && e.confidence == Confidence::Extracted
                && e.dst_name == "src.query.file_changed"
        }),
        "crate::query::file_changed must resolve: {:?}",
        resolution.edges
    );
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
fn rust_self_receiver_call_resolves() {
    let src = "struct Pointer;\nimpl Pointer {\n    fn render(&self) -> i32 { 1 }\n    fn go(&self) -> i32 { self.render() }\n}\n";
    let edges = rust_call_edges(&[("src/query.rs", src)]);
    assert!(
        edges
            .iter()
            .any(|(c, n)| c == "EXTRACTED" && n == "src.query.Pointer.render"),
        "self.render() must resolve to the impl method: {edges:?}"
    );
}

#[test]
fn rust_typed_parameter_receiver_resolves() {
    let src = "struct Pointer;\nimpl Pointer {\n    fn render(&self) -> i32 { 1 }\n}\nfn go(p: &Pointer) -> i32 { p.render() }\n";
    let edges = rust_call_edges(&[("src/query.rs", src)]);
    assert!(
        edges
            .iter()
            .any(|(c, n)| c == "EXTRACTED" && n == "src.query.Pointer.render"),
        "&Pointer parameter must type p: {edges:?}"
    );
}

#[test]
fn rust_let_constructor_receiver_resolves() {
    let src = "struct Pointer;\nimpl Pointer {\n    fn new() -> Pointer { Pointer }\n    fn render(&self) -> i32 { 1 }\n}\nfn go() -> i32 { let p = Pointer::new(); p.render() }\n";
    let edges = rust_call_edges(&[("src/query.rs", src)]);
    assert!(
        edges
            .iter()
            .any(|(c, n)| c == "EXTRACTED" && n == "src.query.Pointer.render"),
        "constructed local must type p: {edges:?}"
    );
}

#[test]
fn rust_self_assoc_and_super_module_calls_resolve() {
    let lib = "struct Foo;\nimpl Foo {\n    fn make() -> Foo { Foo }\n    fn go() -> Foo { Self::make() }\n}\npub fn helper() -> i32 { 1 }\n";
    let child = "fn go() -> i32 { super::helper() }\n";
    let edges = rust_call_edges(&[("src/lib.rs", lib), ("src/child.rs", child)]);
    assert!(
        edges
            .iter()
            .any(|(c, n)| c == "EXTRACTED" && n == "src.Foo.make"),
        "Self::make() must resolve through the impl target: {edges:?}"
    );
    assert!(
        edges
            .iter()
            .any(|(c, n)| c == "EXTRACTED" && n == "src.helper"),
        "super::helper() must resolve in the parent module: {edges:?}"
    );
}

#[test]
fn rust_inline_mod_functions_keep_function_kind() {
    let src = "mod child {\n    pub fn helper() -> i32 { 1 }\n}\n";
    let parsed = parse_source(src.as_bytes(), Lang::Rust).unwrap();
    let fx = extract_file(0, "src/lib.rs", src.as_bytes(), &parsed);
    let helper = fx
        .symbols
        .iter()
        .find(|s| s.name == "helper")
        .expect("helper symbol");
    assert_eq!(helper.qualname, "src.child.helper");
    assert_eq!(
        helper.kind,
        SymbolKind::Function,
        "free function in mod is not a method"
    );
}

#[test]
fn rust_unknown_receiver_without_candidates_stays_unresolved() {
    let src = "struct Pointer;\nimpl Pointer {\n    fn render(&self) -> i32 { 1 }\n}\nfn go(x: External) -> i32 { x.mystery() }\n";
    let edges = rust_call_edges(&[("src/query.rs", src)]);
    assert!(
        !edges.iter().any(|(_, n)| n.contains("mystery")),
        "unknown receiver must not manufacture an edge: {edges:?}"
    );
}

#[test]
fn rust_unknown_receiver_does_not_fan_out_ambiguous() {
    let src = "struct A;\nstruct B;\nimpl A { fn render(&self) -> i32 { 1 } }\nimpl B { fn render(&self) -> i32 { 2 } }\nfn go(p: Unknown) -> i32 { p.render() }\n";
    let edges = rust_call_edges(&[("src/query.rs", src)]);
    assert!(
        !edges
            .iter()
            .any(|(c, n)| c == "AMBIGUOUS" && n.ends_with(".render")),
        "several same-name impls must stay unresolved, not AMBIGUOUS: {edges:?}"
    );
}

#[test]
fn rust_inline_mod_self_and_super_paths_resolve() {
    let src = "mod child {\n    fn local() -> i32 { 1 }\n    fn go() -> i32 { self::local() + super::outer() }\n}\nfn outer() -> i32 { 2 }\n";
    let edges = rust_call_edges(&[("src/lib.rs", src)]);
    assert!(
        edges
            .iter()
            .any(|(c, n)| c == "EXTRACTED" && n == "src.child.local"),
        "self::local() must resolve inside the inline mod: {edges:?}"
    );
    assert!(
        edges
            .iter()
            .any(|(c, n)| c == "EXTRACTED" && n == "src.outer"),
        "super::outer() must resolve in the parent module: {edges:?}"
    );
}
