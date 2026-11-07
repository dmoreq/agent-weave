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

/// One-pass per-file syntax stats (B1): error/missing nodes, decision nodes
/// (the file-level cyclomatic proxy), comment ranges, and the derived
/// comment mask + non-comment LOC. The build previously walked the tree once
/// per metric; this collapses those passes into one.
pub struct SyntaxStats {
    pub error_nodes: usize,
    pub status: &'static str,
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
    let stats = syntax_stats(tree.root_node(), src);
    Ok(ParsedFile {
        tree,
        error_nodes: stats.error_nodes,
        status: stats.status,
    })
}

/// Single tree walk producing the per-file syntax facts the build consumes:
/// error nodes and the comment-derived code-line mask used for effective LOC.
pub fn syntax_stats(root: Node<'_>, src: &[u8]) -> SyntaxStats {
    let has_marker = src.windows(2).any(|w| w == b"//")
        || src.windows(2).any(|w| w == b"/*")
        || src.contains(&b'#');
    let mut error_nodes = 0usize;
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    // Explicit stack: source nesting depth is attacker-controlled, and a
    // recursive walk aborts the process on pathological files (R6d Blocker 1).
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.is_error() || node.is_missing() {
            error_nodes += 1;
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
    let code_lines = if has_marker {
        code_lines_from_ranges(src, &ranges)
    } else {
        blank_code_lines(src)
    };
    SyntaxStats {
        error_nodes,
        status: if error_nodes == 0 { "ok" } else { "partial" },
        code_lines,
    }
}

fn is_comment_kind(kind: &str) -> bool {
    matches!(
        kind,
        "comment" | "line_comment" | "block_comment" | "doc_comment"
    )
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
