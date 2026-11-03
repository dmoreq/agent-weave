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

/// Lib API used by tests and fixtures; the bin uses parse_tree/syntax_stats.
#[allow(dead_code)]
pub struct ParsedFile {
    pub tree: Tree,
    pub error_nodes: usize,
    pub status: &'static str,
}

/// One-pass per-file syntax stats (B1): error/missing nodes, decision nodes
/// (the file-level cyclomatic proxy), comment ranges, and the derived
/// comment mask + non-comment LOC. The build previously walked the tree once
/// per metric; this collapses those passes into one.
pub struct SyntaxStats {
    pub error_nodes: usize,
    pub status: &'static str,
    pub decisions: usize,
    pub loc: i64,
    pub code_lines: Vec<bool>,
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

/// Parse without counting error nodes; `syntax_stats` supplies the count from
/// the same walk that counts decisions and collects comments.
pub fn parse_tree(src: &[u8], lang: Lang) -> Result<Tree> {
    ParserSlot::new().parse(src, lang)
}

#[allow(dead_code)]
pub fn parse_source(src: &[u8], lang: Lang) -> Result<ParsedFile> {
    let tree = parse_tree(src, lang)?;
    let error_nodes = count_error_nodes(tree.root_node());
    let status = if error_nodes == 0 { "ok" } else { "partial" };
    Ok(ParsedFile {
        tree,
        error_nodes,
        status,
    })
}

/// Single tree walk producing every per-file syntax metric. Comment ranges
/// feed the same two line algorithms the separate passes used, so `loc` and
/// `code_lines` stay byte-identical.
pub fn syntax_stats(root: Node<'_>, src: &[u8], lang: Lang) -> SyntaxStats {
    let kinds = decision_kinds(lang);
    let has_marker = src.windows(2).any(|w| w == b"//")
        || src.windows(2).any(|w| w == b"/*")
        || src.contains(&b'#');
    let mut error_nodes = 0usize;
    let mut decisions = 0usize;
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.is_error() || node.is_missing() {
            error_nodes += 1;
        }
        if kinds.contains(&node.kind()) {
            decisions += 1;
        }
        if is_comment_kind(node.kind()) {
            ranges.push((node.start_byte(), node.end_byte()));
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    ranges.sort_unstable();
    let loc = non_comment_loc_from_ranges(src, &ranges);
    let code_lines = if has_marker {
        code_lines_from_ranges(src, &ranges)
    } else {
        blank_code_lines(src)
    };
    SyntaxStats {
        error_nodes,
        status: if error_nodes == 0 { "ok" } else { "partial" },
        decisions,
        loc,
        code_lines,
    }
}

#[allow(dead_code)]
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
            "elif_clause",
            "except_clause",
            "match_statement",
            "case_clause",
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
            "ternary_expression",
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

/// Lines of actual code: blank lines and comment-only lines are excluded.
/// Hotspots' LOC term previously counted every physical line, so a
/// comment-heavy file outranked dense logic (R7 L4). Comment nodes are
/// language-uniform (`comment`) and already in the parse tree.
#[allow(dead_code)]
pub fn non_comment_loc(bytes: &[u8], root: Node<'_>) -> i64 {
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if matches!(
            node.kind(),
            "comment" | "line_comment" | "block_comment" | "doc_comment"
        ) {
            ranges.push((node.start_byte(), node.end_byte()));
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    ranges.sort_unstable();
    let mut ri = 0usize;
    let mut offset = 0usize;
    let mut loc = 0i64;
    for line in bytes.split(|b| *b == b'\n') {
        let first = line.iter().position(|b| !b.is_ascii_whitespace());
        let Some(first) = first else {
            offset += line.len() + 1;
            continue;
        };
        let first = offset + first;
        let line_end = offset + line.len();
        // Advance past comments that ended before this line's first byte.
        while ri < ranges.len() && ranges[ri].1 <= first {
            ri += 1;
        }
        let comment_only = ranges
            .get(ri)
            .is_some_and(|(s, e)| first >= *s && line_end <= *e);
        if !comment_only {
            loc += 1;
        }
        offset += line.len() + 1;
    }
    loc
}

/// Count decision nodes of the given kinds in the subtree.
#[allow(dead_code)]
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

fn is_comment_kind(kind: &str) -> bool {
    matches!(
        kind,
        "comment" | "line_comment" | "block_comment" | "doc_comment"
    )
}

/// `non_comment_loc` over already-collected, sorted comment ranges.
fn non_comment_loc_from_ranges(bytes: &[u8], ranges: &[(usize, usize)]) -> i64 {
    let mut ri = 0usize;
    let mut offset = 0usize;
    let mut loc = 0i64;
    for line in bytes.split(|b| *b == b'\n') {
        let first = line.iter().position(|b| !b.is_ascii_whitespace());
        let Some(first) = first else {
            offset += line.len() + 1;
            continue;
        };
        let first = offset + first;
        let line_end = offset + line.len();
        // Advance past comments that ended before this line's first byte.
        while ri < ranges.len() && ranges[ri].1 <= first {
            ri += 1;
        }
        let comment_only = ranges
            .get(ri)
            .is_some_and(|(s, e)| first >= *s && line_end <= *e);
        if !comment_only {
            loc += 1;
        }
        offset += line.len() + 1;
    }
    loc
}

/// File-wide per-line code flag over already-collected comment ranges. Merges
/// touching ranges first, exactly like the old `file_code_lines` pass.
fn code_lines_from_ranges(src: &[u8], ranges: &[(usize, usize)]) -> Vec<bool> {
    code_lines_from_ranges_work(src, ranges).0
}

/// Same scan plus a work counter: one unit per inspected byte and per advanced
/// range cursor. Tests use it to verify near-linear growth without wall-clock
/// timing.
fn code_lines_from_ranges_work(src: &[u8], ranges: &[(usize, usize)]) -> (Vec<bool>, usize) {
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for &(start, end) in ranges {
        match merged.last_mut() {
            Some((_, last_end)) if start <= *last_end => *last_end = (*last_end).max(end),
            _ => merged.push((start, end)),
        }
    }
    let bounds = line_byte_bounds(src);
    let mut flags = Vec::with_capacity(bounds.len());
    let mut range_idx = 0usize;
    let mut work = 0usize;
    for (start, end) in bounds {
        while range_idx < merged.len() && merged[range_idx].1 <= start {
            range_idx += 1;
            work += 1;
        }
        let mut code = false;
        let mut idx = range_idx;
        for (i, byte) in src[start..end].iter().enumerate() {
            work += 1;
            if !byte.is_ascii_whitespace() {
                let pos = start + i;
                while idx < merged.len() && merged[idx].1 <= pos {
                    idx += 1;
                    work += 1;
                }
                if idx >= merged.len() || pos < merged[idx].0 {
                    code = true;
                    break;
                }
            }
        }
        flags.push(code);
    }
    (flags, work)
}

/// Per-line `(start, end)` byte bounds, excluding the newline.
pub(crate) fn line_byte_bounds(src: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (i, byte) in src.iter().enumerate() {
        if *byte == b'\n' {
            out.push((start, i));
            start = i + 1;
        }
    }
    out.push((start, src.len()));
    out
}

/// Blank-only LOC flags: a line is code when it has any non-whitespace byte.
fn blank_code_lines(src: &[u8]) -> Vec<bool> {
    let mut flags = Vec::new();
    let mut code = false;
    for byte in src {
        if *byte == b'\n' {
            flags.push(code);
            code = false;
        } else if !byte.is_ascii_whitespace() {
            code = true;
        }
    }
    flags.push(code);
    flags
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
    fn non_comment_loc_excludes_blank_and_comment_lines() {
        let src = b"# lead\n\ndef f():\n    # inner\n    return 1\n\n# tail\n";
        let p = parse_source(src, Lang::Python).unwrap();
        assert_eq!(non_comment_loc(src, p.tree.root_node()), 2);
    }

    #[test]
    fn python_elif_and_match_cases_are_decisions() {
        // R7 L4: `elif` is a nested clause and each match case is a branch;
        // counting only the outer if/match undercounted both.
        let src = b"def f(x):\n    if x == 1:\n        return 1\n    elif x == 2:\n        return 2\n    else:\n        return 0\n\ndef g(y):\n    match y:\n        case 1:\n            return 1\n        case _:\n            return 0\n";
        let p = parse_source(src, Lang::Python).unwrap();
        let n = decision_nodes(p.tree.root_node(), decision_kinds(Lang::Python));
        assert_eq!(n, 5, "if + elif + match + 2 cases");
    }

    #[test]
    fn line_numbers_are_one_based_on_emission_boundary() {
        // Extraction offsets rows at emission; parse layer only exposes the tree.
        let p = parse_source(b"a = 1\nb = 2\n", Lang::Python).unwrap();
        assert_eq!(p.tree.root_node().child(0).unwrap().start_position().row, 0);
    }

    #[test]
    fn comment_scan_work_is_near_linear() {
        fn fixture(lines: usize) -> (Vec<u8>, Vec<(usize, usize)>) {
            let text = b"// comment line";
            let mut src = Vec::new();
            let mut ranges = Vec::new();
            for _ in 0..lines {
                let start = src.len();
                src.extend_from_slice(text);
                ranges.push((start, start + text.len()));
                src.push(b'\n');
            }
            (src, ranges)
        }
        let (src1, ranges1) = fixture(1000);
        let (src2, ranges2) = fixture(2000);
        let (_, work1) = code_lines_from_ranges_work(&src1, &ranges1);
        let (_, work2) = code_lines_from_ranges_work(&src2, &ranges2);
        assert!(work1 > 0);
        assert!(
            work2 <= work1 * 3,
            "comment scan work grew superlinearly: n={work1}, 2n={work2}"
        );
    }
}
