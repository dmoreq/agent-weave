//! Build script: derives GRAMMAR_VERSION from the pinned Cargo.lock (R6d A8).
//! The grammar identity gates the fast path, so it must not be a
//! hand-maintained string that can drift from the locked crates.
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=Cargo.lock");
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let lock = std::fs::read_to_string(Path::new(&dir).join("Cargo.lock")).unwrap_or_default();
    let mut versions: std::collections::HashMap<String, String> = Default::default();
    let mut name = String::new();
    for line in lock.lines() {
        if let Some(rest) = line.strip_prefix("name = ") {
            name = rest.trim().trim_matches('"').to_string();
        } else if let Some(rest) = line.strip_prefix("version = ") {
            versions.insert(name.clone(), rest.trim().trim_matches('"').to_string());
        }
    }
    let pair = |crate_name: &str, short: &str| {
        versions
            .get(crate_name)
            .map(|v| format!("{short}-{v}"))
            .unwrap_or_else(|| format!("{short}-unknown"))
    };
    let value = format!(
        "{}+{}+{}+{}",
        pair("tree-sitter-python", "py"),
        pair("tree-sitter-typescript", "ts"),
        pair("tree-sitter-javascript", "js"),
        pair("tree-sitter-rust", "rs")
    );
    println!("cargo:rustc-env=KG_GRAMMAR_VERSION={value}");
}
