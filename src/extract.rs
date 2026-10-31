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

use crate::parse::ParsedFile;

pub const SCHEMA_VERSION: i32 = 1;

/// Python builtins: calls drop with the builtin counter (spec step 4).
const BUILTINS: &[&str] = &[
    "abs", "aiter", "all", "any", "anext", "ascii", "bin", "bool", "bytearray", "bytes",
    "callable", "chr", "classmethod", "compile", "complex", "delattr", "dict", "dir", "divmod",
    "enumerate", "eval", "exec", "filter", "float", "format", "frozenset", "getattr", "globals",
    "hasattr", "hash", "hex", "id", "input", "int", "isinstance", "issubclass", "iter", "len",
    "list", "locals", "map", "max", "memoryview", "min", "next", "object", "oct", "open", "ord",
    "pow", "print", "property", "range", "repr", "reversed", "round", "set", "setattr", "slice",
    "sorted", "staticmethod", "str", "sum", "super", "tuple", "type", "vars", "zip", "__import__",
    "Exception", "BaseException", "ArithmeticError", "AssertionError", "AttributeError",
    "BlockingIOError", "BrokenPipeError", "BufferError", "ChildProcessError", "ConnectionError",
    "ConnectionRefusedError", "ConnectionResetError", "EOFError", "FileExistsError",
    "FileNotFoundError", "FloatingPointError", "GeneratorExit", "ImportError", "IndentationError",
    "IndexError", "InterruptedError", "IsADirectoryError", "KeyError", "KeyboardInterrupt",
    "LookupError", "MemoryError", "ModuleNotFoundError", "NameError", "NotADirectoryError",
    "NotImplementedError", "OSError", "OverflowError", "PermissionError", "ProcessLookupError",
    "RecursionError", "ReferenceError", "RuntimeError", "StopAsyncIteration", "StopIteration",
    "SyntaxError", "SystemError", "SystemExit", "TabError", "TimeoutError", "TypeError",
    "UnboundLocalError", "UnicodeDecodeError", "UnicodeEncodeError", "UnicodeError",
    "UnicodeTranslateError", "ValueError", "ZeroDivisionError",
];

/// Well-known stdlib module roots: `os.path.join(...)` on unindexed externals
/// drops with the builtin counter instead of flooding the unresolved count.
const STDLIB_MODULES: &[&str] = &[
    "os", "sys", "re", "json", "math", "time", "datetime", "logging", "pathlib", "collections",
    "itertools", "functools", "typing", "dataclasses", "enum", "abc", "io", "csv", "sqlite3",
    "argparse", "subprocess", "shutil", "tempfile", "textwrap", "unicodedata", "uuid", "random",
    "statistics", "copy", "hashlib", "base64", "urllib", "http", "socket", "threading",
    "multiprocessing", "concurrent", "asyncio", "contextlib", "warnings", "unittest", "pickle",
    "glob", "fnmatch", "string", "struct", "zlib", "gzip", "tarfile", "zipfile", "heapq",
    "bisect", "array", "weakref", "types", "operator", "inspect", "traceback", "atexit",
    "signal", "stat", "errno", "platform", "getpass", "difflib", "secrets", "gc", "os.path",
];

#[derive(Debug, Clone)]
pub struct NewSymbol {
    pub kind: &'static str, // module | class | function | method
    pub name: String,
    pub qualname: String,
    pub file_idx: usize,
    /// 1-based lines (tree-sitter rows are 0-based; offset at emission).
    pub line_start: usize,
    pub line_end: usize,
}

#[derive(Debug, Clone)]
pub struct NewEdge {
    /// Index into the merged symbols vector.
    pub src_idx: usize,
    pub dst_idx: Option<usize>,
    pub dst_name: String,
    pub kind: &'static str, // calls | imports | contains | inherits
    pub confidence: &'static str, // EXTRACTED | AMBIGUOUS
    pub site_line: usize,
}

#[derive(Debug, Default, Clone)]
pub struct ExtractStats {
    pub builtin_dropped: usize,
    pub unresolved_callees: usize,
}

