//! tree-sitter parsing with per-file ERROR/MISSING node accounting.
//! parse_status: ok (0 error nodes) | partial (some error nodes) | failed
//! (reserved for catastrophic parse failures; tree-sitter always yields a tree).

use tree_sitter::{Language, Node, Parser, Tree};

use crate::error::{KgError, Result};

/// Source language, derived from the file extension (P2: TS/TSX/JSX added).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Python,
    TypeScript,
    Tsx,
    JavaScript,
    Rust,
}

impl Lang {
    pub fn from_rel(rel: &str) -> Lang {
        let name = rel.rsplit('/').next().unwrap_or(rel);
        if name.ends_with(".tsx") {
            Lang::Tsx
        } else if name.ends_with(".ts") || name.ends_with(".mts") || name.ends_with(".cts") {
            Lang::TypeScript
        } else if name.ends_with(".jsx") || name.ends_with(".mjs") || name.ends_with(".cjs") {
            Lang::JavaScript
        } else if name.ends_with(".py") {
            Lang::Python
        } else if name.ends_with(".rs") {
            Lang::Rust
        } else {
            Lang::JavaScript
        }
    }

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

pub struct ParsedFile {
    pub tree: Tree,
    pub error_nodes: usize,
    pub status: &'static str,
}

pub fn parse_source(src: &[u8], lang: Lang) -> Result<ParsedFile> {
    let mut parser = Parser::new();
    parser
        .set_language(&lang.grammar()?)
        .map_err(|e| KgError::Internal(format!("grammar load failed: {e}")))?;
    let tree = parser
        .parse(src, None)
        .ok_or_else(|| KgError::Internal("tree-sitter returned no tree".to_string()))?;
    let error_nodes = count_error_nodes(tree.root_node());
    let status = if error_nodes == 0 { "ok" } else { "partial" };
    Ok(ParsedFile { tree, error_nodes, status })
}

fn count_error_nodes(root: Node<'_>) -> usize {
    // Explicit stack: source nesting depth is attacker-controlled, and a
    // recursive walk aborts the process on pathological files (R6d Blocker 1).
    let mut n = 0usize;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        n += usize::from(node.is_error() || node.is_missing());
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    n
}

/// Language-aware cyclomatic-proxy decision kinds (hotspots metric, G7).
/// Lives beside the parser so the build's parse phase (Amendment 11d) can
/// count decision nodes on the tree it already has.
pub fn decision_kinds(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::Python => &[
            "if_statement",
            "for_statement",
            "while_statement",
            "boolean_operator",
            "except_clause",
            "match_statement",
            "conditional_expression",
        ],
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => &[
            "if_statement",
            "for_statement",
            "for_in_statement",
            "while_statement",
            "do_statement",
            "switch_case",
            "conditional_expression",
        ],
        Lang::Rust => &[
            "if_expression",
            "if_let_expression",
            "for_expression",
            "while_expression",
            "while_let_expression",
            "loop_expression",
            "match_arm",
        ],
    }
}

/// Count decision nodes of the given kinds in the subtree.
pub fn decision_nodes(root: Node<'_>, kinds: &[&str]) -> usize {
    // Explicit stack — same depth-safety rationale as count_error_nodes.
    let mut n = 0usize;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        n += usize::from(kinds.contains(&node.kind()));
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    n
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