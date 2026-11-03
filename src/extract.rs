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

use std::collections::HashSet;

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
    /// Innermost inline `mod` qualname (Rust), for `self::`/`super::` paths.
    scope_mod: Option<String>,
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
    /// Rust crate root: a directory segment such as "src", or "" for a
    /// repo-root lib.rs/main.rs.
    pub crate_root: Option<String>,
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
    /// (scope_func, var) for parameters and local bindings that shadow outer
    /// names during bare-name resolution.
    locals: Vec<(String, String)>,
    self_fields: Vec<SelfField>,
    /// (scope_func, bound name, target): function-local imports stay local.
    aliases: Vec<(String, String, ImportTarget)>,
    /// Modules appearing in `from m import *`.
    /// Star-imported modules with the statement line (per-statement scope).
    stars: Vec<(String, usize)>,
    /// Python `from pkg import ...` parent module edges: (line, module).
    from_import_parents: HashSet<(usize, String)>,
    /// Unresolvable inheritance sites: (base name, line).
    unresolved_inherits: Vec<(String, usize)>,
    /// Methods declared inside a trait impl; trait impl methods are public
    /// even without `pub`, while inherent methods follow `pub`.
    trait_impl_methods: Vec<String>,
    /// Per-line code flag (comment nodes excluded), computed once per file.
    pub code_lines: Vec<bool>,
    calls: Vec<CallSite>,
    import_edges: Vec<ImportEdgeCandidate>,
}

/// Derive the dot-joined module path from the repo-relative file path.
/// Package/root markers (`__init__.py`, `index.ts(x)`, `index.js(x)`,
/// `mod.rs`, `lib.rs`) collapse into the containing module path.
pub fn module_path_for(rel: &str) -> (String, bool) {
    let file_name = rel.rsplit('/').next().unwrap_or(rel);
    let is_py_pkg = file_name == "__init__.py";
    let is_ts_pkg = file_name == "index.ts"
        || file_name == "index.tsx"
        || file_name == "index.js"
        || file_name == "index.jsx"
        || file_name == "index.mts"
        || file_name == "index.cts"
        || file_name == "index.mjs"
        || file_name == "index.cjs";
    let is_rust_root = file_name == "mod.rs" || file_name == "lib.rs" || file_name == "main.rs";
    if is_py_pkg || is_ts_pkg || is_rust_root {
        let dir = rel
            .strip_suffix(file_name)
            .unwrap_or(rel)
            .trim_end_matches('/')
            .replace('/', ".");
        if dir.is_empty() {
            // A root-level `lib.rs`/`mod.rs` has no containing directory;
            // keep its stem instead of inventing a shared "root" module.
            if is_rust_root {
                return (file_name.trim_end_matches(".rs").to_string(), true);
            }
            return ("root".to_string(), true);
        }
        return (dir, true);
    }
    let stem = rel
        .trim_end_matches(".tsx")
        .trim_end_matches(".ts")
        .trim_end_matches(".jsx")
        .trim_end_matches(".js")
        .trim_end_matches(".mts")
        .trim_end_matches(".cts")
        .trim_end_matches(".mjs")
        .trim_end_matches(".cjs")
        .trim_end_matches(".rs")
        .trim_end_matches(".py");
    if stem.is_empty() {
        return ("root".to_string(), false);
    }
    (stem.replace('/', "."), false)
}

/// Rust crate root for `crate::` paths: the top directory, or the empty
/// string for a repo-root `lib.rs`/`main.rs`. Nested module files keep the
/// same top directory; plain root modules have no crate root.
fn rust_crate_root(rel: &str) -> Option<String> {
    let file_name = rel.rsplit('/').next().unwrap_or(rel);
    match rel.rfind('/') {
        Some(i) => rel[..i]
            .replace('/', ".")
            .split('.')
            .next()
            .map(str::to_string),
        None if file_name == "lib.rs" || file_name == "main.rs" => Some(String::new()),
        None => None,
    }
}

/// Lib API used by tests and fixtures; the bin uses
/// `extract_file_with_code_lines` with the caller's syntax-stats mask.
#[allow(dead_code)]
pub fn extract_file(file_idx: usize, rel: &str, src: &[u8], parsed: &ParsedFile) -> FileExtraction {
    let root = parsed.tree.root_node();
    let stats = crate::parse::syntax_stats(root, src, Lang::from_rel(rel));
    extract_file_with_code_lines(file_idx, rel, src, root, stats.code_lines)
}