pub struct Resolution {
    pub symbols: Vec<NewSymbol>,
    /// Parallel to `symbols`: Some(row_id) for symbols carried forward from the
    /// previous index (already stored; not re-inserted), None for fresh rows.
    pub existing_ids: Vec<Option<i64>>,
    pub edges: Vec<NewEdge>,
    pub stats: ExtractStats,
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
    pub is_package: bool,
    pub symbols: Vec<NewSymbol>,
    /// contains edges (indices local to this file's symbols; offset on merge).
    pub edges: Vec<NewEdge>,
    /// (src local idx, base dotted chain)
    inherits: Vec<(usize, Vec<String>)>,
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
/// The package marker (`__init__.py`) collapses into the package path.
pub fn module_path_for(rel: &str) -> (String, bool) {
    if rel == "__init__.py" || rel.ends_with("/__init__.py") {
        let pkg = rel
            .strip_suffix("__init__.py")
            .unwrap_or(rel)
            .trim_end_matches('/')
            .replace('/', ".");
        if pkg.is_empty() {
            return ("root".to_string(), true);
        }
        return (pkg, true);
    }
    let stem = rel.strip_suffix(".py").unwrap_or(rel);
    if stem.is_empty() {
        return ("root".to_string(), false);
    }
    (stem.replace('/', "."), false)
}

pub fn extract_file(file_idx: usize, rel: &str, src: &[u8], parsed: &ParsedFile) -> FileExtraction {
    let (module, is_package) = module_path_for(rel);
    let mod_name = module.rsplit('.').next().unwrap_or(&module).to_string();
    let mut fx = FileExtraction {
        file_idx,
        module: module.clone(),
        is_package,
        symbols: vec![NewSymbol {
            kind: "module",
            name: mod_name,
            qualname: module,
            file_idx,
            line_start: 1,
            line_end: 1,
        }],
        edges: Vec::new(),
        inherits: Vec::new(),
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
    fx
}

#[derive(Default)]
struct WalkCtx {
    class_stack: Vec<String>, // qualnames
    func_stack: Vec<String>,  // qualnames
}

impl WalkCtx {
    fn scope_func(&self) -> String {
        self.func_stack.last().cloned().unwrap_or_default()
    }
}

fn text(node: Node<'_>, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").to_string()
}

/// Dotted identifier chain for identifier/attribute nodes: `a.b.c` -> [a, b, c].
/// Unwraps `type` wrapper nodes (annotations).
fn dotted_chain(node: Node<'_>, src: &[u8]) -> Option<Vec<String>> {
    match node.kind() {
        "identifier" => Some(vec![text(node, src)]),
        "attribute" => {
            let obj = node.child_by_field_name("object")?;
            let attr = node.child_by_field_name("attribute")?;
            let mut chain = dotted_chain(obj, src)?;
            chain.push(text(attr, src));
            Some(chain)
        }
        "type" => {
            let mut cursor = node.walk();
            let inner = node.named_children(&mut cursor).next();
            inner.and_then(|ch| dotted_chain(ch, src))
        }
        _ => None,
    }
}

fn walk_children(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, ctx: &mut WalkCtx) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_node(child, src, fx, ctx);
    }
}

