//! Symbol/edge extraction and the call-target resolution algorithm (core IP).
//!
//! Two-phase design:
//! Phase A (per file, single tree walk): symbols (module/class/function/method),
//! contains edges, inherits candidates, import bindings, typed parameters,
//! constructor assignments, and raw call sites.
//! Phase B (global): resolution over merged symbol tables — first match wins
//! per the spec order: local module -> import alias map -> receiver-class
//! method -> builtin table.
//!
//! Outcome: exactly one candidate -> EXTRACTED; multiple -> AMBIGUOUS
//! (dst_id NULL, dst_name carries the dotted callee); zero -> no edge, counted
//! unresolved. Dynamic dispatch (unknown receivers, getattr) is a non-goal.

use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use crate::parse::{Lang, ParsedFile};

pub const SCHEMA_VERSION: i32 = 7;

/// Python builtins: calls drop with the builtin counter (spec step 4).
const BUILTINS: &[&str] = &[
    "abs",
    "aiter",
    "all",
    "any",
    "anext",
    "ascii",
    "bin",
    "bool",
    "bytearray",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "hex",
    "id",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
    "__import__",
    "Exception",
    "BaseException",
    "ArithmeticError",
    "AssertionError",
    "AttributeError",
    "BlockingIOError",
    "BrokenPipeError",
    "BufferError",
    "ChildProcessError",
    "ConnectionError",
    "ConnectionRefusedError",
    "ConnectionResetError",
    "EOFError",
    "FileExistsError",
    "FileNotFoundError",
    "FloatingPointError",
    "GeneratorExit",
    "ImportError",
    "IndentationError",
    "IndexError",
    "InterruptedError",
    "IsADirectoryError",
    "KeyError",
    "KeyboardInterrupt",
    "LookupError",
    "MemoryError",
    "ModuleNotFoundError",
    "NameError",
    "NotADirectoryError",
    "NotImplementedError",
    "OSError",
    "OverflowError",
    "PermissionError",
    "ProcessLookupError",
    "RecursionError",
    "ReferenceError",
    "RuntimeError",
    "StopAsyncIteration",
    "StopIteration",
    "SyntaxError",
    "SystemError",
    "SystemExit",
    "TabError",
    "TimeoutError",
    "TypeError",
    "UnboundLocalError",
    "UnicodeDecodeError",
    "UnicodeEncodeError",
    "UnicodeError",
    "UnicodeTranslateError",
    "ValueError",
    "ZeroDivisionError",
];

/// Well-known stdlib module roots: `os.path.join(...)` on unindexed externals
/// drops with the builtin counter instead of flooding the unresolved count.
const STDLIB_MODULES: &[&str] = &[
    "os",
    "sys",
    "re",
    "json",
    "math",
    "time",
    "datetime",
    "logging",
    "pathlib",
    "collections",
    "itertools",
    "functools",
    "typing",
    "dataclasses",
    "enum",
    "abc",
    "io",
    "csv",
    "sqlite3",
    "argparse",
    "subprocess",
    "shutil",
    "tempfile",
    "textwrap",
    "unicodedata",
    "uuid",
    "random",
    "statistics",
    "copy",
    "hashlib",
    "base64",
    "urllib",
    "http",
    "socket",
    "threading",
    "multiprocessing",
    "concurrent",
    "asyncio",
    "contextlib",
    "warnings",
    "unittest",
    "pickle",
    "glob",
    "fnmatch",
    "string",
    "struct",
    "zlib",
    "gzip",
    "tarfile",
    "zipfile",
    "heapq",
    "bisect",
    "array",
    "weakref",
    "types",
    "operator",
    "inspect",
    "traceback",
    "atexit",
    "signal",
    "stat",
    "errno",
    "platform",
    "getpass",
    "difflib",
    "secrets",
    "gc",
    "os.path",
];

#[derive(Debug, Clone, Default)]
pub struct NewSymbol {
    pub kind: &'static str, // module | class | function | method | trait | interface
    pub name: String,
    pub qualname: String,
    pub file_idx: usize,
    /// 1-based lines (tree-sitter rows are 0-based; offset at emission).
    pub line_start: usize,
    pub line_end: usize,
    /// One-line signature (Amendment 12a): first line of the node's text,
    /// trimmed, hard cap 160 chars. Empty for module/synthetic symbols.
    pub signature: String,
    /// Per-symbol metrics (Amendment 19); 0 for non-callables.
    pub cyclomatic: usize,
    pub branches: usize,
    pub nesting: usize,
    pub effective_loc: usize,
    /// Public-surface flag (Amendment 19); language rules in the spec.
    pub exported: bool,
}

#[derive(Debug, Clone)]
pub struct NewEdge {
    /// Index into the merged symbols vector.
    pub src_idx: usize,
    pub dst_idx: Option<usize>,
    pub dst_name: String,
    pub kind: &'static str,       // calls | imports | contains | inherits
    pub confidence: &'static str, // EXTRACTED | AMBIGUOUS
    pub site_line: usize,
}

#[derive(Debug, Default, Clone)]
pub struct ExtractStats {
    pub builtin_dropped: usize,
    pub unresolved_callees: usize,
}

/// One unresolved edge site (Amendment 19). `kind` is call|import|inherit;
/// `site_line` is 1-based in the source file.
#[derive(Debug, Clone)]
pub struct UnresolvedSite {
    pub file_idx: usize,
    pub name: String,
    pub kind: &'static str,
    pub site_line: usize,
}

pub struct Resolution {
    pub symbols: Vec<NewSymbol>,
    /// Parallel to `symbols`: Some(row_id) for symbols carried forward from the
    /// previous index (already stored; not re-inserted), None for fresh rows.
    pub existing_ids: Vec<Option<i64>>,
    pub edges: Vec<NewEdge>,
    /// Kept for the lib surface (integration tests assert builtin_dropped);
    /// the binary's meta total is the per-file SQL sum instead.
    #[allow(dead_code)]
    pub stats: ExtractStats,
    /// Unresolved edge sites as (fresh-file index, written name, kind, line).
    /// Amendment 15/19: lets the fast path re-extract exactly the carried
    /// files whose unresolved names became resolvable after a change.
    pub unresolved_calls: Vec<UnresolvedSite>,
    pub unresolved_per_file: Vec<usize>,
}

/// Symbols loaded from the previous index's fast-path files. They join the
/// resolution tables so fresh->carried calls still resolve; their rows are
/// already in the DB and are never re-inserted.
pub struct CarriedSymbol {
    pub row_id: i64,
    pub kind: &'static str,
    pub name: String,
    pub qualname: String,
    /// Module path of the carrying file (drives module_paths + bare-name table).
    pub module: String,
}

/// Inheritance pairs read from the previous index (subclass qualname, base qualname).
pub type CarriedInherits = Vec<(String, String)>;

/// Previous-index symbols joined into resolution (fast path).
#[derive(Default)]
pub struct CarriedIndex {
    pub symbols: Vec<CarriedSymbol>,
    pub inherits: CarriedInherits,
}

// ---------------------------------------------------------------------------
// Phase A: per-file extraction
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum ImportTarget {
    /// `import a.b.c` binds "a"; `import a.b.c as z` binds the full path to z.
    ModuleRoot { path: String },
    /// A module-level import edge target (Amendment 19): `import a.b`,
    /// `from pkg import mod`, TS `import './x'`, Rust `use`/`mod`.
    Module { path: String },
    /// `from m import n [as y]` binds the symbol (m, n).
    Symbol { module: String, name: String },
}

struct Assignment {
    var: String,
    target_dotted: Vec<String>,
    scope_func: String,
}

/// `self.field = Ctor(...)` inside a class body: receiver typing for
/// `self.field.m()` calls in any method of that class.
struct SelfField {
    class_qn: String,
    field: String,
    target_dotted: Vec<String>,
}

struct ParamType {
    var: String,
    ann_dotted: Vec<String>,
    scope_func: String,
}

struct CallSite {
    line: usize,
    scope_func: String,
    scope_class: Option<String>,
    callee: Callee,
}

enum Callee {
    Name(String),
    Attr(Vec<String>),
}

struct ImportEdgeCandidate {
    line: usize,
    target: ImportTarget,
}

pub struct FileExtraction {
    pub file_idx: usize,
    pub module: String,
    /// Dot-joined DIRECTORY of the file (no file stem) — relative TS imports
    /// resolve against this, because stems may contain dots (x.test.ts).
    pub dir: String,
    pub is_package: bool,
    pub lang: Lang,
    pub symbols: Vec<NewSymbol>,
    /// contains edges (indices local to this file's symbols; offset on merge).
    pub edges: Vec<NewEdge>,
    /// (src local idx, base dotted chain)
    inherits: Vec<(usize, Vec<String>)>,
    /// Rust `impl Trait for Type` edges: edge-only, never `class_bases`.
    impl_inherits: Vec<(usize, Vec<String>, usize)>,
    /// Rust impls collected before symbol lookup:
    /// (target qualname, trait chain, impl line). Same-file only in v1.
    rust_impls: Vec<(String, Vec<String>, usize)>,
    assignments: Vec<Assignment>,
    params: Vec<ParamType>,
    self_fields: Vec<SelfField>,
    aliases: Vec<(String, ImportTarget)>,
    /// Modules appearing in `from m import *`.
    stars: Vec<String>,
    calls: Vec<CallSite>,
    import_edges: Vec<ImportEdgeCandidate>,
}

/// Derive the dot-joined module path from the repo-relative file path.
/// Package markers (`__init__.py`, `index.ts(x)`, `index.js(x)`) collapse
/// into the package path.
pub fn module_path_for(rel: &str) -> (String, bool) {
    let file_name = rel.rsplit('/').next().unwrap_or(rel);
    let is_py_pkg = file_name == "__init__.py";
    let is_ts_pkg = file_name == "index.ts"
        || file_name == "index.tsx"
        || file_name == "index.js"
        || file_name == "index.jsx";
    if is_py_pkg || is_ts_pkg {
        let dir = rel
            .strip_suffix(file_name)
            .unwrap_or(rel)
            .trim_end_matches('/')
            .replace('/', ".");
        if dir.is_empty() {
            return ("root".to_string(), true);
        }
        return (dir, true);
    }
    let stem = rel
        .trim_end_matches(".tsx")
        .trim_end_matches(".ts")
        .trim_end_matches(".jsx")
        .trim_end_matches(".js")
        .trim_end_matches(".rs")
        .trim_end_matches(".py");
    if stem.is_empty() {
        return ("root".to_string(), false);
    }
    (stem.replace('/', "."), false)
}

pub fn extract_file(file_idx: usize, rel: &str, src: &[u8], parsed: &ParsedFile) -> FileExtraction {
    let (module, is_package) = module_path_for(rel);
    let mod_name = module.rsplit('.').next().unwrap_or(&module).to_string();
    let dir = match rel.rfind('/') {
        Some(i) => rel[..i].replace('/', "."),
        None => String::new(),
    };
    let mut fx = FileExtraction {
        file_idx,
        module: module.clone(),
        dir,
        is_package,
        lang: Lang::from_rel(rel),
        symbols: vec![NewSymbol {
            kind: "module",
            name: mod_name,
            qualname: module,
            file_idx,
            line_start: 1,
            line_end: 1,
            signature: String::new(),

            ..Default::default()
        }],
        edges: Vec::new(),
        inherits: Vec::new(),
        impl_inherits: Vec::new(),
        rust_impls: Vec::new(),
        assignments: Vec::new(),
        params: Vec::new(),
        self_fields: Vec::new(),
        aliases: Vec::new(),
        stars: Vec::new(),
        calls: Vec::new(),
        import_edges: Vec::new(),
    };
    let mut ctx = WalkCtx::default();
    let root = parsed.tree.root_node();
    walk_children(root, src, &mut fx, &mut ctx);
    finalize_exports(&mut fx, src, root);
    finalize_rust_impls(&mut fx);
    fx
}

