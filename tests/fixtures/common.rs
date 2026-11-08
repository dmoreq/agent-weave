//! Shared extraction helpers for the fixture corpus tests.

use agent_weave::extract::{extract_file, resolve, EdgeKind};
use agent_weave::parse::{parse_source, Lang};

pub(crate) fn edges_for(src: &str) -> Vec<(String, String, String)> {
    // (kind, confidence, dst_name) of all calls edges, single-file project.
    let parsed = parse_source(src.as_bytes(), Lang::Python).unwrap();
    let fx = extract_file(0, "pkg/mod.py", src.as_bytes(), &parsed);
    let resolution = resolve(vec![fx], Default::default());
    resolution
        .edges
        .iter()
        .map(|e| {
            (
                e.kind.as_str().to_string(),
                e.confidence.as_str().to_string(),
                e.dst_name.clone(),
            )
        })
        .collect()
}

/// Resolve a set of Rust files and return (confidence, dst_name) for call edges.
pub(crate) fn rust_call_edges(files: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut extractions = Vec::new();
    for (i, (rel, src)) in files.iter().enumerate() {
        let parsed = parse_source(src.as_bytes(), Lang::Rust).unwrap();
        extractions.push(extract_file(i, rel, src.as_bytes(), &parsed));
    }
    resolve(extractions, Default::default())
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Calls)
        .map(|e| (e.confidence.as_str().to_string(), e.dst_name.clone()))
        .collect()
}