/// Extraction with the file comment mask computed by the caller's single
/// `syntax_stats` walk (B1). Keeps the public `extract_file` wrapper for tests.
pub fn extract_file_with_code_lines(
    file_idx: usize,
    rel: &str,
    src: &[u8],
    root: Node<'_>,
    code_lines: Vec<bool>,
) -> FileExtraction {
    let (module, is_package) = module_path_for(rel);
    let lang = Lang::from_rel(rel);
    let crate_root = if lang == Lang::Rust {
        rust_crate_root(rel)
    } else {
        None
    };
    let mod_name = module.rsplit('.').next().unwrap_or(&module).to_string();
    let dir = match rel.rfind('/') {
        Some(i) => rel[..i].replace('/', "."),
        None => String::new(),
    };
    let mut fx = FileExtraction {
        file_idx,
        module: module.clone(),
        crate_root,
        dir,
        is_package,
        lang,
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
        locals: Vec::new(),
        self_fields: Vec::new(),
        aliases: Vec::new(),
        stars: Vec::new(),
        from_import_parents: HashSet::new(),
        unresolved_inherits: Vec::new(),
        trait_impl_methods: Vec::new(),
        code_lines,
        calls: Vec::new(),
        import_edges: Vec::new(),
    };
    let mut ctx = WalkCtx::for_file(lang, src);
    walk_children(root, src, &mut fx, &mut ctx);
    finalize_rust_impls(&mut fx);
    finalize_exports(&mut fx, src, root);
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
/// Two passes: non-method symbols first, then methods, so an `impl Type` that
/// appears before `pub struct Type` still exports its public methods.
fn finalize_exports(fx: &mut FileExtraction, src: &[u8], root: Node<'_>) {
    let lines: Vec<&[u8]> = src.split(|b| *b == b'\n').collect();
    let all_names = if fx.lang == Lang::Python {
        python_all_names(root, src)
    } else {
        None
    };
    let ts_local = if matches!(fx.lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript) {
        ts_local_export_names(root, src)
    } else {
        HashSet::new()
    };
    for i in 0..fx.symbols.len() {
        if fx.symbols[i].kind != "method" {
            fx.symbols[i].exported = export_base(fx, i, all_names.as_ref(), &ts_local);
        }
    }
    for i in 0..fx.symbols.len() {
        if fx.symbols[i].kind == "method" {
            fx.symbols[i].exported = export_method(fx, i, &lines);
        }
    }
}

/// True when `qualname` is exactly `module.name` (no extra nesting). Avoids a
/// per-symbol `format!` allocation in the export walk.
fn is_direct_child(module: &str, name: &str, qualname: &str) -> bool {
    qualname.len() == module.len() + 1 + name.len()
        && qualname.starts_with(module)
        && qualname.as_bytes().get(module.len()) == Some(&b'.')
        && qualname.ends_with(name)
}

fn export_base(
    fx: &FileExtraction,
    i: usize,
    all_names: Option<&HashSet<String>>,
    ts_local: &HashSet<String>,
) -> bool {
    let sym = &fx.symbols[i];
    if sym.kind == "module" {
        return false;
    }
    let top_level = is_direct_child(&fx.module, &sym.name, &sym.qualname);
    match fx.lang {
        Lang::Python => match all_names {
            Some(all) => all.contains(&sym.name),
            None => top_level && !sym.name.starts_with('_'),
        },
        Lang::Rust => rust_is_pub(&sym.signature),
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => {
            if !matches!(sym.kind, "class" | "function" | "interface" | "trait") {
                return false;
            }
            sym.exported || (top_level && ts_local.contains(&sym.name))
        }
    }
}

fn export_method(fx: &FileExtraction, i: usize, lines: &[&[u8]]) -> bool {
    let sym = &fx.symbols[i];
    let parent_qn = sym.qualname.rsplit_once('.').map(|(p, _)| p.to_string());
    let parent = parent_qn
        .as_ref()
        .and_then(|p| fx.symbols.iter().find(|s| &s.qualname == p));
    let parent_exported = parent.map(|s| s.exported).unwrap_or(false);
    let parent_is_module = parent.map(|s| s.kind == "module").unwrap_or(false);
    let parent_is_trait = parent
        .map(|s| matches!(s.kind, "trait" | "interface"))
        .unwrap_or(false);
    let trait_impl_method = fx.trait_impl_methods.contains(&sym.qualname);
    match fx.lang {
        Lang::Python => parent_exported && !sym.name.starts_with('_'),
        Lang::Rust => {
            ((rust_is_pub(&sym.signature) || parent_is_trait || trait_impl_method)
                && parent_exported)
                || (parent_is_module && rust_is_pub(&sym.signature))
        }
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => {
            let source_line = lines
                .get(sym.line_start.saturating_sub(1))
                .map(|l| String::from_utf8_lossy(l).to_string())
                .unwrap_or_default();
            parent_exported
                && !sym.name.starts_with('#')
                && !source_line.contains("private ")
                && !source_line.contains("protected ")
        }
    }
}

/// `pub fn` is public; `pub(crate)`/`pub(super)`/`pub(in ...)` are not.
/// Signatures may be just `pub` when the rest of the header wraps.
fn rust_is_pub(signature: &str) -> bool {
    let sig = signature.trim();
    if sig == "pub" {
        return true;
    }
    sig.strip_prefix("pub")
        .map(|rest| {
            let rest = rest.trim_start();
            !rest.is_empty() && !rest.starts_with('(')
        })
        .unwrap_or(false)
}

/// Names listed by a local `export { X }` (no source specifier). This is a
/// module binding list, so only top-level symbols consume it.
fn ts_local_export_names(root: Node<'_>, src: &[u8]) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut cursor = root.walk();
    for stmt in root.named_children(&mut cursor) {
        if stmt.kind() != "export_statement" || stmt.child_by_field_name("source").is_some() {
            continue;
        }
        let mut stack = vec![stmt];
        while let Some(n) = stack.pop() {
            if n.kind() == "export_specifier" {
                if let Some(name) = n.child_by_field_name("name") {
                    out.insert(text(name, src));
                }
                continue;
            }
            let mut c = n.walk();
            for child in n.children(&mut c) {
                stack.push(child);
            }
        }
    }
    out
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
        } else if let Some(base) = trait_chain.last() {
            fx.unresolved_inherits.push((base.clone(), line));
        }
    }
}

/// Per-language metric tables computed once per file instead of once per
/// callable (B2): the filtered decision kinds and the branch kinds.
mod walk;
pub use walk::split_ident;
use walk::{text, walk_children, WalkCtx};

mod resolve;
pub use resolve::resolve;
