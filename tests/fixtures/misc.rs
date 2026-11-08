//! Language-agnostic checks: signatures, identifier splitting, FTS5
//! availability, and deep-nesting stack safety.

use agent_weave::extract::{extract_file, SymbolKind};
use agent_weave::parse::{parse_source, Lang};

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

#[test]
fn deep_call_nesting_stays_iterative() {
    // A recursive walk re-enters walk_children once per nested call and
    // overflows the default test-thread stack at this size (SIGABRT). The
    // traversal must stay heap-bound.
    let depth = 10_000;
    let src = format!("{}1{}\n", "f(".repeat(depth), ")".repeat(depth));
    let parsed = parse_source(src.as_bytes(), Lang::Python).unwrap();
    let fx = extract_file(0, "deep.py", src.as_bytes(), &parsed);
    assert!(fx.symbols.iter().all(|s| s.kind == SymbolKind::Module));
}