fn python_all_names(root: Node<'_>, src: &[u8]) -> Option<std::collections::HashSet<String>> {
    let mut cursor = root.walk();
    for stmt in root.named_children(&mut cursor) {
        if stmt.kind() != "expression_statement" {
            continue;
        }
        let mut c = stmt.walk();
        let Some(assign) = stmt
            .named_children(&mut c)
            .find(|n| n.kind() == "assignment")
        else {
            continue;
        };
        let Some(left) = assign.child_by_field_name("left") else {
            continue;
        };
        if text(left, src) != "__all__" {
            continue;
        }
        let right = assign.child_by_field_name("right")?;
        if !matches!(right.kind(), "list" | "tuple") {
            return None;
        }
        let mut out = std::collections::HashSet::new();
        let mut rc = right.walk();
        for item in right.named_children(&mut rc) {
            if item.kind() == "string" {
                let raw = text(item, src);
                let name = raw.trim_matches(|c| c == '"' || c == '\'').to_string();
                if !name.is_empty() {
                    out.insert(name);
                }
            } else {
                return None;
            }
        }
        return Some(out);
    }
    None
}

/// Populate the exported/public flag from language rules (Amendment 19).
fn finalize_exports(fx: &mut FileExtraction, src: &[u8], root: Node<'_>) {
    let lines: Vec<&[u8]> = src.split(|b| *b == b'\n').collect();
    let all_names = if fx.lang == Lang::Python {
        python_all_names(root, src)
    } else {
        None
    };
    for i in 0..fx.symbols.len() {
        let kind = fx.symbols[i].kind;
        let name = fx.symbols[i].name.clone();
        let qualname = fx.symbols[i].qualname.clone();
        let signature = fx.symbols[i].signature.clone();
        let line = fx.symbols[i].line_start.saturating_sub(1);
        let parent_qn = qualname.rsplit_once('.').map(|(p, _)| p.to_string());
        let parent = parent_qn
            .as_ref()
            .and_then(|p| fx.symbols.iter().find(|s| &s.qualname == p));
        let parent_exported = parent.map(|s| s.exported).unwrap_or(false);
        let parent_is_module = parent.map(|s| s.kind == "module").unwrap_or(false);
        let exported = match fx.lang {
            Lang::Python => {
                if kind == "module" {
                    false
                } else if let Some(all) = &all_names {
                    all.contains(&name)
                } else if kind == "method" {
                    parent_exported && !name.starts_with('_')
                } else {
                    qualname == format!("{}.{}", fx.module, name) && !name.starts_with('_')
                }
            }
            Lang::Rust => {
                let sig = signature.trim_start();
                let is_pub = sig.starts_with("pub ");
                is_pub && (kind != "method" || parent_exported || parent_is_module)
            }
            Lang::TypeScript | Lang::Tsx | Lang::JavaScript => {
                let source_line = lines
                    .get(line)
                    .map(|l| String::from_utf8_lossy(l).to_string())
                    .unwrap_or_default();
                if matches!(kind, "class" | "function") {
                    fx.symbols[i].exported
                } else if kind == "method" && parent_exported {
                    !name.starts_with('#') && !source_line.contains("private ")
                } else {
                    false
                }
            }
        };
        fx.symbols[i].exported = exported;
    }
}

/// Resolve Rust `impl Trait for Type` pairs against same-file type symbols.
/// Cross-file impls are out of scope in v1 (Amendment 19).
fn finalize_rust_impls(fx: &mut FileExtraction) {
    let impls = std::mem::take(&mut fx.rust_impls);
    for (target_qn, trait_chain, line) in impls {
        if let Some(idx) = fx
            .symbols
            .iter()
            .position(|s| s.kind == "class" && s.qualname == target_qn)
        {
            fx.impl_inherits.push((idx, trait_chain, line));
        }
    }
}

#[derive(Default)]
struct WalkCtx {
    class_stack: Vec<String>, // qualnames
    func_stack: Vec<String>,  // qualnames
    depth: usize,             // AST recursion depth (R6d Blocker 1 guard)
}

impl WalkCtx {
    fn scope_func(&self) -> String {
        self.func_stack.last().cloned().unwrap_or_default()
    }
}

fn text(node: Node<'_>, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").to_string()
}

/// One-line signature (Amendment 12a): first line of the node's text,
/// trimmed, hard cap 160 chars. Never spans the body: capture stops at the
/// first newline, so the stored text is a header fragment by construction.
fn signature_of(node: Node<'_>, src: &[u8]) -> String {
    let start = node.start_byte();
    let stop = src.len().min(start + 4096);
    let python = matches!(node.kind(), "function_definition" | "class_definition");
    let mut depth = 0i32;
    let mut end = start;
    let mut prev_non_space: u8 = 0;
    while end < stop {
        let b = src[end];
        match b {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'{' => {
                if depth == 0 {
                    break;
                }
                depth += 1;
            }
            b'}' => depth -= 1,
            b';' if depth == 0 && !python => break,
            b':' if depth == 0 && python => break,
            b'\n' if depth == 0 => {
                // Continue only when the line wraps inside an expression.
                if !matches!(
                    prev_non_space,
                    b',' | b'(' | b'[' | b'{' | b'&' | b'|' | b'=' | b'>'
                ) {
                    break;
                }
            }
            _ => {}
        }
        if !b.is_ascii_whitespace() {
            prev_non_space = b;
        }
        end += 1;
    }
    let raw = String::from_utf8_lossy(&src[start..end]);
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(160).collect()
}

/// Lowercased token split of an identifier path for symbol search
/// (Amendment 12b): camelCase humps, snake_case underscores, dots and other
/// separators all become single spaces. `HierarchicalAddressParser.parse`
/// -> "hierarchical address parser parse". Known limitation (pinned by test):
/// consecutive-capital runs are not split — `HTTPServer` stays one token —
/// so acronym-led identifiers match by full prefix only.
pub fn split_ident(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 8);
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '_' || !ch.is_alphanumeric() {
            out.push(' ');
            continue;
        }
        if ch.is_uppercase() && i > 0 {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|c| c.is_lowercase());
            // Split lowerCase|UPPER and the last capital of an acronym
            // before a lowercase run ("HTTPServer" -> http server) (R7).
            if prev.is_lowercase() || prev.is_numeric() || (prev.is_uppercase() && next_lower) {
                out.push(' ');
            }
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

