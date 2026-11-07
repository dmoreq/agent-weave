//! tree-sitter parsing with per-file ERROR/MISSING node accounting.
//! parse_status: ok (0 error nodes) | partial (some error nodes) | failed
//! (reserved for catastrophic parse failures; tree-sitter always yields a tree).

use tree_sitter::{Language, Node, Parser, Tree};

use crate::error::{AwError, Result};

/// Source language, derived from the file extension (P2: TS/TSX/JSX added).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Python,
    TypeScript,
    Tsx,
    JavaScript,
    Rust,
}

/// The single source of truth for source-extension vocabulary: one row per
/// supported extension, dotted as it appears in paths, with its language.
/// Every other surface derives from this table — the walk allowlist
/// (`walk::source_exts`), `Lang::from_rel`, module stems and `index.*`
/// package markers (`extract::module_path_for`), the TS/JS import resolution
/// order (`extract::walk::typescript`), and the path guards in `query` — so
/// adding an extension is one line here.
pub const SOURCE_EXTENSIONS: &[(&str, Lang)] = &[
    (".py", Lang::Python),
    (".ts", Lang::TypeScript),
    (".tsx", Lang::Tsx),
    (".js", Lang::JavaScript),
    (".jsx", Lang::JavaScript),
    (".mts", Lang::TypeScript),
    (".cts", Lang::TypeScript),
    (".mjs", Lang::JavaScript),
    (".cjs", Lang::JavaScript),
    (".rs", Lang::Rust),
];

impl Lang {
    /// Language for a repo-relative path; `None` when the extension is not in
    /// `SOURCE_EXTENSIONS`. An unknown extension is the caller's explicit
    /// error — never a silent default language.
    pub fn from_rel(rel: &str) -> Option<Lang> {
        let name = rel.rsplit('/').next().unwrap_or(rel);
        SOURCE_EXTENSIONS
            .iter()
            .find(|(ext, _)| name.ends_with(ext))
            .map(|(_, lang)| *lang)
    }

    /// True for the TS/JS family: import resolution and `index.*` package
    /// markers treat these extensions as one group.
    pub fn is_ts_js(self) -> bool {
        matches!(self, Lang::TypeScript | Lang::Tsx | Lang::JavaScript)
    }
}

/// Bare (dot-less) extension membership: `query` treats a final path segment
/// naming a source extension as a file, not a directory.
pub fn is_source_ext(ext: &str) -> bool {
    SOURCE_EXTENSIONS
        .iter()
        .any(|(dotted, _)| &dotted[1..] == ext)
}

/// Bare extension names in table order (the walk allowlist derives from it).
pub fn source_ext_names() -> impl Iterator<Item = &'static str> {
    SOURCE_EXTENSIONS.iter().map(|(dotted, _)| &dotted[1..])
}

impl Lang {
    pub fn grammar(self) -> Result<Language> {
        let lang: Language = match self {
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
            Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
        };
        Ok(lang)
    }
}

/// Lib API used by tests and fixtures.
#[allow(dead_code)]
pub struct ParsedFile {
    pub tree: Tree,
    pub error_nodes: usize,
    pub status: &'static str,
}

/// One-pass per-file syntax stats: error/missing nodes and status.
pub struct SyntaxStats {
    pub error_nodes: usize,
    pub status: &'static str,
}

/// Worker-local parser (B3): one `Parser` per build worker instead of one per
/// file; the language is reset only when the chunk changes language.
pub struct ParserSlot {
    parser: Parser,
    lang: Option<Lang>,
}

impl Default for ParserSlot {
    fn default() -> ParserSlot {
        ParserSlot::new()
    }
}

impl ParserSlot {
    pub fn new() -> ParserSlot {
        ParserSlot {
            parser: Parser::new(),
            lang: None,
        }
    }

    pub fn parse(&mut self, src: &[u8], lang: Lang) -> Result<Tree> {
        if self.lang != Some(lang) {
            self.parser
                .set_language(&lang.grammar()?)
                .map_err(|e| AwError::Internal(format!("grammar load failed: {e}")))?;
            self.lang = Some(lang);
        }
        self.parser
            .parse(src, None)
            .ok_or_else(|| AwError::Internal("tree-sitter returned no tree".to_string()))
    }
}

/// Test/fixture helper: parse and count error nodes in one call.
#[allow(dead_code)]
pub fn parse_source(src: &[u8], lang: Lang) -> Result<ParsedFile> {
    let tree = ParserSlot::new().parse(src, lang)?;
    let stats = syntax_stats(tree.root_node());
    Ok(ParsedFile {
        tree,
        error_nodes: stats.error_nodes,
        status: stats.status,
    })
}

/// Single tree walk producing error nodes count.
pub fn syntax_stats(root: Node<'_>) -> SyntaxStats {
    let mut error_nodes = 0usize;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.is_error() || node.is_missing() {
            error_nodes += 1;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    SyntaxStats {
        error_nodes,
        status: if error_nodes == 0 { "ok" } else { "partial" },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_source_is_ok() {
        let p = parse_source(b"def f():\n    return 1\n", Lang::Python).unwrap();
        assert_eq!(p.status, "ok");
        assert_eq!(p.error_nodes, 0);
    }

    #[test]
    fn broken_source_counts_error_nodes() {
        let src = b"def f(:\n    pass\n";
        let p = parse_source(src, Lang::Python).unwrap();
        assert_eq!(p.status, "partial");
        assert!(p.error_nodes >= 1);
    }

    #[test]
    fn line_numbers_are_one_based_on_emission_boundary() {
        // Extraction offsets rows at emission; parse layer only exposes the tree.
        let p = parse_source(b"a = 1\nb = 2\n", Lang::Python).unwrap();
        assert_eq!(p.tree.root_node().child(0).unwrap().start_position().row, 0);
    }
}