fn walk_node(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, ctx: &mut WalkCtx) {
    match node.kind() {
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
            let Some(name_node) = node.child_by_field_name("name") else { return };
            let name = text(name_node, src);
            let kind = if ctx.class_stack.is_empty() { "function" } else { "method" };
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
                line_start: start,
                line_end: node.end_position().row + 1,
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
            let Some(name_node) = node.child_by_field_name("name") else { return };
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
                kind: "class",
                name: name.clone(),
                qualname: qualname.clone(),
                file_idx: fx.file_idx,
                line_start: start,
                line_end: node.end_position().row + 1,
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
        "call" => {
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
            let (Some(ident), Some(ty)) = (ident, ty) else { continue };
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

/// Constructor-target scan for receiver typing: direct call, or the LAST
/// resolvable call in the RHS subtree (`a or B(...)` idiom). Handles both
/// `var = Ctor(...)` (function-local) and `self.field = Ctor(...)` (class-wide).
fn record_assignment(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, ctx: &mut WalkCtx) {
    let Some(left) = node.child_by_field_name("left") else { return };
    let Some(right) = node.child_by_field_name("right") else { return };
    let mut target: Option<Vec<String>> = None;
    let mut stack = vec![right];
    while let Some(n) = stack.pop() {
        if n.kind() == "call" {
            if let Some(f) = n.child_by_field_name("function") {
                if let Some(chain) = dotted_chain(f, src) {
                    target = Some(chain);
                }
            }
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            stack.push(child);
        }
    }
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
    let Some(f) = node.child_by_field_name("function") else { return };
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
                    }
                    "alias" => {
                        let Some(target) = child.child_by_field_name("name") else { continue };
                        let Some(alias) = child.child_by_field_name("alias") else { continue };
                        fx.aliases.push((
                            text(alias, src),
                            ImportTarget::ModuleRoot { path: text(target, src) },
                        ));
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
                            ImportTarget::Symbol { module: m.clone(), name: name.clone() },
                        ));
                        fx.import_edges.push(ImportEdgeCandidate {
                            line,
                            target: ImportTarget::Symbol { module: m.clone(), name },
                        });
                    }
                    // `from m import n as y` wraps the binding in an
                    // aliased_import node (import_statement uses "alias").
                    "aliased_import" | "alias" => {
                        let Some(target) = child.child_by_field_name("name") else { continue };
                        let Some(alias) = child.child_by_field_name("alias") else { continue };
                        let name = text(target, src);
                        fx.aliases.push((
                            text(alias, src),
                            ImportTarget::Symbol { module: m.clone(), name: name.clone() },
                        ));
                        fx.import_edges.push(ImportEdgeCandidate {
                            line,
                            target: ImportTarget::Symbol { module: m.clone(), name },
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
    if rest.is_empty() {
        parts.join(".")
    } else {
        format!("{}.{}", parts.join("."), rest)
    }
}

// ---------------------------------------------------------------------------
// Phase B: global resolution
// ---------------------------------------------------------------------------

struct FileCtx {
    module: String,
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
        t.file_ctx.push(FileCtx {
            module,
            module_sym_idx: base,
            aliases,
            stars: fx.stars,
            assignments,
            params,
            self_fields,
            calls: fx.calls,
            import_edges: fx.import_edges,
            inherits,
        });
    }

    // Merge carried symbols (fast path): join the resolution tables; their
    // rows are already stored and are never re-inserted.
    let CarriedIndex { symbols: carried_syms, inherits: carried_inherits } = carried;
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
        });
        t.existing_ids.push(Some(c.row_id));
        t.by_qualname.entry(c.qualname.clone()).or_default().push(idx);
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

    // Resolve fresh base chains now that alias tables exist; emit inherits
    // edges for fresh classes (carried inherits edges were carried as rows).
    for fctx in &t.file_ctx {
        for (src_idx, chain) in &fctx.inherits {
            let class_file = t.symbols[*src_idx].file_idx;
            if let Some(base_qn) = resolve_class_of_chain(&t, class_file, chain) {
                t.class_bases
                    .entry(t.symbols[*src_idx].qualname.clone())
                    .or_default()
                    .push(base_qn.clone());
                if let Some(dsts) = t.by_qualname.get(&base_qn) {
                    if let Some(&dst) = dsts.first() {
                        t.edges.push(NewEdge {
                            src_idx: *src_idx,
                            dst_idx: Some(dst),
                            dst_name: base_qn,
                            kind: "inherits",
                            confidence: "EXTRACTED",
                            site_line: t.symbols[*src_idx].line_start,
                        });
                    }
                }
            }
        }
    }

    let mut stats = ExtractStats::default();

    // Import edges.
    for fctx in &t.file_ctx {
        for cand in &fctx.import_edges {
            match &cand.target {
                ImportTarget::Symbol { module, name } => {
                    if let Some(dsts) = t.by_mod_name.get(&(module.clone(), name.clone())) {
                        if let Some(&dst) = dsts.first() {
                            t.edges.push(NewEdge {
                                src_idx: fctx.module_sym_idx,
                                dst_idx: Some(dst),
                                dst_name: format!("{module}.{name}"),
                                kind: "imports",
                                confidence: "EXTRACTED",
                                site_line: cand.line,
                            });
                        }
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
                Outcome::Extracted(dst) => t.edges.push(NewEdge {
                    src_idx,
                    dst_idx: Some(dst),
                    dst_name: t.symbols[dst].qualname.clone(),
                    kind: "calls",
                    confidence: "EXTRACTED",
                    site_line: site.line,
                }),
                Outcome::Ambiguous(name) => t.edges.push(NewEdge {
                    src_idx,
                    dst_idx: None,
                    dst_name: name,
                    kind: "calls",
                    confidence: "AMBIGUOUS",
                    site_line: site.line,
                }),
                Outcome::Dropped => stats.builtin_dropped += 1,
                Outcome::Unresolved => stats.unresolved_callees += 1,
            }
        }
    }

    Resolution { symbols: t.symbols, existing_ids: t.existing_ids, edges: t.edges, stats }
}

fn resolve_call(t: &Tables, fi: usize, site: &CallSite) -> Outcome {
    let outcome = raw_resolve_call(t, fi, site);
    redirect_ctor(t, outcome)
}

/// Instantiating a class calls its `__init__`: redirect class-symbol targets
/// to the class's own `__init__` method when it exists.
fn redirect_ctor(t: &Tables, outcome: Outcome) -> Outcome {
    let Outcome::Extracted(idx) = outcome else { return outcome };
    if t.symbols[idx].kind != "class" {
        return Outcome::Extracted(idx);
    }
    let class_qn = t.symbols[idx].qualname.clone();
    match t
        .class_methods
        .get(&class_qn)
        .and_then(|m| m.get("__init__"))
    {
        Some(inits) if inits.len() == 1 => Outcome::Extracted(inits[0]),
        _ => Outcome::Extracted(idx), // no (or ambiguous) __init__: keep the class
    }
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
                }
                Some(ImportTarget::ModuleRoot { .. }) => {
                    // Calling a bare module alias is invalid Python; fall through.
                }
                None => {}
            }
            // Star imports: defer to each star module's top-level names.
            for m in &fctx.stars {
                if let Some(out) = bare_candidates(t.by_mod_name.get(&(m.clone(), name.clone())), name) {
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
            // self.m() / cls.m() / self.field.m() inside a class body.
            if chain[0] == "self" || chain[0] == "cls" {
                let Some(class_qn) = &site.scope_class else { return Outcome::Unresolved };
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
            // Class-attribute call: Class.m(...) or pkg.Class.m(...).
            if let Some(class_qn) = resolve_class_of_chain(t, fi, chain) {
                if chain.len() == 2 {
                    return mro_outcome(t, &class_qn, &method);
                }
            }
            // Receiver typed by constructor assignment or annotation.
            if chain.len() == 2 {
                if let Some(var_class) = resolve_var_class(t, fi, &chain[0], &site.scope_func) {
                    return mro_outcome(t, &var_class, &method);
                }
            }
            // Module-qualified call through the alias map / module tree.
            module_chain_outcome(t, fi, chain)
        }
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
        Some(ImportTarget::Symbol { module: m2, name: n2 }) => {
            follow_binding(t, m2, n2, depth + 1, seen, display)
        }
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
            _ => level = next,
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
                if chain.len() == 1 {
                    if class_exists(t, &qn) {
                        return Some(qn);
                    }
                } else {
                    let nested = format!("{}.{}", qn, chain[1..].join("."));
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
    t.by_qualname
        .get(qualname)
        .is_some_and(|v| v.iter().any(|&i| t.symbols[i].kind == "class"))
}

/// Walk the module tree: consume chain segments while the prefix names an
/// indexed module; return (module, remaining segments).
fn longest_module_prefix(
    t: &Tables,
    start: &str,
    rest: &[String],
) -> Option<(String, Vec<String>)> {
    if !t.module_paths.contains(start) {
        return None;
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
    use crate::parse::parse_python;

    fn extract_src(src: &str) -> FileExtraction {
        let parsed = parse_python(src.as_bytes()).unwrap();
        extract_file(0, "m.py", src.as_bytes(), &parsed)
    }

    #[test]
    fn typed_param_receiver_call_is_recorded() {
        let fx = extract_src(
            "class A:\n    def f(self, h: Handler):\n        h.handle(1)\n",
        );
        let calls: Vec<_> = fx.calls.iter().map(|c| (&c.callee, &c.scope_func)).collect();
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
        let fx = extract_src(
            "class C:\n    def __init__(self):\n        self.res = Resolver()\n",
        );
        assert_eq!(fx.self_fields.len(), 1, "self_fields: {}", fx.self_fields.len());
    }
}

#[cfg(test)]
mod alias_repro {
    use super::*;
    use crate::parse::parse_python;

    #[test]
    fn aliased_from_import_binds_and_emits_edge_candidate() {
        let src = "from ..knowledge.marshal_io import default_artifacts_dir as knowledge_artifacts_dir\n\ndef g():\n    return knowledge_artifacts_dir()\n";
        let parsed = parse_python(src.as_bytes()).unwrap();
        let fx = extract_file(0, "vihap/validators/gso_loader.py", src.as_bytes(), &parsed);
        assert!(
            fx.aliases.iter().any(|(a, _)| a == "knowledge_artifacts_dir"),
            "aliases: {:?}",
            fx.aliases
        );
        assert_eq!(fx.import_edges.len(), 1, "import_edges: {}", fx.import_edges.len());
        assert_eq!(fx.calls.len(), 1, "calls: {}", fx.calls.len());
    }
}