/// Dotted identifier chain for identifier/attribute nodes: `a.b.c` -> [a, b, c].
/// Unwraps `type` wrapper nodes (annotations). Rust path keywords (`crate`,
/// `self`, `super`) join the chain as plain segments.
fn dotted_chain(node: Node<'_>, src: &[u8]) -> Option<Vec<String>> {
    match node.kind() {
        "identifier" | "this" | "super" | "crate" | "type_identifier" => {
            Some(vec![text(node, src)])
        }
        "attribute" => {
            let obj = node.child_by_field_name("object")?;
            let attr = node.child_by_field_name("attribute")?;
            let mut chain = dotted_chain(obj, src)?;
            chain.push(text(attr, src));
            Some(chain)
        }
        // Rust scoped_identifier: a::b::c (fields path + name).
        "scoped_identifier" => {
            let mut chain = Vec::new();
            if let Some(path) = node.child_by_field_name("path") {
                if let Some(mut sub) = dotted_chain(path, src) {
                    chain.append(&mut sub);
                }
            }
            if let Some(name) = node.child_by_field_name("name") {
                chain.push(text(name, src));
            }
            if chain.is_empty() {
                None
            } else {
                Some(chain)
            }
        }
        // TypeScript member_expression: a.b.c (field `property`, not `attribute`).
        "member_expression" => {
            let obj = node.child_by_field_name("object")?;
            let attr = node.child_by_field_name("property")?;
            let mut chain = dotted_chain(obj, src)?;
            chain.push(text(attr, src));
            Some(chain)
        }
        "type" => {
            let mut cursor = node.walk();
            let inner = node.named_children(&mut cursor).next();
            inner.and_then(|ch| dotted_chain(ch, src))
        }
        "generic_type" => {
            // `Trait<T>`: use the first type identifier as the base name.
            let mut stack = vec![node];
            while let Some(n) = stack.pop() {
                if n.kind() == "type_identifier" {
                    return Some(vec![text(n, src)]);
                }
                let mut c = n.walk();
                let children: Vec<_> = n.children(&mut c).collect();
                for child in children.into_iter().rev() {
                    stack.push(child);
                }
            }
            None
        }
        // Constructor-call receiver: `Base().m()` / `super().m()` /
        // `super(C, self).m()` (Amendment 18). Non-uppercase factory calls
        // are left unresolved to avoid manufacturing class edges.
        "call" => {
            let func = node.child_by_field_name("function")?;
            let chain = dotted_chain(func, src)?;
            let first = chain.first()?;
            if first == "super" || first.chars().next().is_some_and(|c| c.is_uppercase()) {
                Some(chain)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Hard nesting ceiling. Source nesting is attacker-controlled; beyond this
/// depth the subtree is skipped instead of risking a stack abort (R6d
/// Blocker 1). Workers run on 64 MB stacks, so legitimate deep ASTs below
/// this bound are unaffected.
const MAX_AST_DEPTH: usize = 8_192;

fn walk_children(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, ctx: &mut WalkCtx) {
    if ctx.depth >= MAX_AST_DEPTH {
        return;
    }
    ctx.depth += 1;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_node(child, src, fx, ctx);
    }
    ctx.depth -= 1;
}

fn walk_node(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, ctx: &mut WalkCtx) {
    match node.kind() {
        // ---- Rust definitions (P2.5) ----
        "function_item" => {
            define_function(
                node,
                src,
                fx,
                ctx,
                if ctx.class_stack.is_empty() {
                    "function"
                } else {
                    "method"
                },
            );
        }
        "struct_item" | "enum_item" => {
            define_class_ts(node, src, fx, ctx, "class");
        }
        "trait_item" => {
            define_class_ts(node, src, fx, ctx, "trait");
        }
        "function_signature_item" => {
            define_function(
                node,
                src,
                fx,
                ctx,
                if ctx.class_stack.is_empty() {
                    "function"
                } else {
                    "method"
                },
            );
        }
        "mod_item" if node.child_by_field_name("body").is_some() => {
            // Inline `mod name { ... }` is a scope, not just an alias (R7).
            let name = node
                .child_by_field_name("name")
                .map(|n| text(n, src))
                .unwrap_or_default();
            let qualname = if ctx.class_stack.is_empty() {
                format!("{}.{}", fx.module, name)
            } else {
                format!("{}.{}", ctx.class_stack.last().unwrap(), name)
            };
            let sym_idx = fx.symbols.len();
            fx.symbols.push(NewSymbol {
                kind: "module",
                name,
                qualname: qualname.clone(),
                file_idx: fx.file_idx,
                signature: String::new(),
                line_start: node.start_position().row + 1,
                line_end: node.end_position().row + 1,

                ..Default::default()
            });
            if let Some(pidx) = parent_symbol_idx(fx, ctx) {
                fx.edges.push(NewEdge {
                    src_idx: pidx,
                    dst_idx: Some(sym_idx),
                    dst_name: qualname.clone(),
                    kind: "contains",
                    confidence: "EXTRACTED",
                    site_line: node.start_position().row + 1,
                });
            }
            ctx.class_stack.push(qualname);
            if let Some(body) = node.child_by_field_name("body") {
                walk_children(body, src, fx, ctx);
            }
            ctx.class_stack.pop();
        }
        "internal_module" | "module" => {
            // TypeScript `namespace N { ... }` (R7).
            let name = node
                .child_by_field_name("name")
                .map(|n| text(n, src))
                .unwrap_or_default();
            let qualname = if ctx.class_stack.is_empty() {
                format!("{}.{}", fx.module, name)
            } else {
                format!("{}.{}", ctx.class_stack.last().unwrap(), name)
            };
            let sym_idx = fx.symbols.len();
            fx.symbols.push(NewSymbol {
                kind: "module",
                name,
                qualname: qualname.clone(),
                file_idx: fx.file_idx,
                signature: String::new(),
                line_start: node.start_position().row + 1,
                line_end: node.end_position().row + 1,

                ..Default::default()
            });
            if let Some(pidx) = parent_symbol_idx(fx, ctx) {
                fx.edges.push(NewEdge {
                    src_idx: pidx,
                    dst_idx: Some(sym_idx),
                    dst_name: qualname.clone(),
                    kind: "contains",
                    confidence: "EXTRACTED",
                    site_line: node.start_position().row + 1,
                });
            }
            ctx.class_stack.push(qualname);
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.child_by_field_name("body").is_some() || child.kind() == "statement_block"
                {
                    walk_children(child, src, fx, ctx);
                }
            }
            ctx.class_stack.pop();
        }
        "impl_item" => {
            // impl Type { ... } — scope methods under the type name.
            let type_name = impl_type_name(node, src);
            let qualname = if ctx.class_stack.is_empty() {
                format!("{}.{}", fx.module, type_name)
            } else {
                format!("{}.{}", ctx.class_stack.last().unwrap(), type_name)
            };
            if let Some(trait_node) = node.child_by_field_name("trait") {
                if let Some(chain) = dotted_chain(trait_node, src) {
                    fx.rust_impls
                        .push((qualname.clone(), chain, node.start_position().row + 1));
                }
            }
            ctx.class_stack.push(qualname);
            walk_children(node, src, fx, ctx);
            ctx.class_stack.pop();
        }
        // ---- TypeScript/TSX/JSX definitions (P2) ----
        "function_declaration" | "generator_function_declaration" => {
            define_function(node, src, fx, ctx, "function");
        }
        "class_declaration" | "abstract_class_declaration" => {
            define_class_ts(node, src, fx, ctx, "class");
        }
        "interface_declaration" => {
            define_class_ts(node, src, fx, ctx, "interface");
        }
        "method_definition" => {
            define_method_ts(node, src, fx, ctx);
        }
        "variable_declarator" => {
            record_ts_arrow_function(node, src, fx, ctx);
            walk_children(node, src, fx, ctx);
        }
        "new_expression" => {
            record_call(node, src, fx, ctx);
            walk_children(node, src, fx, ctx);
        }
        "import_statement" if fx.lang != Lang::Python => {
            record_import_ts(node, src, fx);
        }
        "export_statement" if fx.lang != Lang::Python => {
            record_export_ts(node, src, fx);
            walk_children(node, src, fx, ctx);
        }
        "mod_item" if fx.lang == Lang::Rust => {
            // `mod db;` declares a module sibling: bind db -> crate_root.db.
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, src);
                if node.child_by_field_name("body").is_none() {
                    if let Some(root) = fx.dir.split('.').next().map(|p| p.to_string()) {
                        let path = format!("{root}.{name}");
                        fx.aliases
                            .push((name, ImportTarget::ModuleRoot { path: path.clone() }));
                        fx.import_edges.push(ImportEdgeCandidate {
                            line: node.start_position().row + 1,
                            target: ImportTarget::Module { path },
                        });
                    }
                }
            }
            walk_children(node, src, fx, ctx);
        }
        "use_declaration" if fx.lang == Lang::Rust => {
            record_use_decl(node, src, fx);
            walk_children(node, src, fx, ctx);
        }
        "decorated_definition" => {
            let mut cursor = node.walk();
            let mut decorators = Vec::new();
            for child in node.children(&mut cursor) {
                if child.kind() == "decorator" {
                    decorators.push(child);
                }
            }
            // Decorator calls execute at definition scope (before the function
            // is pushed), so collect them with the current context.
            for d in decorators {
                let mut stack = vec![d];
                while let Some(n) = stack.pop() {
                    if n.kind() == "call" {
                        record_call(n, src, fx, ctx);
                    }
                    let mut c = n.walk();
                    for child in n.children(&mut c) {
                        stack.push(child);
                    }
                }
            }
            if let Some(def) = node.child_by_field_name("definition") {
                walk_node(def, src, fx, ctx);
            }
        }
        "function_definition" => {
            let Some(name_node) = node.child_by_field_name("name") else {
                return;
            };
            let name = text(name_node, src);
            let kind = if ctx.class_stack.is_empty() {
                "function"
            } else {
                "method"
            };
            // Innermost scope wins: a nested def belongs to its enclosing
            // function; a method (no func scope) belongs to its class (R7).
            let parent_qn = ctx
                .func_stack
                .last()
                .or_else(|| ctx.class_stack.last())
                .cloned()
                .unwrap_or_else(|| fx.module.clone());
            let qualname = format!("{parent_qn}.{name}");
            let start = node.start_position().row + 1;
            let sym_idx = fx.symbols.len();
            let (cyc, br, nest, loc) = callable_metrics(node, src, fx.lang);
            fx.symbols.push(NewSymbol {
                kind,
                name: name.clone(),
                qualname: qualname.clone(),
                file_idx: fx.file_idx,
                signature: signature_of(node, src),
                line_start: start,
                line_end: node.end_position().row + 1,
                cyclomatic: cyc,
                branches: br,
                nesting: nest,
                effective_loc: loc,
                ..Default::default()
            });
            if let Some(params) = node.child_by_field_name("parameters") {
                collect_param_types(params, src, fx, &qualname);
            }
            if let Some(pidx) = parent_symbol_idx(fx, ctx) {
                fx.edges.push(NewEdge {
                    src_idx: pidx,
                    dst_idx: Some(sym_idx),
                    dst_name: qualname.clone(),
                    kind: "contains",
                    confidence: "EXTRACTED",
                    site_line: start,
                });
            }
            ctx.func_stack.push(qualname);
            if let Some(body) = node.child_by_field_name("body") {
                walk_children(body, src, fx, ctx);
            }
            ctx.func_stack.pop();
        }
        "class_definition" => {
            let Some(name_node) = node.child_by_field_name("name") else {
                return;
            };
            let name = text(name_node, src);
            // Innermost scope wins: a nested def belongs to its enclosing
            // function; a method (no func scope) belongs to its class (R7).
            let parent_qn = ctx
                .func_stack
                .last()
                .or_else(|| ctx.class_stack.last())
                .cloned()
                .unwrap_or_else(|| fx.module.clone());
            let qualname = format!("{parent_qn}.{name}");
            let start = node.start_position().row + 1;
            let sym_idx = fx.symbols.len();
            fx.symbols.push(NewSymbol {
                kind: "class",
                name: name.clone(),
                qualname: qualname.clone(),
                file_idx: fx.file_idx,
                signature: signature_of(node, src),
                line_start: start,
                line_end: node.end_position().row + 1,

                ..Default::default()
            });
            if let Some(bases) = node.child_by_field_name("superclasses") {
                let mut cursor = bases.walk();
                for b in bases.named_children(&mut cursor) {
                    if let Some(chain) = dotted_chain(b, src) {
                        fx.inherits.push((sym_idx, chain));
                    }
                }
            }
            if let Some(pidx) = parent_symbol_idx(fx, ctx) {
                fx.edges.push(NewEdge {
                    src_idx: pidx,
                    dst_idx: Some(sym_idx),
                    dst_name: qualname.clone(),
                    kind: "contains",
                    confidence: "EXTRACTED",
                    site_line: start,
                });
            }
            ctx.class_stack.push(qualname);
            if let Some(body) = node.child_by_field_name("body") {
                walk_children(body, src, fx, ctx);
            }
            ctx.class_stack.pop();
        }
        "assignment" => {
            record_assignment(node, src, fx, ctx);
            walk_children(node, src, fx, ctx);
        }
        "call" | "call_expression" => {
            record_call(node, src, fx, ctx);
            walk_children(node, src, fx, ctx);
        }
        "import_statement" | "import_from_statement" => {
            record_import(node, src, fx);
        }
        _ => walk_children(node, src, fx, ctx),
    }
}

fn parent_symbol_idx(fx: &FileExtraction, ctx: &WalkCtx) -> Option<usize> {
    let qn = if let Some(f) = ctx.func_stack.last() {
        f.clone()
    } else if let Some(c) = ctx.class_stack.last() {
        c.clone()
    } else {
        return Some(0); // module symbol is always symbols[0]
    };
    fx.symbols.iter().position(|s| s.qualname == qn)
}

fn collect_param_types(params: Node<'_>, src: &[u8], fx: &mut FileExtraction, scope: &str) {
    let mut cursor = params.walk();
    for p in params.children(&mut cursor) {
        // TS: required_parameter / optional_parameter carry pattern + type.
        if matches!(p.kind(), "required_parameter" | "optional_parameter") {
            let Some(ty) = p.child_by_field_name("type") else {
                continue;
            };
            let Some(ident) = p.child_by_field_name("pattern") else {
                continue;
            };
            if let Some(chain) = dotted_chain(ty, src) {
                fx.params.push(ParamType {
                    var: text(ident, src),
                    ann_dotted: chain,
                    scope_func: scope.to_string(),
                });
            }
            continue;
        }
        if matches!(p.kind(), "typed_parameter" | "typed_default_parameter") {
            // typed_parameter has NO 'name' field (positional identifier);
            // typed_default_parameter does. The annotation lives in the
            // 'type' field, wrapped in a 'type' node.
            let ident = p.child_by_field_name("name").or_else(|| {
                let mut c = p.walk();
                let found = p.children(&mut c).find(|ch| ch.kind() == "identifier");
                found
            });
            let ty = p.child_by_field_name("type");
            let (Some(ident), Some(ty)) = (ident, ty) else {
                continue;
            };
            if let Some(chain) = dotted_chain(ty, src) {
                fx.params.push(ParamType {
                    var: text(ident, src),
                    ann_dotted: chain,
                    scope_func: scope.to_string(),
                });
            }
        }
    }
}

