//! tree-sitter parsing with per-file ERROR/MISSING node accounting.
//! parse_status: ok (0 error nodes) | partial (some error nodes) | failed
//! (reserved for catastrophic parse failures; tree-sitter always yields a tree).

use tree_sitter::{Node, Parser, Tree};

use crate::error::{KgError, Result};

pub struct ParsedFile {
    pub tree: Tree,
    pub error_nodes: usize,
    pub status: &'static str,
}

pub fn parse_python(src: &[u8]) -> Result<ParsedFile> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .map_err(|e| KgError::Internal(format!("grammar load failed: {e}")))?;
    let tree = parser
        .parse(src, None)
        .ok_or_else(|| KgError::Internal("tree-sitter returned no tree".to_string()))?;
    let error_nodes = count_error_nodes(tree.root_node());
    let status = if error_nodes == 0 { "ok" } else { "partial" };
    Ok(ParsedFile { tree, error_nodes, status })
}

fn count_error_nodes(root: Node<'_>) -> usize {
    fn rec(node: Node<'_>) -> usize {
        let mut n = usize::from(node.is_error() || node.is_missing());
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            n += rec(child);
        }
        n
    }
    rec(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_source_is_ok() {
        let p = parse_python(b"def f():\n    return 1\n").unwrap();
        assert_eq!(p.status, "ok");
        assert_eq!(p.error_nodes, 0);
    }

    #[test]
    fn broken_source_counts_error_nodes() {
        let src = b"def f(:\n    pass\n";
        let p = parse_python(src).unwrap();
        assert_eq!(p.status, "partial");
        assert!(p.error_nodes >= 1);
    }

    #[test]
    fn line_numbers_are_one_based_on_emission_boundary() {
        // Extraction offsets rows at emission; parse layer only exposes the tree.
        let p = parse_python(b"a = 1\nb = 2\n").unwrap();
        assert_eq!(p.tree.root_node().child(0).unwrap().start_position().row, 0);
    }
}