/// Constructor-target scan for receiver typing. CONSERVATIVE (R1 fix): only a
/// DIRECT call RHS types the receiver — `var = Ctor(...)`. For `a or B(...)`
/// the last top-level OR operand is accepted (the fallback-construction idiom);
/// nested calls like `obj = factory(A())` type NOTHING (the outer call's
/// return type is unknowable). Handles `var = ...` (function-local) and
/// `self.field = ...` (class-wide).
fn record_assignment(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, ctx: &mut WalkCtx) {
    let Some(left) = node.child_by_field_name("left") else {
        return;
    };
    let Some(right) = node.child_by_field_name("right") else {
        return;
    };
    let direct_call_chain = |call: Node<'_>| -> Option<Vec<String>> {
        let f = call
            .child_by_field_name("function")
            .or_else(|| call.child_by_field_name("constructor"))?;
        dotted_chain(f, src)
    };
    let target: Option<Vec<String>> =
        if matches!(right.kind(), "call" | "call_expression" | "new_expression") {
            direct_call_chain(right)
        } else if matches!(right.kind(), "boolean_operator" | "bool_op") {
            // `x = a or B(...)`: the last top-level operand that is a direct call.
            let mut cursor = right.walk();
            let operands: Vec<Node<'_>> = right.named_children(&mut cursor).collect();
            operands
                .iter()
                .rev()
                .find(|o| matches!(o.kind(), "call" | "call_expression"))
                .and_then(|o| direct_call_chain(*o))
        } else {
            None
        };
    let Some(target_dotted) = target else { return };
    match left.kind() {
        "identifier" => fx.assignments.push(Assignment {
            var: text(left, src),
            target_dotted,
            scope_func: ctx.scope_func(),
        }),
        "attribute" => {
            // self.field = Ctor(...) -> class-wide receiver typing.
            let obj = left.child_by_field_name("object");
            let attr = left.child_by_field_name("attribute");
            if let (Some(obj), Some(attr)) = (obj, attr) {
                if obj.kind() == "identifier" && text(obj, src) == "self" {
                    if let Some(class_qn) = ctx.class_stack.last().cloned() {
                        fx.self_fields.push(SelfField {
                            class_qn,
                            field: text(attr, src),
                            target_dotted,
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

fn record_call(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, ctx: &mut WalkCtx) {
    // call_expression uses field "function"; TS new_expression uses "constructor".
    let f = node
        .child_by_field_name("function")
        .or_else(|| node.child_by_field_name("constructor"));
    let Some(f) = f else { return };
    if matches!(fx.lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript) {
        let callee = text(f, src);
        if callee == "require" || callee == "import" {
            if let Some(args) = node.child_by_field_name("arguments") {
                let mut c = args.walk();
                if let Some(arg) = args.named_children(&mut c).next() {
                    if arg.kind() == "string" {
                        let raw = text(arg, src);
                        let source = raw.trim_matches(|ch| ch == '\'' || ch == '"');
                        let module = resolve_ts_module(source, &fx.dir);
                        fx.import_edges.push(ImportEdgeCandidate {
                            line: node.start_position().row + 1,
                            target: ImportTarget::Module { path: module },
                        });
                        return;
                    }
                };
            }
        }
    }
    let _ = fx.lang;
    let callee = match dotted_chain(f, src) {
        Some(chain) if chain.len() == 1 => Callee::Name(chain.into_iter().next().unwrap()),
        Some(chain) => Callee::Attr(chain),
        None => return, // foo()[i](...): unresolvable, not recorded
    };
    fx.calls.push(CallSite {
        line: node.start_position().row + 1,
        scope_func: ctx.scope_func(),
        scope_class: ctx.class_stack.last().cloned(),
        callee,
    });
}

fn record_import(node: Node<'_>, src: &[u8], fx: &mut FileExtraction) {
    let line = node.start_position().row + 1;
    match node.kind() {
        "import_statement" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "dotted_name" => {
                        let path = text(child, src);
                        let root = path.split('.').next().unwrap_or(&path).to_string();
                        fx.aliases
                            .push((root.clone(), ImportTarget::ModuleRoot { path: root }));
                        let parts: Vec<&str> = path.split('.').collect();
                        for i in 1..=parts.len() {
                            fx.import_edges.push(ImportEdgeCandidate {
                                line,
                                target: ImportTarget::Module {
                                    path: parts[..i].join("."),
                                },
                            });
                        }
                    }
                    // `import a.b as c` wraps in aliased_import; a bare
                    // "alias" node appears in some grammar versions.
                    "aliased_import" | "alias" => {
                        let Some(target) = child.child_by_field_name("name") else {
                            continue;
                        };
                        let Some(alias) = child.child_by_field_name("alias") else {
                            continue;
                        };
                        let target_path = text(target, src);
                        fx.aliases.push((
                            text(alias, src),
                            ImportTarget::ModuleRoot {
                                path: target_path.clone(),
                            },
                        ));
                        let parts: Vec<&str> = target_path.split('.').collect();
                        for i in 1..=parts.len() {
                            fx.import_edges.push(ImportEdgeCandidate {
                                line,
                                target: ImportTarget::Module {
                                    path: parts[..i].join("."),
                                },
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        "import_from_statement" => {
            let module_node = node.child_by_field_name("module_name");
            let module_abs =
                module_node.map(|m| resolve_from_module(m, src, &fx.module, fx.is_package));
            let Some(m) = module_abs else { return };
            fx.import_edges.push(ImportEdgeCandidate {
                line,
                target: ImportTarget::Module { path: m.clone() },
            });
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if Some(child) == module_node {
                    continue;
                }
                match child.kind() {
                    "wildcard_import" => {
                        fx.stars.push(m.clone());
                        fx.import_edges.push(ImportEdgeCandidate {
                            line,
                            target: ImportTarget::ModuleRoot { path: m.clone() },
                        });
                    }
                    "dotted_name" => {
                        let name = text(child, src);
                        fx.aliases.push((
                            name.clone(),
                            ImportTarget::Symbol {
                                module: m.clone(),
                                name: name.clone(),
                            },
                        ));
                        fx.import_edges.push(ImportEdgeCandidate {
                            line,
                            target: ImportTarget::Module {
                                path: format!("{m}.{name}"),
                            },
                        });
                        fx.import_edges.push(ImportEdgeCandidate {
                            line,
                            target: ImportTarget::Symbol {
                                module: m.clone(),
                                name,
                            },
                        });
                    }
                    // `from m import n as y` wraps the binding in an
                    // aliased_import node (import_statement uses "alias").
                    "aliased_import" | "alias" => {
                        let Some(target) = child.child_by_field_name("name") else {
                            continue;
                        };
                        let Some(alias) = child.child_by_field_name("alias") else {
                            continue;
                        };
                        let name = text(target, src);
                        fx.aliases.push((
                            text(alias, src),
                            ImportTarget::Symbol {
                                module: m.clone(),
                                name: name.clone(),
                            },
                        ));
                        fx.import_edges.push(ImportEdgeCandidate {
                            line,
                            target: ImportTarget::Module {
                                path: format!("{m}.{name}"),
                            },
                        });
                        fx.import_edges.push(ImportEdgeCandidate {
                            line,
                            target: ImportTarget::Symbol {
                                module: m.clone(),
                                name,
                            },
                        });
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// `from <m> import ...`: resolve m to an absolute module path.
/// Relative levels: for a plain module file, level 1 = its package; for a
/// package file, level 1 = the package itself.
fn resolve_from_module(
    module_node: Node<'_>,
    src: &[u8],
    file_module: &str,
    is_package: bool,
) -> String {
    let raw = text(module_node, src);
    if !raw.starts_with('.') {
        return raw;
    }
    let level = raw.chars().take_while(|c| *c == '.').count();
    let rest = raw.trim_start_matches('.');
    let mut parts: Vec<&str> = file_module.split('.').collect();
    if !is_package {
        parts.pop(); // plain module -> its containing package
    }
    for _ in 1..level {
        parts.pop();
    }
    // Root-level plain module: the containing package is the empty string, and
    // joining it must not leave a stray leading dot (".widget" never matches
    // the indexed module "widget" and misclassifies project calls as dropped).
    if rest.is_empty() {
        parts.join(".")
    } else if parts.is_empty() {
        rest.to_string()
    } else {
        format!("{}.{}", parts.join("."), rest)
    }
}

// ---------------------------------------------------------------------------
// TypeScript/TSX/JSX definitions (P2)
// ---------------------------------------------------------------------------

fn branch_kinds(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::Python => &[
            "if_statement",
            "for_statement",
            "while_statement",
            "elif_clause",
            "except_clause",
            "case_clause",
        ],
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => &[
            "if_statement",
            "for_statement",
            "for_in_statement",
            "while_statement",
            "do_statement",
            "switch_case",
        ],
        Lang::Rust => &[
            "if_expression",
            "for_expression",
            "while_expression",
            "loop_expression",
            "match_arm",
        ],
    }
}

fn is_nested_callable(kind: &str) -> bool {
    matches!(
        kind,
        "function_definition"
            | "function_declaration"
            | "method_definition"
            | "generator_function_declaration"
            | "function_expression"
            | "arrow_function"
            | "lambda"
            | "closure_expression"
            | "function_item"
    )
}

/// Per-symbol metrics (Amendment 19). Walks the callable subtree, skipping
/// nested callable bodies; branch depth starts at 1 for the body.
fn callable_metrics(node: Node<'_>, src: &[u8], lang: Lang) -> (usize, usize, usize, usize) {
    let decisions: Vec<&str> = crate::parse::decision_kinds(lang)
        .iter()
        .copied()
        .filter(|k| {
            !matches!(
                *k,
                "match_statement" | "if_let_expression" | "while_let_expression"
            )
        })
        .collect();
    let branches = branch_kinds(lang);
    let mut cyclomatic = 1usize;
    let mut branch_count = 0usize;
    let mut max_nest = 0usize;
    let mut stack: Vec<(Node<'_>, usize)> = node
        .children(&mut node.walk())
        .map(|c| (c, 0usize))
        .collect();
    while let Some((n, depth)) = stack.pop() {
        if n.id() != node.id() && is_nested_callable(n.kind()) {
            continue;
        }
        if decisions.iter().any(|k| *k == n.kind()) {
            cyclomatic += 1;
        }
        let is_branch = branches.contains(&n.kind());
        if is_branch {
            branch_count += 1;
            max_nest = max_nest.max(depth + 1);
        }
        let child_depth = if is_branch { depth + 1 } else { depth };
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            stack.push((child, child_depth));
        }
    }
    let mut effective_loc = 0usize;
    let mut in_block = false;
    let base_row = node.start_position().row;
    let doc_rows = if lang == Lang::Python {
        if let Some(body) = node.child_by_field_name("body") {
            let mut c = body.walk();
            let first = body.named_children(&mut c).next();
            if let Some(stmt) = first.filter(|s| s.kind() == "expression_statement") {
                let mut c2 = stmt.walk();
                let inner = stmt.named_children(&mut c2).next();
                inner
                    .filter(|s| s.kind() == "string")
                    .map(|s| (s.start_position().row, s.end_position().row))
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    let slice = &src[node.start_byte()..node.end_byte()];
    for (offset, line) in slice.split(|b| *b == b'\n').enumerate() {
        let row = base_row + offset;
        if doc_rows.is_some_and(|(s, e)| row >= s && row <= e) {
            continue;
        }
        let trimmed: Vec<u8> = line
            .iter()
            .copied()
            .skip_while(|b| b.is_ascii_whitespace())
            .collect();
        if trimmed.is_empty() {
            continue;
        }
        let has = |needle: &[u8]| trimmed.windows(needle.len()).any(|w| w == needle);
        if in_block {
            if has(b"*/") {
                in_block = false;
            }
            continue;
        }
        if trimmed.starts_with(b"/*") {
            if !has(b"*/") {
                in_block = true;
            }
            continue;
        }
        if (lang == Lang::Python && trimmed.starts_with(b"#")) || trimmed.starts_with(b"//") {
            continue;
        }
        effective_loc += 1;
    }
    (cyclomatic, branch_count, max_nest, effective_loc)
}

fn define_function(
    node: Node<'_>,
    src: &[u8],
    fx: &mut FileExtraction,
    ctx: &mut WalkCtx,
    kind: &'static str,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = text(name_node, src);
    let parent_qn = ctx
        .class_stack
        .last()
        .or_else(|| ctx.func_stack.last())
        .cloned()
        .unwrap_or_else(|| fx.module.clone());
    let qualname = format!("{parent_qn}.{name}");
    let start = node.start_position().row + 1;
    let sym_idx = fx.symbols.len();
    let (cyc, br, nest, loc) = if node.child_by_field_name("body").is_some() {
        callable_metrics(node, src, fx.lang)
    } else {
        (0, 0, 0, 0)
    };
    fx.symbols.push(NewSymbol {
        kind,
        name: name.clone(),
        qualname: qualname.clone(),
        file_idx: fx.file_idx,
        signature: signature_of(node, src),
        line_start: start,
        line_end: node.end_position().row + 1,
        cyclomatic: cyc,
        branches: br,
        nesting: nest,
        effective_loc: loc,
        exported: matches!(fx.lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript)
            && node
                .parent()
                .is_some_and(|p| p.kind() == "export_statement"),
    });
    if let Some(params) = node.child_by_field_name("parameters") {
        collect_param_types(params, src, fx, &qualname);
    }
    if let Some(pidx) = parent_symbol_idx(fx, ctx) {
        fx.edges.push(NewEdge {
            src_idx: pidx,
            dst_idx: Some(sym_idx),
            dst_name: qualname.clone(),
            kind: "contains",
            confidence: "EXTRACTED",
            site_line: start,
        });
    }
    ctx.func_stack.push(qualname);
    if let Some(body) = node.child_by_field_name("body") {
        walk_children(body, src, fx, ctx);
    }
    ctx.func_stack.pop();
}

fn define_class_ts(
    node: Node<'_>,
    src: &[u8],
    fx: &mut FileExtraction,
    ctx: &mut WalkCtx,
    kind: &'static str,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = text(name_node, src);
    let parent_qn = ctx
        .class_stack
        .last()
        .or_else(|| ctx.func_stack.last())
        .cloned()
        .unwrap_or_else(|| fx.module.clone());
    let qualname = format!("{parent_qn}.{name}");
    let start = node.start_position().row + 1;
    let sym_idx = fx.symbols.len();
    fx.symbols.push(NewSymbol {
        kind,
        name: name.clone(),
        qualname: qualname.clone(),
        file_idx: fx.file_idx,
        signature: signature_of(node, src),
        line_start: start,
        line_end: node.end_position().row + 1,
        exported: matches!(fx.lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript)
            && node
                .parent()
                .is_some_and(|p| p.kind() == "export_statement"),
        ..Default::default()
    });
    // Heritage: extends X / implements Y / trait bounds.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_heritage" | "implements_clause" | "extends_type_clause" => {
                collect_heritage(child, src, fx, sym_idx);
            }
            _ => {}
        }
    }
    if let Some(bounds) = node.child_by_field_name("bounds") {
        collect_heritage(bounds, src, fx, sym_idx);
    }
    if let Some(pidx) = parent_symbol_idx(fx, ctx) {
        fx.edges.push(NewEdge {
            src_idx: pidx,
            dst_idx: Some(sym_idx),
            dst_name: qualname.clone(),
            kind: "contains",
            confidence: "EXTRACTED",
            site_line: start,
        });
    }
    ctx.class_stack.push(qualname);
    if let Some(body) = node.child_by_field_name("body") {
        walk_children(body, src, fx, ctx);
    }
    ctx.class_stack.pop();
}

fn collect_heritage(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, class_idx: usize) {
    // Only outer type nodes are bases. Descending into every identifier turns
    // `Base<Arg>` into a base `Arg` and `std::error::Error` into `std`/`error`.
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        match n.kind() {
            "type" => {
                let mut c = n.walk();
                if let Some(inner) = n.named_children(&mut c).next() {
                    stack.push(inner);
                };
            }
            "class_heritage"
            | "extends_clause"
            | "implements_clause"
            | "extends_type_clause"
            | "trait_bounds"
            | "bounds"
            | "argument_list" => {
                let mut c = n.walk();
                for child in n.named_children(&mut c) {
                    stack.push(child);
                }
            }
            "call_expression" => {
                // `extends mixin(Base, Other)`: only the first identifier argument.
                if let Some(args) = n.child_by_field_name("arguments") {
                    let mut c = args.walk();
                    for arg in args.named_children(&mut c) {
                        if matches!(
                            arg.kind(),
                            "identifier"
                                | "type_identifier"
                                | "member_expression"
                                | "scoped_type_identifier"
                        ) {
                            if let Some(chain) = dotted_chain(arg, src) {
                                fx.inherits.push((class_idx, chain));
                            }
                            break;
                        }
                    }
                }
            }
            "generic_type"
            | "type_identifier"
            | "identifier"
            | "member_expression"
            | "nested_type_identifier"
            | "scoped_type_identifier"
            | "attribute"
            | "subscript" => {
                if let Some(chain) = dotted_chain(n, src) {
                    fx.inherits.push((class_idx, chain));
                }
            }
            _ => {}
        }
    }
}

fn define_method_ts(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, ctx: &mut WalkCtx) {
    // method_definition: name field is a property_identifier inside a class body.
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Some(class_qn) = ctx.class_stack.last().cloned() else {
        return;
    };
    let name = text(name_node, src);
    let qualname = format!("{class_qn}.{name}");
    let start = node.start_position().row + 1;
    let sym_idx = fx.symbols.len();
    let (cyc, br, nest, loc) = callable_metrics(node, src, fx.lang);
    fx.symbols.push(NewSymbol {
        kind: "method",
        name: name.clone(),
        qualname: qualname.clone(),
        file_idx: fx.file_idx,
        signature: signature_of(node, src),
        line_start: start,
        line_end: node.end_position().row + 1,
        cyclomatic: cyc,
        branches: br,
        nesting: nest,
        effective_loc: loc,
        ..Default::default()
    });
    if let Some(params) = node.child_by_field_name("parameters") {
        collect_param_types(params, src, fx, &qualname);
    }
    if let Some(pidx) = parent_symbol_idx(fx, ctx) {
        fx.edges.push(NewEdge {
            src_idx: pidx,
            dst_idx: Some(sym_idx),
            dst_name: qualname.clone(),
            kind: "contains",
            confidence: "EXTRACTED",
            site_line: start,
        });
    }
    ctx.func_stack.push(qualname);
    if let Some(body) = node.child_by_field_name("body") {
        walk_children(body, src, fx, ctx);
    }
    ctx.func_stack.pop();
}

/// Module-level `const X = () => {...}` / `= function() {...}` -> function
/// symbol (React components and hooks live here).
fn record_ts_arrow_function(
    node: Node<'_>,
    src: &[u8],
    fx: &mut FileExtraction,
    ctx: &mut WalkCtx,
) {
    if !ctx.class_stack.is_empty() || !ctx.func_stack.is_empty() {
        return; // nested functions are not indexed symbols
    }
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Some(value) = node.child_by_field_name("value") else {
        return;
    };
    if !matches!(
        value.kind(),
        "arrow_function" | "function" | "function_expression"
    ) {
        return;
    }
    let name = text(name_node, src);
    let qualname = format!("{}.{}", fx.module, name);
    let start = node.start_position().row + 1;
    let sym_idx = fx.symbols.len();
    let (cyc, br, nest, loc) = callable_metrics(value, src, fx.lang);
    fx.symbols.push(NewSymbol {
        kind: "function",
        name: name.clone(),
        qualname: qualname.clone(),
        file_idx: fx.file_idx,
        signature: signature_of(node, src),
        line_start: start,
        line_end: node.end_position().row + 1,
        cyclomatic: cyc,
        branches: br,
        nesting: nest,
        effective_loc: loc,
        ..Default::default()
    });
    // Module symbol contains it.
    fx.edges.push(NewEdge {
        src_idx: 0,
        dst_idx: Some(sym_idx),
        dst_name: qualname.clone(),
        kind: "contains",
        confidence: "EXTRACTED",
        site_line: start,
    });
    ctx.func_stack.push(qualname);
    walk_children(value, src, fx, ctx);
    ctx.func_stack.pop();
}

/// TS/TSX/JSX static ESM imports. Relative sources resolve by module path;
/// bare/aliased sources bind as EXTERNAL (calls into them drop with the
/// builtin counter, not the unresolved counter).
fn record_export_ts(node: Node<'_>, src: &[u8], fx: &mut FileExtraction) {
    if let Some(source_node) = node.child_by_field_name("source") {
        let raw = text(source_node, src);
        let source = raw.trim_matches(|c| c == '\'' || c == '"');
        let module = resolve_ts_module(source, &fx.dir);
        fx.import_edges.push(ImportEdgeCandidate {
            line: node.start_position().row + 1,
            target: ImportTarget::Module { path: module },
        });
    }
}

fn record_import_ts(node: Node<'_>, src: &[u8], fx: &mut FileExtraction) {
    let line = node.start_position().row + 1;
    let Some(source_node) = node.child_by_field_name("source") else {
        return;
    };
    let raw = text(source_node, src);
    let source = raw.trim_matches(|c| c == '\'' || c == '"');
    let module = resolve_ts_module(source, &fx.dir);
    fx.import_edges.push(ImportEdgeCandidate {
        line,
        target: ImportTarget::Module {
            path: module.clone(),
        },
    });
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "import_clause" {
            continue;
        }
        let mut c2 = child.walk();
        for part in child.children(&mut c2) {
            match part.kind() {
                "identifier" => {
                    // default import: import X from "..."
                    let name = text(part, src);
                    fx.aliases.push((
                        name.clone(),
                        ImportTarget::Symbol {
                            module: module.clone(),
                            name: "default".to_string(),
                        },
                    ));
                }
                "namespace_import" => {
                    // * as ns — bind ns to the module itself
                    let mut c3 = part.walk();
                    for p2 in part.children(&mut c3) {
                        if p2.kind() == "identifier" {
                            fx.aliases.push((
                                text(p2, src),
                                ImportTarget::ModuleRoot {
                                    path: module.clone(),
                                },
                            ));
                        }
                    }
                }
                "named_imports" => {
                    let mut c3 = part.walk();
                    for spec in part.children(&mut c3) {
                        if spec.kind() != "import_specifier" {
                            continue;
                        }
                        let Some(name_node) = spec.child_by_field_name("name") else {
                            continue;
                        };
                        let name = text(name_node, src);
                        let bound = spec
                            .child_by_field_name("alias")
                            .map(|a| text(a, src))
                            .unwrap_or_else(|| name.clone());
                        fx.aliases.push((
                            bound,
                            ImportTarget::Symbol {
                                module: module.clone(),
                                name: name.clone(),
                            },
                        ));
                        fx.import_edges.push(ImportEdgeCandidate {
                            line,
                            target: ImportTarget::Symbol {
                                module: module.clone(),
                                name,
                            },
                        });
                    }
                }
                _ => {}
            }
        }
    }
}

/// Resolve a TS import source to a module path, relative to the FILE'S
/// DIRECTORY (stems may contain dots, so the directory — not the module
/// string — is the only trustworthy base). Bare sources map to dotted
/// external names; the resolver treats non-indexed modules as external.
fn resolve_ts_module(source: &str, dir: &str) -> String {
    let stripped = source
        .trim_end_matches(".tsx")
        .trim_end_matches(".ts")
        .trim_end_matches(".jsx")
        .trim_end_matches(".js");
    if stripped.starts_with("./") || stripped.starts_with("../") {
        let mut parts: Vec<&str> = if dir.is_empty() {
            Vec::new()
        } else {
            dir.split('.').collect()
        };
        let mut segs: Vec<&str> = stripped.split('/').collect();
        while segs.first() == Some(&"..") {
            segs.remove(0);
            parts.pop();
        }
        if segs.first() == Some(&".") {
            segs.remove(0);
        }
        for seg in segs {
            if !seg.is_empty() {
                parts.push(seg);
            }
        }
        parts.join(".")
    } else {
        stripped.replace('/', ".")
    }
}

fn impl_type_name(node: Node<'_>, src: &[u8]) -> String {
    fn nominal(t: Node<'_>, src: &[u8]) -> Option<String> {
        match t.kind() {
            "type_identifier" => Some(text(t, src)),
            "generic_type" => {
                let mut c = t.walk();
                for ch in t.named_children(&mut c) {
                    if ch.kind() == "type_identifier" {
                        return Some(text(ch, src));
                    }
                }
                None
            }
            "reference_type" => t
                .child_by_field_name("type")
                .and_then(|inner| nominal(inner, src)),
            _ => None,
        }
    }
    node.child_by_field_name("type")
        .and_then(|t| nominal(t, src))
        .unwrap_or_else(|| "UnknownImpl".to_string())
}

/// Rust use declarations: `use crate::error::Result;` binds the last segment.
/// crate:: maps to the file's top-level directory (the crate root dir);
/// external crates (std, clap, ...) bind as external and drop at call time.
fn record_use_decl(node: Node<'_>, src: &[u8], fx: &mut FileExtraction) {
    let line = node.start_position().row + 1;
    let mut stack = vec![node];
    let mut crate_root: Option<String> = None;
    if let Some(first_seg) = fx.module.split('.').next() {
        crate_root = Some(first_seg.to_string());
    }
    while let Some(n) = stack.pop() {
        if n.kind() == "scoped_identifier" {
            let full = text(n, src);
            let parts: Vec<&str> = full.split("::").collect();
            if parts.len() >= 2 {
                let name = parts.last().unwrap().to_string();
                let mut path_parts: Vec<String> = parts[..parts.len() - 1]
                    .iter()
                    .map(|p| p.to_string())
                    .collect();
                if path_parts.first().map(|p| p.as_str()) == Some("crate") {
                    if let Some(root) = &crate_root {
                        path_parts[0] = root.clone();
                    } else {
                        path_parts.remove(0);
                    }
                }
                let module = path_parts.join(".");
                fx.aliases.push((
                    name.clone(),
                    ImportTarget::Symbol {
                        module: module.clone(),
                        name: name.clone(),
                    },
                ));
                fx.import_edges.push(ImportEdgeCandidate {
                    line,
                    target: ImportTarget::Module {
                        path: module.clone(),
                    },
                });
                fx.import_edges.push(ImportEdgeCandidate {
                    line,
                    target: ImportTarget::Symbol { module, name },
                });
            }
            continue;
        }
        let mut c = n.walk();
        for child in n.children(&mut c) {
            stack.push(child);
        }
    }
}

// ---------------------------------------------------------------------------
// Phase B: global resolution
// ---------------------------------------------------------------------------

struct FileCtx {
    module: String,
    /// Crate-root dir segment for Rust files (e.g. "src") — maps crate:: paths.
    crate_root: Option<String>,
    module_sym_idx: usize,
    aliases: HashMap<String, ImportTarget>,
    stars: Vec<String>,
    /// (scope_func, var) -> constructor target chain; later assignment wins.
    assignments: HashMap<(String, String), Vec<String>>,
    /// (scope_func, var) -> annotation chain.
    params: HashMap<(String, String), Vec<String>>,
    /// (class qualname, field) -> constructor target chain.
    self_fields: HashMap<(String, String), Vec<String>>,
    calls: Vec<CallSite>,
    import_edges: Vec<ImportEdgeCandidate>,
    /// (src idx global, base chain)
    inherits: Vec<(usize, Vec<String>)>,
    /// (src idx global, base chain, impl line) — edge-only, no class_bases.
    impl_inherits: Vec<(usize, Vec<String>, usize)>,
}

struct Tables {
    symbols: Vec<NewSymbol>,
    existing_ids: Vec<Option<i64>>,
    edges: Vec<NewEdge>,
    /// Direct module children only (Python bare-name visibility).
    by_mod_name: HashMap<(String, String), Vec<usize>>,
    by_qualname: HashMap<String, Vec<usize>>,
    class_methods: HashMap<String, HashMap<String, Vec<usize>>>,
    /// class qualname -> resolved base qualnames
    class_bases: HashMap<String, Vec<String>>,
    module_paths: HashSet<String>,
    file_ctx: Vec<FileCtx>,
}

enum Outcome {
    Extracted(usize),
    Ambiguous(String),
    Dropped,
    Unresolved,
}

pub fn resolve(mut files: Vec<FileExtraction>, carried: CarriedIndex) -> Resolution {
    let mut t = Tables {
        symbols: Vec::new(),
        existing_ids: Vec::new(),
        edges: Vec::new(),
        by_mod_name: HashMap::new(),
        by_qualname: HashMap::new(),
        class_methods: HashMap::new(),
        class_bases: HashMap::new(),
        module_paths: HashSet::new(),
        file_ctx: Vec::new(),
    };

    for fx in files.drain(..) {
        let base = t.symbols.len();
        let module = fx.module.clone();
        t.module_paths.insert(module.clone());
        for s in fx.symbols {
            let idx = t.symbols.len();
            let qualname = s.qualname.clone();
            let kind = s.kind;
            let name = s.name.clone();
            t.symbols.push(s);
            t.existing_ids.push(None);
            t.by_qualname.entry(qualname.clone()).or_default().push(idx);
            let direct_child = qualname == format!("{module}.{name}");
            if kind != "method" && direct_child {
                t.by_mod_name
                    .entry((module.clone(), name.clone()))
                    .or_default()
                    .push(idx);
            }
            if kind == "method" {
                let class_qn = qualname
                    .rsplit_once('.')
                    .map(|(p, _)| p.to_string())
                    .unwrap_or_default();
                t.class_methods
                    .entry(class_qn)
                    .or_default()
                    .entry(name)
                    .or_default()
                    .push(idx);
            }
            if kind == "class" {
                t.class_methods.entry(qualname).or_default();
            }
        }
        for mut e in fx.edges {
            e.src_idx += base;
            if let Some(d) = e.dst_idx.as_mut() {
                *d += base;
            }
            t.edges.push(e);
        }
        let inherits = fx
            .inherits
            .into_iter()
            .map(|(src, chain)| (src + base, chain))
            .collect();
        let impl_inherits = fx
            .impl_inherits
            .into_iter()
            .map(|(src, chain, line)| (src + base, chain, line))
            .collect();
        let mut aliases = HashMap::new();
        for (alias, target) in fx.aliases {
            aliases.insert(alias, target); // later rebinding wins
        }
        let mut assignments = HashMap::new();
        for a in fx.assignments {
            assignments.insert((a.scope_func, a.var), a.target_dotted);
        }
        let mut params = HashMap::new();
        for p in fx.params {
            params.insert((p.scope_func, p.var), p.ann_dotted);
        }
        let mut self_fields = HashMap::new();
        for sf in fx.self_fields {
            self_fields.insert((sf.class_qn, sf.field), sf.target_dotted);
        }
        let crate_root = fx.dir.split('.').next().map(|p| p.to_string());
        t.file_ctx.push(FileCtx {
            module,
            crate_root,
            module_sym_idx: base,
            aliases,
            stars: fx.stars,
            assignments,
            params,
            self_fields,
            calls: fx.calls,
            import_edges: fx.import_edges,
            inherits,
            impl_inherits,
        });
    }

    // Merge carried symbols (fast path): join the resolution tables; their
    // rows are already stored and are never re-inserted.
    let CarriedIndex {
        symbols: carried_syms,
        inherits: carried_inherits,
    } = carried;
    for c in carried_syms {
        let idx = t.symbols.len();
        t.module_paths.insert(c.module.clone());
        t.symbols.push(NewSymbol {
            kind: c.kind,
            name: c.name.clone(),
            qualname: c.qualname.clone(),
            file_idx: usize::MAX,
            line_start: 0,
            line_end: 0,
            signature: String::new(),

            ..Default::default()
        });
        t.existing_ids.push(Some(c.row_id));
        t.by_qualname
            .entry(c.qualname.clone())
            .or_default()
            .push(idx);
        let direct_child = c.qualname == format!("{}.{}", c.module, c.name);
        if c.kind != "method" && direct_child {
            t.by_mod_name
                .entry((c.module.clone(), c.name.clone()))
                .or_default()
                .push(idx);
        }
        if c.kind == "method" {
            let class_qn = c
                .qualname
                .rsplit_once('.')
                .map(|(p, _)| p.to_string())
                .unwrap_or_default();
            t.class_methods
                .entry(class_qn)
                .or_default()
                .entry(c.name)
                .or_default()
                .push(idx);
        }
        if c.kind == "class" {
            t.class_methods.entry(c.qualname).or_default();
        }
    }
    for (sub_qn, base_qn) in carried_inherits {
        t.class_bases.entry(sub_qn).or_default().push(base_qn);
    }

    let mut unresolved_calls: Vec<UnresolvedSite> = Vec::new();

    // Resolve fresh base chains now that alias tables exist; emit inherits
    // edges for fresh classes (carried inherits edges were carried as rows).
    for fctx in &t.file_ctx {
        for (src_idx, chain) in &fctx.inherits {
            let class_file = t.symbols[*src_idx].file_idx;
            let src_qn = t.symbols[*src_idx].qualname.clone();
            let site = t.symbols[*src_idx].line_start;
            let base_qn = resolve_base_chain(&t, class_file, chain, &src_qn);
            let dsts = base_qn
                .as_ref()
                .and_then(|qn| t.by_qualname.get(qn))
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let class_dsts: Vec<usize> = dsts
                .iter()
                .copied()
                .filter(|i| matches!(t.symbols[*i].kind, "class" | "trait" | "interface"))
                .collect();
            if class_dsts.len() > 1 {
                t.edges.push(NewEdge {
                    src_idx: *src_idx,
                    dst_idx: None,
                    dst_name: base_qn.clone().unwrap_or_else(|| chain.join(".")),
                    kind: "inherits",
                    confidence: "AMBIGUOUS",
                    site_line: site,
                });
                unresolved_calls.push(UnresolvedSite {
                    file_idx: class_file,
                    name: base_qn.unwrap_or_else(|| chain.join(".")),
                    kind: "inherit",
                    site_line: site,
                });
            } else if let Some(&dst) = class_dsts.first() {
                let qn = t.symbols[dst].qualname.clone();
                t.class_bases.entry(src_qn).or_default().push(qn.clone());
                t.edges.push(NewEdge {
                    src_idx: *src_idx,
                    dst_idx: Some(dst),
                    dst_name: qn,
                    kind: "inherits",
                    confidence: "EXTRACTED",
                    site_line: site,
                });
            } else {
                unresolved_calls.push(UnresolvedSite {
                    file_idx: class_file,
                    name: base_qn.unwrap_or_else(|| chain.join(".")),
                    kind: "inherit",
                    site_line: site,
                });
            }
        }
        for (src_idx, chain, line) in &fctx.impl_inherits {
            let class_file = t.symbols[*src_idx].file_idx;
            let src_qn = t.symbols[*src_idx].qualname.clone();
            let base_qn = resolve_base_chain(&t, class_file, chain, &src_qn);
            let dsts = base_qn
                .as_ref()
                .and_then(|qn| t.by_qualname.get(qn))
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let class_dsts: Vec<usize> = dsts
                .iter()
                .copied()
                .filter(|i| matches!(t.symbols[*i].kind, "class" | "trait" | "interface"))
                .collect();
            if class_dsts.len() > 1 {
                t.edges.push(NewEdge {
                    src_idx: *src_idx,
                    dst_idx: None,
                    dst_name: base_qn.clone().unwrap_or_else(|| chain.join(".")),
                    kind: "inherits",
                    confidence: "AMBIGUOUS",
                    site_line: *line,
                });
                unresolved_calls.push(UnresolvedSite {
                    file_idx: class_file,
                    name: base_qn.unwrap_or_else(|| chain.join(".")),
                    kind: "inherit",
                    site_line: *line,
                });
            } else if let Some(&dst) = class_dsts.first() {
                t.edges.push(NewEdge {
                    src_idx: *src_idx,
                    dst_idx: Some(dst),
                    dst_name: t.symbols[dst].qualname.clone(),
                    kind: "inherits",
                    confidence: "EXTRACTED",
                    site_line: *line,
                });
            } else {
                unresolved_calls.push(UnresolvedSite {
                    file_idx: class_file,
                    name: base_qn.unwrap_or_else(|| chain.join(".")),
                    kind: "inherit",
                    site_line: *line,
                });
            }
        }
    }

    let mut stats = ExtractStats::default();
    let mut unresolved_per_file = vec![0usize; t.file_ctx.len()];

    // Import edges.
    for (fi, fctx) in t.file_ctx.iter().enumerate() {
        for cand in &fctx.import_edges {
            match &cand.target {
                ImportTarget::Symbol { module, name } => {
                    if let Some(dsts) = t.by_mod_name.get(&(module.clone(), name.clone())) {
                        if dsts.len() > 1 {
                            t.edges.push(NewEdge {
                                src_idx: fctx.module_sym_idx,
                                dst_idx: None,
                                dst_name: format!("{module}.{name}"),
                                kind: "imports",
                                confidence: "AMBIGUOUS",
                                site_line: cand.line,
                            });
                        } else if let Some(&dst) = dsts.first() {
                            if dst == fctx.module_sym_idx {
                                continue;
                            }
                            t.edges.push(NewEdge {
                                src_idx: fctx.module_sym_idx,
                                dst_idx: Some(dst),
                                dst_name: format!("{module}.{name}"),
                                kind: "imports",
                                confidence: "EXTRACTED",
                                site_line: cand.line,
                            });
                        }
                    } else {
                        unresolved_calls.push(UnresolvedSite {
                            file_idx: fi,
                            name: format!("{module}.{name}"),
                            kind: "import",
                            site_line: cand.line,
                        });
                    }
                }
                ImportTarget::Module { path } => {
                    if fctx.stars.iter().any(|m| m == path) {
                        continue; // the star edge below is the canonical one
                    }
                    if let Some(dsts) = t.by_qualname.get(path) {
                        let dst = dsts.iter().find(|&&i| t.symbols[i].kind == "module");
                        if let Some(&dst) = dst {
                            if dst != fctx.module_sym_idx {
                                t.edges.push(NewEdge {
                                    src_idx: fctx.module_sym_idx,
                                    dst_idx: Some(dst),
                                    dst_name: path.clone(),
                                    kind: "imports",
                                    confidence: "EXTRACTED",
                                    site_line: cand.line,
                                });
                            }
                        }
                    } else {
                        unresolved_calls.push(UnresolvedSite {
                            file_idx: fi,
                            name: path.clone(),
                            kind: "import",
                            site_line: cand.line,
                        });
                    }
                }
                ImportTarget::ModuleRoot { path } => {
                    // Star imports use ModuleRoot as their edge target: AMBIGUOUS
                    // edge to the module symbol (spec). Plain `import x` emits no
                    // edge; resolution happens at call time.
                    let is_star = fctx.stars.iter().any(|m| m == path);
                    if is_star {
                        if let Some(dsts) = t.by_qualname.get(path) {
                            if let Some(&dst) = dsts.first() {
                                t.edges.push(NewEdge {
                                    src_idx: fctx.module_sym_idx,
                                    dst_idx: Some(dst),
                                    dst_name: path.clone(),
                                    kind: "imports",
                                    confidence: "AMBIGUOUS",
                                    site_line: cand.line,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // Deduplicate import edges and reconcile unresolved import candidates:
    // a statement that resolved by any candidate is not unresolved.
    {
        let mut seen_edges: HashSet<(usize, Option<usize>, String, usize)> = HashSet::new();
        t.edges.retain(|e| {
            e.kind != "imports"
                || seen_edges.insert((e.src_idx, e.dst_idx, e.dst_name.clone(), e.site_line))
        });
        let mut resolved: HashSet<(usize, usize, String)> = HashSet::new();
        for e in &t.edges {
            if e.kind == "imports" {
                let fi = t.symbols[e.src_idx].file_idx;
                resolved.insert((fi, e.site_line, e.dst_name.clone()));
            }
        }
        unresolved_calls.retain(|u| {
            u.kind != "import" || !resolved.contains(&(u.file_idx, u.site_line, u.name.clone()))
        });
        let mut seen_unresolved: HashSet<(usize, String, &'static str, usize)> = HashSet::new();
        unresolved_calls
            .retain(|u| seen_unresolved.insert((u.file_idx, u.name.clone(), u.kind, u.site_line)));
    }

    // Call sites.
    for (fi, fctx) in t.file_ctx.iter().enumerate() {
        for site in &fctx.calls {
            let outcome = resolve_call(&t, fi, site);
            let src_idx = if site.scope_func.is_empty() {
                fctx.module_sym_idx
            } else {
                t.by_qualname
                    .get(&site.scope_func)
                    .and_then(|v| v.first().copied())
                    .unwrap_or(fctx.module_sym_idx)
            };
            match outcome {
                Outcome::Extracted(dst) => {
                    // R6d A6: keep the edge to the class itself so
                    // `callers Class` and `dead` are correct, and add a
                    // second edge to its constructor when unique.
                    let dst_name = t.symbols[dst].qualname.clone();
                    t.edges.push(NewEdge {
                        src_idx,
                        dst_idx: Some(dst),
                        dst_name: dst_name.clone(),
                        kind: "calls",
                        confidence: "EXTRACTED",
                        site_line: site.line,
                    });
                    if t.symbols[dst].kind == "class" {
                        if let Some(ctor) = ctor_target(&t, dst) {
                            if ctor != dst {
                                t.edges.push(NewEdge {
                                    src_idx,
                                    dst_idx: Some(ctor),
                                    dst_name: t.symbols[ctor].qualname.clone(),
                                    kind: "calls",
                                    confidence: "EXTRACTED",
                                    site_line: site.line,
                                });
                            }
                        }
                    }
                }
                Outcome::Ambiguous(name) => t.edges.push(NewEdge {
                    src_idx,
                    dst_idx: None,
                    dst_name: name,
                    kind: "calls",
                    confidence: "AMBIGUOUS",
                    site_line: site.line,
                }),
                Outcome::Dropped => stats.builtin_dropped += 1,
                Outcome::Unresolved => {
                    stats.unresolved_callees += 1;
                    unresolved_per_file[fi] += 1;
                    let name = match &site.callee {
                        Callee::Name(n) => n.clone(),
                        Callee::Attr(parts) => parts.join("."),
                    };
                    unresolved_calls.push(UnresolvedSite {
                        file_idx: fi,
                        name,
                        kind: "call",
                        site_line: site.line,
                    });
                }
            }
        }
    }

    Resolution {
        symbols: t.symbols,
        existing_ids: t.existing_ids,
        edges: t.edges,
        stats,
        unresolved_per_file,
        unresolved_calls,
    }
}

fn resolve_call(t: &Tables, fi: usize, site: &CallSite) -> Outcome {
    raw_resolve_call(t, fi, site)
}

/// Unique constructor of a class (`__init__` / `constructor`), if any.
/// Instantiations emit edges to BOTH the class and this constructor (R6d A6).
fn ctor_target(t: &Tables, class_idx: usize) -> Option<usize> {
    let class_qn = t.symbols[class_idx].qualname.clone();
    for ctor in ["__init__", "constructor"] {
        match t.class_methods.get(&class_qn).and_then(|m| m.get(ctor)) {
            Some(inits) if inits.len() == 1 => return Some(inits[0]),
            Some(_) => return None, // ambiguous ctor: class edge only
            None => continue,
        }
    }
    None
}

fn raw_resolve_call(t: &Tables, fi: usize, site: &CallSite) -> Outcome {
    let fctx = &t.file_ctx[fi];
    match &site.callee {
        Callee::Name(name) => {
            // 1. Local module definition.
            if let Some(out) = bare_candidates(
                t.by_mod_name.get(&(fctx.module.clone(), name.clone())),
                name,
            ) {
                return out;
            }
            // 2. Import alias map (follows re-export chains transitively).
            match fctx.aliases.get(name) {
                Some(ImportTarget::Symbol { module, name: n }) => {
                    let mut seen: HashSet<(String, String)> = HashSet::new();
                    if let Some(out) = follow_binding(t, module, n, 0, &mut seen, name) {
                        return out;
                    }
                    // Binding to a module that is not indexed at all = an
                    // external library (react, pydantic, os.path, ...): drop.
                    if !t.module_paths.contains(module) {
                        return Outcome::Dropped;
                    }
                }
                Some(ImportTarget::ModuleRoot { .. }) | Some(ImportTarget::Module { .. }) => {
                    // Calling a bare module alias is invalid Python; fall through.
                }
                None => {}
            }
            // Star imports: defer to each star module's top-level names.
            for m in &fctx.stars {
                if let Some(out) =
                    bare_candidates(t.by_mod_name.get(&(m.clone(), name.clone())), name)
                {
                    return out;
                }
            }
            // 4. Builtin table.
            if BUILTINS.contains(&name.as_str()) {
                return Outcome::Dropped;
            }
            Outcome::Unresolved
        }
        Callee::Attr(chain) => {
            if chain.len() < 2 {
                return Outcome::Unresolved;
            }
            let method = chain[1].clone();
            // self/cls (Python) and this (TS) inside a class body.
            if chain[0] == "self" || chain[0] == "cls" || chain[0] == "this" {
                let Some(class_qn) = &site.scope_class else {
                    return Outcome::Unresolved;
                };
                if chain.len() == 2 {
                    return mro_outcome(t, class_qn, &method);
                }
                if chain.len() == 3 && chain[0] == "self" {
                    if let Some(field_chain) =
                        fctx.self_fields.get(&(class_qn.clone(), chain[1].clone()))
                    {
                        if let Some(field_class) = resolve_class_of_chain(t, fi, field_chain) {
                            return mro_outcome(t, &field_class, &chain[2]);
                        }
                    }
                }
                return Outcome::Unresolved;
            }
            // super.m(): skip the own class, resolve against its bases.
            if chain[0] == "super" && chain.len() == 2 {
                let Some(class_qn) = &site.scope_class else {
                    return Outcome::Unresolved;
                };
                if let Some(bases) = t.class_bases.get(class_qn) {
                    for base in bases {
                        let out = mro_outcome(t, base, &method);
                        if !matches!(out, Outcome::Unresolved) {
                            return out;
                        }
                    }
                }
                return Outcome::Unresolved;
            }
            // Receiver typing takes precedence over module interpretation
            // (Amendment 7b): a local binding shadows a same-named module.
            if chain.len() == 2 {
                if let Some(var_class) = resolve_var_class(t, fi, &chain[0], &site.scope_func) {
                    let out = mro_outcome(t, &var_class, &method);
                    if !matches!(out, Outcome::Unresolved) {
                        return out;
                    }
                }
            }
            // Class-attribute call: Class.m(...) or pkg.Class.m(...).
            // R1 fix: a module-bound first segment (import x as y / from pkg
            // import mod) must NOT enter the class branch — `mod.K()` is a
            // constructor through the module path, not a method lookup. And a
            // class-path miss falls through to the module path.
            if !chain_is_module_start(t, fi, &chain[0]) {
                if let Some(class_qn) = resolve_class_of_chain(t, fi, chain) {
                    if chain.len() == 2 {
                        let out = mro_outcome(t, &class_qn, &method);
                        if !matches!(out, Outcome::Unresolved) {
                            return out;
                        }
                    }
                }
                // Constructor receiver: `Base().m()` arrives as ["Base", "m"]
                // (Amendment 18). Resolve the single segment as a class, then
                // the method through its MRO.
                if chain.len() == 2 {
                    if let Some(class_qn) =
                        resolve_class_of_chain(t, fi, std::slice::from_ref(&chain[0]))
                    {
                        let out = mro_outcome(t, &class_qn, &method);
                        if !matches!(out, Outcome::Unresolved) {
                            return out;
                        }
                    }
                }
                // Receiver typed by constructor assignment or annotation.
                if chain.len() == 2 {
                    if let Some(var_class) = resolve_var_class(t, fi, &chain[0], &site.scope_func) {
                        return mro_outcome(t, &var_class, &method);
                    }
                }
            }
            // Rust crate::path::call — map crate:: to the crate root dir.
            if chain[0] == "crate" {
                if let Some(root) = &fctx.crate_root {
                    let out = module_chain_outcome(t, fi, &root_chain(root, chain));
                    if !matches!(out, Outcome::Unresolved) {
                        return out;
                    }
                    // Intermediate segments may be plain namespaces (a src/
                    // tree with no mod.rs): look the whole path up as a
                    // qualname under the crate root (Amendment 7a).
                    let full = format!("{}.{}", root, chain[1..].join("."));
                    if let Some(cands) = t.by_qualname.get(&full) {
                        return match cands.len() {
                            1 => Outcome::Extracted(cands[0]),
                            0 => Outcome::Unresolved,
                            _ => Outcome::Ambiguous(full),
                        };
                    }
                }
                return Outcome::Unresolved;
            }
            // Module-qualified call through the alias map / module tree.
            module_chain_outcome(t, fi, chain)
        }
    }
}

/// Replace a leading `crate` segment with the crate root dir segment.
fn root_chain(root: &str, chain: &[String]) -> Vec<String> {
    let mut out = vec![root.to_string()];
    out.extend_from_slice(&chain[1..]);
    out
}

/// Does the first chain segment bind to a module (alias or bare indexed name)?
fn chain_is_module_start(t: &Tables, fi: usize, first: &str) -> bool {
    let fctx = &t.file_ctx[fi];
    match fctx.aliases.get(first) {
        Some(ImportTarget::ModuleRoot { .. }) | Some(ImportTarget::Module { .. }) => true,
        Some(ImportTarget::Symbol { module, name }) => {
            t.module_paths.contains(&format!("{module}.{name}"))
        }
        None => t.module_paths.contains(first),
    }
}

/// Candidate outcome for a bare-name lookup across one namespace.
/// by_mod_name already holds only direct module children (function/class).
fn bare_candidates(cands: Option<&Vec<usize>>, display: &str) -> Option<Outcome> {
    let v = cands?;
    match v.len() {
        1 => Some(Outcome::Extracted(v[0])),
        0 => None,
        _ => Some(Outcome::Ambiguous(display.to_string())),
    }
}

/// Follow `from m import n` bindings transitively through re-export chains
/// (module m has no def of n but itself imports it from elsewhere), cycle-safe.
fn follow_binding(
    t: &Tables,
    module: &str,
    name: &str,
    depth: usize,
    seen: &mut HashSet<(String, String)>,
    display: &str,
) -> Option<Outcome> {
    if !seen.insert((module.to_string(), name.to_string())) {
        return None;
    }
    if let Some(out) = bare_candidates(
        t.by_mod_name.get(&(module.to_string(), name.to_string())),
        display,
    ) {
        return Some(out);
    }
    if depth >= 4 {
        return None;
    }
    let file_idx = t.file_ctx.iter().position(|f| f.module == module)?;
    let fctx = &t.file_ctx[file_idx];
    match fctx.aliases.get(name) {
        Some(ImportTarget::Symbol {
            module: m2,
            name: n2,
        }) => follow_binding(t, m2, n2, depth + 1, seen, display),
        _ => None,
    }
}

/// MRO walk, level by level: one match at the nearest level wins; several at
/// the same level are AMBIGUOUS; exhausted -> UNRESOLVED.
fn mro_outcome(t: &Tables, class_qn: &str, method: &str) -> Outcome {
    let mut level = vec![class_qn.to_string()];
    let mut seen: HashSet<String> = HashSet::new();
    for _ in 0..6 {
        let mut next: Vec<String> = Vec::new();
        let mut found: Vec<usize> = Vec::new();
        for qn in &level {
            if !seen.insert(qn.clone()) {
                continue;
            }
            if let Some(idxs) = t.class_methods.get(qn).and_then(|m| m.get(method)) {
                found.extend_from_slice(idxs);
            } else if let Some(bases) = t.class_bases.get(qn) {
                next.extend(bases.iter().cloned());
            }
        }
        match found.len() {
            1 => return Outcome::Extracted(found[0]),
            0 if next.is_empty() => return Outcome::Unresolved,
            0 => level = next,
            // Two or more same-level definitions (e.g. diamond): AMBIGUOUS, as
            // documented — never silently drop the call site.
            _ => return Outcome::Ambiguous(format!("{class_qn}.{method}")),
        }
    }
    Outcome::Unresolved
}

/// Receiver typing: annotation first, then constructor assignment (exact
/// function scope, then module scope).
fn resolve_var_class(t: &Tables, fi: usize, var: &str, scope: &str) -> Option<String> {
    let fctx = &t.file_ctx[fi];
    if let Some(ann) = fctx.params.get(&(scope.to_string(), var.to_string())) {
        if let Some(qn) = resolve_class_of_chain(t, fi, ann) {
            return Some(qn);
        }
    }
    for s in [scope, ""] {
        if let Some(target) = fctx.assignments.get(&(s.to_string(), var.to_string())) {
            if let Some(qn) = resolve_class_of_chain(t, fi, target) {
                return Some(qn);
            }
        }
    }
    None
}

/// Resolve a dotted chain to a class qualname via: local module -> alias map ->
/// absolute qualname.
fn resolve_base_chain(
    t: &Tables,
    fi: usize,
    chain: &[String],
    inheriting_qn: &str,
) -> Option<String> {
    let mut parent = inheriting_qn
        .rsplit_once('.')
        .map(|(p, _)| p.to_string())
        .unwrap_or_default();
    loop {
        let candidate = if parent.is_empty() {
            chain.join(".")
        } else {
            format!("{parent}.{}", chain.join("."))
        };
        if class_exists(t, &candidate) {
            return Some(candidate);
        }
        match parent.rsplit_once('.') {
            Some((p, _)) => parent = p.to_string(),
            None => break,
        }
    }
    resolve_class_of_chain(t, fi, chain)
}

fn resolve_class_of_chain(t: &Tables, fi: usize, chain: &[String]) -> Option<String> {
    if chain.is_empty() {
        return None;
    }
    let fctx = &t.file_ctx[fi];
    // 1. Local module class (possibly nested: a.B.C).
    let local = format!("{}.{}", fctx.module, chain.join("."));
    if class_exists(t, &local) {
        return Some(local);
    }
    // 2. Alias map.
    if let Some(target) = fctx.aliases.get(&chain[0]) {
        match target {
            ImportTarget::Symbol { module, name } => {
                let qn = format!("{module}.{name}");
                // Direct hit first; then follow package re-export chains
                // (`from pkg import Base` where pkg/__init__ re-exports it).
                let mut base_qn: Option<String> = None;
                if class_exists(t, &qn) {
                    base_qn = Some(qn.clone());
                } else {
                    let mut seen: HashSet<(String, String)> = HashSet::new();
                    if let Some(Outcome::Extracted(idx)) =
                        follow_binding(t, module, name, 0, &mut seen, name)
                    {
                        if t.symbols[idx].kind == "class" {
                            base_qn = Some(t.symbols[idx].qualname.clone());
                        }
                    }
                }
                if let Some(bq) = base_qn {
                    if chain.len() == 1 {
                        return Some(bq);
                    }
                    let nested = format!("{}.{}", bq, chain[1..].join("."));
                    if class_exists(t, &nested) {
                        return Some(nested);
                    }
                }
            }
            ImportTarget::ModuleRoot { path } => {
                if let Some((module, rest)) = longest_module_prefix(t, path, &chain[1..]) {
                    if !rest.is_empty() {
                        let qn = format!("{}.{}", module, rest.join("."));
                        if class_exists(t, &qn) {
                            return Some(qn);
                        }
                    }
                }
            }
            ImportTarget::Module { .. } => {}
        }
    }
    // 3. Absolute qualname.
    let abs = chain.join(".");
    if class_exists(t, &abs) {
        return Some(abs);
    }
    None
}

fn class_exists(t: &Tables, qualname: &str) -> bool {
    t.by_qualname.get(qualname).is_some_and(|v| {
        v.iter()
            .any(|&i| matches!(t.symbols[i].kind, "class" | "trait" | "interface"))
    })
}

/// Walk the module tree: consume chain segments while the prefix names an
/// indexed module; return (module, remaining segments).
fn longest_module_prefix(
    t: &Tables,
    start: &str,
    rest: &[String],
) -> Option<(String, Vec<String>)> {
    if !t.module_paths.contains(start) {
        // PEP 420 namespace root (R6d I13): the package itself has no
        // __init__.py module symbol, but a deeper module may be indexed
        // (pkg -> pkg.mod). Resolve against the longest indexed prefix.
        let mut probe = start.to_string();
        let mut best: Option<(String, usize)> = None;
        for (i, seg) in rest.iter().enumerate() {
            probe = format!("{probe}.{seg}");
            if t.module_paths.contains(&probe) {
                best = Some((probe.clone(), i + 1));
            }
        }
        let (m, k) = best?;
        return Some((m, rest[k..].to_vec()));
    }
    let mut cur = start.to_string();
    let mut consumed = 0usize;
    for seg in rest {
        let next = format!("{cur}.{seg}");
        if t.module_paths.contains(&next) {
            cur = next;
            consumed += 1;
        } else {
            break;
        }
    }
    Some((cur, rest[consumed..].to_vec()))
}

/// `module.func(...)`, `pkg.mod.Class.m(...)` — resolve through the module tree.
fn module_chain_outcome(t: &Tables, fi: usize, chain: &[String]) -> Outcome {
    let fctx = &t.file_ctx[fi];
    let start: Option<String> = match fctx.aliases.get(&chain[0]) {
        Some(ImportTarget::ModuleRoot { path }) => Some(path.clone()),
        Some(ImportTarget::Module { .. }) => None,
        Some(ImportTarget::Symbol { module, name }) => {
            let qn = format!("{module}.{name}");
            if t.module_paths.contains(&qn) {
                return chain_from_module(t, qn, &chain[1..]);
            }
            // Imported class: ImportedClass.method(...)
            if class_exists(t, &qn) && chain.len() == 2 {
                return mro_outcome(t, &qn, &chain[1]);
            }
            return Outcome::Unresolved;
        }
        None => {
            if t.module_paths.contains(&chain[0]) {
                Some(chain[0].clone())
            } else {
                None
            }
        }
    };
    let Some(start_module) = start else {
        return Outcome::Unresolved;
    };
    chain_from_module(t, start_module, &chain[1..])
}

fn chain_from_module(t: &Tables, module: String, rest: &[String]) -> Outcome {
    if rest.is_empty() {
        return Outcome::Unresolved; // calling a bare module
    }
    if let Some((m, r)) = longest_module_prefix(t, &module, rest) {
        if r.is_empty() {
            return Outcome::Unresolved;
        }
        if r.len() == 1 {
            if let Some(cands) = t.by_mod_name.get(&(m.clone(), r[0].clone())) {
                return match cands.len() {
                    1 => Outcome::Extracted(cands[0]),
                    0 => Outcome::Unresolved,
                    _ => Outcome::Ambiguous(r.join(".")),
                };
            }
        }
        // Dotted path into the module: module.func or module.Class.method.
        let full = format!("{}.{}", m, r.join("."));
        if let Some(cands) = t.by_qualname.get(&full) {
            if cands.len() == 1 {
                return Outcome::Extracted(cands[0]);
            }
            if cands.len() > 1 {
                return Outcome::Ambiguous(full);
            }
        }
        if r.len() >= 2 {
            let class_qn = format!("{}.{}", m, r[..r.len() - 1].join("."));
            if class_exists(t, &class_qn) {
                return mro_outcome(t, &class_qn, r.last().unwrap());
            }
        }
        if STDLIB_MODULES.contains(&m.split('.').next().unwrap_or(&m)) {
            return Outcome::Dropped;
        }
        return Outcome::Unresolved;
    }
    // Start module itself is external (import os; os.path.join(...)).
    if STDLIB_MODULES.contains(&module.split('.').next().unwrap_or(&module)) {
        return Outcome::Dropped;
    }
    Outcome::Unresolved
}

#[cfg(test)]
mod gt_repro_tests {
    use super::*;
    use crate::parse::parse_source;
    use crate::parse::Lang;

    fn extract_src(src: &str) -> FileExtraction {
        let parsed = parse_source(src.as_bytes(), Lang::Python).unwrap();
        extract_file(0, "m.py", src.as_bytes(), &parsed)
    }

    #[test]
    fn typed_param_receiver_call_is_recorded() {
        let fx = extract_src("class A:\n    def f(self, h: Handler):\n        h.handle(1)\n");
        let calls: Vec<_> = fx
            .calls
            .iter()
            .map(|c| (&c.callee, &c.scope_func))
            .collect();
        assert_eq!(calls.len(), 1, "calls recorded: {:?}", calls.len());
        assert!(matches!(calls[0].0, Callee::Attr(_)));
        assert_eq!(fx.params.len(), 1, "params: {:?}", fx.params.len());
    }

    #[test]
    fn call_in_try_block_is_recorded() {
        let fx = extract_src(
            "def g():\n    try:\n        x = foo()\n    except ValueError:\n        pass\n",
        );
        assert_eq!(fx.calls.len(), 1, "calls: {}", fx.calls.len());
    }

    #[test]
    fn self_field_assignment_recorded() {
        let fx = extract_src("class C:\n    def __init__(self):\n        self.res = Resolver()\n");
        assert_eq!(
            fx.self_fields.len(),
            1,
            "self_fields: {}",
            fx.self_fields.len()
        );
    }
}

#[cfg(test)]
mod alias_repro {
    use super::*;
    use crate::parse::{parse_source, Lang};

    #[test]
    fn aliased_from_import_binds_and_emits_edge_candidate() {
        let src = "from ..knowledge.marshal_io import default_artifacts_dir as knowledge_artifacts_dir\n\ndef g():\n    return knowledge_artifacts_dir()\n";
        let parsed = parse_source(src.as_bytes(), Lang::Python).unwrap();
        let fx = extract_file(0, "vihap/validators/gso_loader.py", src.as_bytes(), &parsed);
        assert!(
            fx.aliases
                .iter()
                .any(|(a, _)| a == "knowledge_artifacts_dir"),
            "aliases: {:?}",
            fx.aliases
        );
        assert_eq!(
            fx.import_edges.len(),
            3,
            "import_edges: {} (module, child module, symbol)",
            fx.import_edges.len()
        );
        assert_eq!(fx.calls.len(), 1, "calls: {}", fx.calls.len());
    }
}
