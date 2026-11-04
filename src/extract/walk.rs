//! AST walk: symbol, call, import, and inheritance extraction for one file.
//!
//! The public entry (`extract_file`) lives in the parent; this module owns the
//! node dispatch and the per-file walking state.

use super::*;

pub(super) struct MetricTables {
    decisions: Vec<&'static str>,
    branches: &'static [&'static str],
}

impl MetricTables {
    fn for_lang(lang: Lang) -> MetricTables {
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
        MetricTables {
            decisions,
            branches: branch_kinds(lang),
        }
    }
}

pub(super) struct WalkCtx {
    class_stack: Vec<String>, // class/impl qualnames
    mod_stack: Vec<String>,   // inline `mod` qualnames (Rust)
    func_stack: Vec<String>,  // qualnames
    depth: usize,             // AST recursion depth (R6d Blocker 1 guard)
    /// Test ancestry (schema v8): inside a Rust `#[test]`/`#[cfg(test)]` item
    /// or a TS `describe(...)`/`it(...)` callback. Save/restore per scope.
    test_scope: bool,
    /// Per-line byte bounds for the Python docstring adjustment, computed once
    /// per file instead of per callable.
    line_bounds: Vec<(usize, usize)>,
    metrics: MetricTables,
}

impl WalkCtx {
    pub(super) fn for_file(lang: Lang, src: &[u8]) -> WalkCtx {
        WalkCtx {
            class_stack: Vec::new(),
            mod_stack: Vec::new(),
            func_stack: Vec::new(),
            depth: 0,
            test_scope: false,
            line_bounds: if lang == Lang::Python {
                crate::parse::line_byte_bounds(src)
            } else {
                Vec::new()
            },
            metrics: MetricTables::for_lang(lang),
        }
    }

    fn scope_func(&self) -> String {
        self.func_stack.last().cloned().unwrap_or_default()
    }
}

pub(super) fn text(node: Node<'_>, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").to_string()
}

/// One-line signature (Amendments 12a and 18): capture the declaration
/// header, continue across line wraps inside a parameter list, stop at the
/// body delimiter (`{`, `;`, or Python `:`), collapse whitespace, and cap
/// 160 chars. Never stores the body.
pub(super) fn signature_of(node: Node<'_>, src: &[u8]) -> String {
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
    let mut collapsed = String::with_capacity(raw.len());
    for word in raw.split_whitespace() {
        if !collapsed.is_empty() {
            collapsed.push(' ');
        }
        collapsed.push_str(word);
    }
    collapsed.chars().take(160).collect()
}

/// Lowercased token split of an identifier path for symbol search
/// (Amendment 12b): camelCase humps, snake_case underscores, dots and other
/// separators all become single spaces. `HierarchicalAddressParser.parse`
/// -> "hierarchical address parser parse". Known limitation (pinned by test):
/// consecutive-capital runs are not split — `HTTPServer` stays one token —
/// so acronym-led identifiers match by full prefix only.
pub fn split_ident(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    let mut chars = s.chars().peekable();
    let mut prev: Option<char> = None;
    while let Some(ch) = chars.next() {
        if ch == '_' || !ch.is_alphanumeric() {
            out.push(' ');
            prev = Some(ch);
            continue;
        }
        if ch.is_uppercase() {
            if let Some(prev_char) = prev {
                let next_lower = chars.peek().is_some_and(|c| c.is_lowercase());
                // Split lowerCase|UPPER and the last capital of an acronym
                // before a lowercase run ("HTTPServer" -> http server) (R7).
                if prev_char.is_lowercase()
                    || prev_char.is_numeric()
                    || (prev_char.is_uppercase() && next_lower)
                {
                    out.push(' ');
                }
            }
        }
        out.push(ch.to_ascii_lowercase());
        prev = Some(ch);
    }
    out
}

/// Dotted identifier chain for identifier/attribute nodes: `a.b.c` -> [a, b, c].
/// Unwraps `type` wrapper nodes (annotations). Rust path keywords (`crate`,
/// `self`, `super`) join the chain as plain segments.
pub(super) fn dotted_chain(node: Node<'_>, src: &[u8]) -> Option<Vec<String>> {
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
        // Rust scoped type paths in impl/trait positions: crate::Shape,
        // std::fmt::Display. Treat them like scoped_identifier so a qualified
        // impl target still produces an edge or unresolved row.
        "scoped_type_identifier" => {
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
        "type_annotation" => {
            let inner = if let Some(ty) = node.child_by_field_name("type") {
                Some(ty)
            } else {
                let mut cursor = node.walk();
                let first = node.named_children(&mut cursor).next();
                first
            };
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
        // Rust receiver and field access: `self.foo()` / `x.foo()`. The
        // field_expression carries `value` (receiver) and `field` (member);
        // `self` is its own node kind in the Rust grammar.
        "self" => Some(vec![text(node, src)]),
        "field_identifier" => Some(vec![text(node, src)]),
        "field_expression" => {
            let value = node.child_by_field_name("value")?;
            let field = node.child_by_field_name("field")?;
            let mut chain = dotted_chain(value, src)?;
            chain.push(text(field, src));
            Some(chain)
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

pub(super) fn walk_children(
    node: Node<'_>,
    src: &[u8],
    fx: &mut FileExtraction,
    ctx: &mut WalkCtx,
) {
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
            let parent = ctx.class_stack.last().or_else(|| ctx.mod_stack.last());
            let qualname = match parent {
                Some(parent) => format!("{parent}.{name}"),
                // A repo-root lib.rs/main.rs is the crate root; its inline
                // modules share the file-module namespace (`a`, not `lib.a`),
                // so `crate::a` resolves for file and inline modules alike.
                None if fx.crate_root.as_deref() == Some("") => name.clone(),
                None => format!("{}.{}", fx.module, name),
            };
            let sym_idx = fx.symbols.len();
            let item_is_test = fx.is_test_file
                || ctx.test_scope
                || crate::extract::is_test_name(&name)
                || has_test_attr(node, src);
            fx.symbols.push(NewSymbol {
                kind: "module",
                name,
                qualname: qualname.clone(),
                file_idx: fx.file_idx,
                signature: String::new(),
                line_start: node.start_position().row + 1,
                line_end: node.end_position().row + 1,

                is_test: item_is_test,
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
            ctx.mod_stack.push(qualname);
            if let Some(body) = node.child_by_field_name("body") {
                // `#[cfg(test)] mod tests { ... }` (or any test-attributed
                // item module): every symbol inside inherits the test flag.
                let was_test_scope = ctx.test_scope;
                if has_test_attr(node, src) {
                    ctx.test_scope = true;
                }
                walk_children(body, src, fx, ctx);
                ctx.test_scope = was_test_scope;
            }
            ctx.mod_stack.pop();
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
            let item_is_test =
                fx.is_test_file || ctx.test_scope || crate::extract::is_test_name(&name);
            fx.symbols.push(NewSymbol {
                kind: "module",
                name,
                qualname: qualname.clone(),
                file_idx: fx.file_idx,
                signature: String::new(),
                line_start: node.start_position().row + 1,
                line_end: node.end_position().row + 1,

                is_test: item_is_test,
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
            let parent = ctx.class_stack.last().or_else(|| ctx.mod_stack.last());
            let qualname = match parent {
                Some(parent) => format!("{parent}.{type_name}"),
                None => format!("{}.{}", fx.module, type_name),
            };
            let trait_impl = node
                .child_by_field_name("trait")
                .and_then(|trait_node| dotted_chain(trait_node, src));
            if let Some(chain) = &trait_impl {
                fx.rust_impls.push((
                    qualname.clone(),
                    chain.clone(),
                    node.start_position().row + 1,
                ));
            }
            let method_start = fx.symbols.len();
            ctx.class_stack.push(qualname);
            walk_children(node, src, fx, ctx);
            ctx.class_stack.pop();
            if trait_impl.is_some() {
                for sym in &fx.symbols[method_start..] {
                    if sym.kind == "method" {
                        fx.trait_impl_methods.push(sym.qualname.clone());
                    }
                }
            }
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
            if !record_ts_arrow_function(node, src, fx, ctx) {
                record_ts_variable_binding(node, src, fx, ctx);
                walk_children(node, src, fx, ctx);
            }
        }
        "arrow_function" | "function_expression" if fx.lang != Lang::Python => {
            let start = node.start_position().row + 1;
            let end = node.end_position().row + 1;
            collect_arrow_params(node, src, fx, &ctx.scope_func(), Some((start, end)));
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
            // `mod db;` declares a child of the current module (inline module
            // or file module), not of the crate root.
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, src);
                if node.child_by_field_name("body").is_none() {
                    let scope = ctx
                        .class_stack
                        .last()
                        .cloned()
                        .unwrap_or_else(|| fx.module.clone());
                    let path = if ctx.class_stack.is_empty() && fx.crate_root.as_deref() == Some("")
                    {
                        name.clone()
                    } else {
                        format!("{scope}.{name}")
                    };
                    fx.aliases.push((
                        String::new(),
                        name,
                        ImportTarget::ModuleRoot { path: path.clone() },
                    ));
                    fx.import_edges.push(ImportEdgeCandidate {
                        line: node.start_position().row + 1,
                        target: ImportTarget::Module { path },
                    });
                }
            }
            walk_children(node, src, fx, ctx);
        }
        "use_declaration" if fx.lang == Lang::Rust => {
            let scope = ctx
                .class_stack
                .last()
                .or_else(|| ctx.mod_stack.last())
                .cloned()
                .unwrap_or_else(|| fx.module.clone());
            record_use_decl(node, src, fx, &scope);
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
            // Innermost lexical scope wins: class > enclosing function when
            // the class is nested in that function, and function > class when
            // a nested function sits inside a method.
            let parent_qn = innermost_scope(ctx, &fx.module);
            let kind = if ctx.class_stack.last() == Some(&parent_qn) {
                "method"
            } else {
                "function"
            };
            let qualname = format!("{parent_qn}.{name}");
            let start = node.start_position().row + 1;
            let sym_idx = fx.symbols.len();
            let (cyc, br, nest, loc) = callable_metrics(
                node,
                src,
                fx.lang,
                &fx.code_lines,
                &ctx.line_bounds,
                &ctx.metrics,
            );
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
                is_test: fx.is_test_file || ctx.test_scope || crate::extract::is_test_name(&name),
                ..Default::default()
            });
            if let Some(params) = node.child_by_field_name("parameters") {
                collect_param_types(params, src, fx, &qualname, kind == "method");
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
            // Innermost lexical scope wins: a class nested in a function
            // belongs to that function; a class nested in a method belongs to
            // the method.
            let parent_qn = innermost_scope(ctx, &fx.module);
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

                is_test: fx.is_test_file || ctx.test_scope || crate::extract::is_test_name(&name),
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
        "augmented_assignment" if fx.lang == Lang::Python => {
            record_assignment(node, src, fx, ctx);
            walk_children(node, src, fx, ctx);
        }
        "assignment_expression" if fx.lang != Lang::Python => {
            record_reassignment(node, src, fx, ctx);
            walk_children(node, src, fx, ctx);
        }
        "let_declaration" if fx.lang == Lang::Rust => {
            record_rust_let(node, src, fx, ctx);
            walk_children(node, src, fx, ctx);
        }
        "call" | "call_expression" => {
            record_call(node, src, fx, ctx);
            // TS/JS test frameworks: declarations inside describe/it callbacks
            // are test-scoped (schema v8 is_test).
            let ts_test_call = matches!(fx.lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript)
                && matches!(
                    node.child_by_field_name("function")
                        .map(|f| text(f, src))
                        .as_deref(),
                    Some("describe" | "it")
                );
            if ts_test_call {
                let was_test_scope = ctx.test_scope;
                ctx.test_scope = true;
                walk_children(node, src, fx, ctx);
                ctx.test_scope = was_test_scope;
            } else {
                walk_children(node, src, fx, ctx);
            }
        }
        "macro_invocation" if fx.lang == Lang::Rust => {
            record_macro_calls(node, src, fx, ctx);
            walk_children(node, src, fx, ctx);
        }
        "import_statement" | "import_from_statement" => {
            record_import(node, src, fx, &ctx.scope_func());
        }
        _ => walk_children(node, src, fx, ctx),
    }
}

fn innermost_scope(ctx: &WalkCtx, module: &str) -> String {
    [
        ctx.func_stack.last(),
        ctx.class_stack.last(),
        ctx.mod_stack.last(),
    ]
    .into_iter()
    .flatten()
    .max_by_key(|qualname| qualname.len())
    .cloned()
    .unwrap_or_else(|| module.to_string())
}

fn parent_symbol_idx(fx: &FileExtraction, ctx: &WalkCtx) -> Option<usize> {
    let qn = innermost_scope(ctx, &fx.module);
    fx.symbols.iter().position(|s| s.qualname == qn)
}

fn collect_param_types(
    params: Node<'_>,
    src: &[u8],
    fx: &mut FileExtraction,
    scope: &str,
    is_method_receiver: bool,
) {
    let mut cursor = params.walk();
    for p in params.children(&mut cursor) {
        if let Some(var) = param_binding(p, src) {
            if var == "self" || var == "cls" || var == "this" {
                // Only the method's own implicit receiver is skipped; an
                // explicit self/cls/this parameter of a nested function
                // shadows the class receiver.
                if !is_method_receiver {
                    fx.locals.push((scope.to_string(), var));
                }
            } else {
                fx.locals.push((scope.to_string(), var));
            }
        }
        // Rust: `x: Foo`, `x: &Foo`, `x: &mut Foo`. `self`/`&self` parameters
        // carry no type and are skipped.
        if p.kind() == "parameter" {
            let (Some(pattern), Some(ty)) = (
                p.child_by_field_name("pattern"),
                p.child_by_field_name("type"),
            ) else {
                continue;
            };
            let Some(var) = pattern_name(pattern, src) else {
                continue;
            };
            let Some(chain) = dotted_chain(unwrap_ref_type(ty), src) else {
                continue;
            };
            fx.params.push(ParamType {
                var,
                ann_dotted: chain,
                scope_func: scope.to_string(),
            });
            continue;
        }
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

/// Arrow/function-expression parameters. A named module-level arrow owns its
/// symbol scope (`range = None`); an anonymous nested callback is scoped by
/// its source line range so it does not shadow calls outside the callback.
fn collect_arrow_params(
    node: Node<'_>,
    src: &[u8],
    fx: &mut FileExtraction,
    scope: &str,
    range: Option<(usize, usize)>,
) {
    if let Some(params) = node.child_by_field_name("parameters") {
        if range.is_some() {
            let mut cursor = params.walk();
            for p in params.children(&mut cursor) {
                if let Some(var) = param_binding(p, src) {
                    push_arrow_binding(fx, scope, var, range);
                }
            }
        } else {
            collect_param_types(params, src, fx, scope, false);
        }
    }
    if let Some(param) = node.child_by_field_name("parameter") {
        if let Some(var) = param_binding(param, src) {
            push_arrow_binding(fx, scope, var, range);
        }
    }
}

fn push_arrow_binding(
    fx: &mut FileExtraction,
    scope: &str,
    var: String,
    range: Option<(usize, usize)>,
) {
    match range {
        Some((start_line, end_line)) => fx.arrow_params.push(ArrowParam {
            var,
            scope_func: scope.to_string(),
            start_line,
            end_line,
        }),
        None => fx.locals.push((scope.to_string(), var)),
    }
}

/// Binding name of a parameter node: identifier, named pattern, or typed name.
fn param_binding(node: Node<'_>, src: &[u8]) -> Option<String> {
    if node.kind() == "identifier" {
        return Some(text(node, src));
    }
    node.child_by_field_name("name")
        .or_else(|| node.child_by_field_name("pattern"))
        .and_then(|n| pattern_name(n, src))
        .or_else(|| pattern_name(node, src))
}

/// Binding name of a declarator pattern: `x`, `mut x`, `(a, b)` (first name).
fn pattern_name(node: Node<'_>, src: &[u8]) -> Option<String> {
    if node.kind() == "identifier" {
        return Some(text(node, src));
    }
    if node.kind() == "mut_pattern" {
        if let Some(inner) = node.child_by_field_name("pattern") {
            return pattern_name(inner, src);
        }
    }
    let mut cursor = node.walk();
    let ident = node
        .named_children(&mut cursor)
        .find(|c| c.kind() == "identifier");
    ident.map(|c| text(c, src))
}

/// Strip `&`, `&mut`, and raw-pointer wrappers from a Rust type node.
fn unwrap_ref_type(mut node: Node<'_>) -> Node<'_> {
    while matches!(node.kind(), "reference_type" | "pointer_type") {
        match node.child_by_field_name("type") {
            Some(inner) => node = inner,
            None => break,
        }
    }
    node
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
    if matches!(
        left.kind(),
        "pattern_list" | "tuple_pattern" | "list_pattern" | "expression_list" | "list"
    ) {
        record_pattern_shadow(left, src, fx, ctx, node.start_position().row + 1);
        return;
    }
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
    let Some(target) = target else {
        // A simple `x = <unknown>` still overwrites an earlier constructor
        // type; record it as an unknown binding instead of leaving the old
        // assignment in place.
        if left.kind() == "identifier" {
            fx.assignments.push(Assignment {
                var: text(left, src),
                target_dotted: Vec::new(),
                scope_func: ctx.scope_func(),
                site_line: node.start_position().row + 1,
            });
        }
        return;
    };
    match left.kind() {
        "identifier" => fx.assignments.push(Assignment {
            var: text(left, src),
            target_dotted: target,
            scope_func: ctx.scope_func(),
            site_line: node.start_position().row + 1,
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
                            target_dotted: target,
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

/// Rust local receiver typing: `let x: Foo` (annotation) and
/// `let x = Foo::new()` / `let x = Foo { .. }` (construction). Only a direct
/// constructor-shaped value types the binding; factory calls stay unknown.
fn record_rust_let(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, ctx: &mut WalkCtx) {
    let Some(pattern) = node.child_by_field_name("pattern") else {
        return;
    };
    let Some(var) = pattern_name(pattern, src) else {
        return;
    };
    if let Some(ty) = node.child_by_field_name("type") {
        if let Some(chain) = dotted_chain(unwrap_ref_type(ty), src) {
            fx.params.push(ParamType {
                var,
                ann_dotted: chain,
                scope_func: ctx.scope_func(),
            });
        }
        return;
    }
    let Some(value) = node.child_by_field_name("value") else {
        return;
    };
    let value = {
        let mut v = value;
        while v.kind() == "reference_expression" {
            match v.child_by_field_name("value") {
                Some(inner) => v = inner,
                None => break,
            }
        }
        v
    };
    let Some(chain) = rust_constructor_target(value, src, ctx) else {
        return;
    };
    fx.assignments.push(Assignment {
        var,
        target_dotted: chain,
        scope_func: ctx.scope_func(),
        site_line: node.start_position().row + 1,
    });
}

/// Constructor-shaped Rust value: `Foo::new()`, `Self::new()`, or a struct
/// literal. An arbitrary associated function (`A::make() -> B`) says nothing
/// about the returned type and must not type `A`.
fn rust_constructor_target(value: Node<'_>, src: &[u8], ctx: &WalkCtx) -> Option<Vec<String>> {
    let (mut chain, is_struct) = match value.kind() {
        "call_expression" => (
            value
                .child_by_field_name("function")
                .and_then(|f| dotted_chain(f, src))?,
            false,
        ),
        "struct_expression" => (
            value
                .child_by_field_name("name")
                .and_then(|n| dotted_chain(n, src))?,
            true,
        ),
        _ => return None,
    };
    if !is_struct && !matches!(chain.last().map(String::as_str), Some("new" | "default")) {
        return None;
    }
    // `Foo::new()` -> Foo; `crate::m::Foo::new()` -> crate::m::Foo.
    // A bare lowercase call (`make()`) has no knowable type.
    let head = chain.first().map(String::as_str).unwrap_or("");
    let type_like = matches!(head, "crate" | "self" | "super" | "Self")
        || head.chars().next().is_some_and(|c| c.is_uppercase());
    if !type_like {
        return None;
    }
    if head == "Self" {
        // `Self::new()` -> Self: drop the constructor segment before
        // substituting the enclosing impl type.
        if !is_struct {
            if chain.len() < 2 {
                return None;
            }
            chain.pop();
        }
        let class_qn = ctx.class_stack.last()?;
        chain = class_qn.split('.').map(str::to_string).collect();
    } else if !is_struct && chain.len() >= 2 {
        chain.pop();
    }
    Some(chain)
}

/// TS/Rust `value = ...` reassignment. A direct constructor updates the
/// receiver type; an unknown right-hand side records an empty binding so the
/// later call cannot reuse an earlier type.
/// Tuple/list destructuring binds every identifier as an unknown local. The
/// binding shadows outer function names from its site line onward.
fn record_pattern_shadow(
    node: Node<'_>,
    src: &[u8],
    fx: &mut FileExtraction,
    ctx: &WalkCtx,
    site_line: usize,
) {
    if node.kind() == "identifier" {
        fx.assignments.push(Assignment {
            var: text(node, src),
            target_dotted: Vec::new(),
            scope_func: ctx.scope_func(),
            site_line,
        });
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        record_pattern_shadow(child, src, fx, ctx, site_line);
    }
}

fn record_reassignment(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, ctx: &mut WalkCtx) {
    let Some(left) = node.child_by_field_name("left") else {
        return;
    };
    if left.kind() != "identifier" {
        return;
    }
    let Some(right) = node.child_by_field_name("right") else {
        return;
    };
    let target = match fx.lang {
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript if right.kind() == "new_expression" => {
            right
                .child_by_field_name("constructor")
                .and_then(|c| dotted_chain(c, src))
        }
        Lang::Rust => rust_constructor_target(right, src, ctx),
        _ => None,
    };
    fx.assignments.push(Assignment {
        var: text(left, src),
        target_dotted: target.unwrap_or_default(),
        scope_func: ctx.scope_func(),
        site_line: node.start_position().row + 1,
    });
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
    let callee = match dotted_chain(f, src) {
        Some(chain) if chain.len() == 1 => Callee::Name(chain.into_iter().next().unwrap()),
        Some(mut chain) => {
            // `self::f()` and `super::f()` are Rust module paths, distinct
            // from the `self.m()` receiver form. Mark the leading segment so
            // resolution can tell them apart.
            if fx.lang == Lang::Rust && f.kind() == "scoped_identifier" {
                match chain.first().map(String::as_str) {
                    Some("self") => chain[0] = "self::".to_string(),
                    Some("super") => chain[0] = "super::".to_string(),
                    _ => {}
                }
            }
            Callee::Attr(chain)
        }
        None => return, // foo()[i](...): unresolvable, not recorded
    };
    fx.calls.push(CallSite {
        line: node.start_position().row + 1,
        scope_func: ctx.scope_func(),
        scope_class: ctx.class_stack.last().cloned(),
        scope_mod: ctx.mod_stack.last().cloned(),
        callee,
    });
}

/// Rust `#[test]` / `#[cfg(test)]` on the item itself: attributes attach as
/// preceding `attribute_item` siblings, so scan backwards past comments.
fn has_test_attr(node: Node<'_>, src: &[u8]) -> bool {
    let mut prev = node.prev_sibling();
    while let Some(p) = prev {
        match p.kind() {
            "attribute_item" => {
                let t = text(p, src);
                if t.contains("#[test]") || t.contains("#[cfg(test)]") {
                    return true;
                }
            }
            "line_comment" | "block_comment" => {}
            _ => return false,
        }
        prev = p.prev_sibling();
    }
    false
}

/// Assert-family macros parse as flat token trees, so calls inside them never
/// surface as call_expression nodes (e.g. `assert_eq!(fnv1a64(b""), ...)`
/// inside a `#[test]` fn). Scan the token text for `name(` and `a::b(` sites
/// and record them through the regular call pipeline; the scan is bounded so
/// a pathological token tree stays cheap (same spirit as MAX_AST_DEPTH).
const MACRO_CALL_FAMILIES: &[&str] = &[
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
];
const MAX_MACRO_TOKEN_TEXT: usize = 64 * 1024;
const MAX_MACRO_CALLS_PER_SITE: usize = 128;

fn record_macro_calls(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, ctx: &mut WalkCtx) {
    let Some(mac) = node.child_by_field_name("macro") else {
        return;
    };
    let macro_path = text(mac, src);
    let macro_name = macro_path.rsplit("::").next().unwrap_or(&macro_path);
    if !MACRO_CALL_FAMILIES.contains(&macro_name) {
        return;
    }
    let mut cursor = node.walk();
    let Some(tree) = node
        .children(&mut cursor)
        .find(|c| c.kind() == "token_tree")
    else {
        return;
    };
    let line = node.start_position().row + 1;
    for callee in scan_macro_call_names(&text(tree, src)) {
        let callee = if callee.contains("::") {
            Callee::Attr(callee.split("::").map(str::to_string).collect())
        } else {
            Callee::Name(callee)
        };
        fx.calls.push(CallSite {
            line,
            scope_func: ctx.scope_func(),
            scope_class: ctx.class_stack.last().cloned(),
            scope_mod: ctx.mod_stack.last().cloned(),
            callee,
        });
    }
}

/// Byte scanner over macro token text: captures identifier and `::`-path
/// calls, skips string/char literals, and ignores method-call receivers
/// (`x.f(`) whose receiver expression a text scan cannot reconstruct.
fn scan_macro_call_names(raw: &str) -> Vec<String> {
    let text = raw.get(..MAX_MACRO_TOKEN_TEXT).unwrap_or(raw);
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut calls = Vec::new();
    let mut i = 0;
    // Last non-whitespace byte seen; `.`/`)` before an ident marks a method
    // receiver or call result, not a plain path call.
    let mut prev_nonspace = 0u8;
    while i < n && calls.len() < MAX_MACRO_CALLS_PER_SITE {
        match bytes[i] {
            b'"' => i = skip_quoted(bytes, i, b'"'),
            b'\'' => i = skip_char_or_lifetime(bytes, i),
            b if is_ident_start(b) => {
                let start = i;
                while i < n && is_ident_char(bytes[i]) {
                    i += 1;
                }
                let mut name = text[start..i].to_string();
                // `a::b::c(` — walk the path to its final segment.
                while i + 1 < n && bytes[i] == b':' && bytes[i + 1] == b':' {
                    let seg = i + 2;
                    let mut j = seg;
                    while j < n && is_ident_char(bytes[j]) {
                        j += 1;
                    }
                    if j == seg {
                        break;
                    }
                    name.push_str("::");
                    name.push_str(&text[seg..j]);
                    i = j;
                }
                if i < n && bytes[i] == b'(' && prev_nonspace != b'.' && prev_nonspace != b')' {
                    calls.push(name);
                    prev_nonspace = b'(';
                    i += 1;
                }
            }
            _ => {
                if !bytes[i].is_ascii_whitespace() {
                    prev_nonspace = bytes[i];
                }
                i += 1;
            }
        }
    }
    calls
}

fn is_ident_start(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphabetic()
}

fn is_ident_char(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphanumeric()
}

fn skip_quoted(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b if b == quote => return i + 1,
            _ => i += 1,
        }
    }
    i
}

/// `'a'` char literal vs `'a` lifetime token: a char literal closes within a
/// few bytes, a lifetime does not.
fn skip_char_or_lifetime(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() && i - start <= 6 {
        match bytes[i] {
            b'\'' => return i + 1,
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
    // Lifetime: skip the quote plus the identifier that follows.
    while i < bytes.len() && is_ident_char(bytes[i]) {
        i += 1;
    }
    i
}

fn record_import(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, scope: &str) {
    let line = node.start_position().row + 1;
    match node.kind() {
        "import_statement" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "dotted_name" => {
                        let path = text(child, src);
                        let root = path.split('.').next().unwrap_or(&path).to_string();
                        fx.aliases.push((
                            scope.to_string(),
                            root.clone(),
                            ImportTarget::ModuleRoot { path: root },
                        ));
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
                            scope.to_string(),
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
            if !m.is_empty() {
                fx.import_edges.push(ImportEdgeCandidate {
                    line,
                    target: ImportTarget::Module { path: m.clone() },
                });
                fx.from_import_parents.insert((line, m.clone()));
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if Some(child) == module_node {
                    continue;
                }
                match child.kind() {
                    "wildcard_import" => {
                        fx.stars.push((m.clone(), line));
                        fx.import_edges.push(ImportEdgeCandidate {
                            line,
                            target: ImportTarget::ModuleRoot { path: m.clone() },
                        });
                    }
                    "dotted_name" => {
                        let name = text(child, src);
                        // `from .. import X` at a first-level package has an
                        // empty parent: bind the root child directly.
                        let alias = if m.is_empty() {
                            ImportTarget::ModuleRoot { path: name.clone() }
                        } else {
                            ImportTarget::Symbol {
                                module: m.clone(),
                                name: name.clone(),
                            }
                        };
                        fx.aliases.push((scope.to_string(), name.clone(), alias));
                        fx.import_edges.push(ImportEdgeCandidate {
                            line,
                            target: ImportTarget::Module {
                                path: if m.is_empty() {
                                    name.clone()
                                } else {
                                    format!("{m}.{name}")
                                },
                            },
                        });
                        if !m.is_empty() {
                            fx.import_edges.push(ImportEdgeCandidate {
                                line,
                                target: ImportTarget::Symbol {
                                    module: m.clone(),
                                    name,
                                },
                            });
                        }
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
                        let alias_target = if m.is_empty() {
                            ImportTarget::ModuleRoot { path: name.clone() }
                        } else {
                            ImportTarget::Symbol {
                                module: m.clone(),
                                name: name.clone(),
                            }
                        };
                        fx.aliases
                            .push((scope.to_string(), text(alias, src), alias_target));
                        fx.import_edges.push(ImportEdgeCandidate {
                            line,
                            target: ImportTarget::Module {
                                path: if m.is_empty() {
                                    name.clone()
                                } else {
                                    format!("{m}.{name}")
                                },
                            },
                        });
                        if !m.is_empty() {
                            fx.import_edges.push(ImportEdgeCandidate {
                                line,
                                target: ImportTarget::Symbol {
                                    module: m.clone(),
                                    name,
                                },
                            });
                        }
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
fn callable_metrics(
    node: Node<'_>,
    src: &[u8],
    lang: Lang,
    code_lines: &[bool],
    line_bounds: &[(usize, usize)],
    tables: &MetricTables,
) -> (usize, usize, usize, usize) {
    let decisions = &tables.decisions;
    let branches = tables.branches;
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
    (
        cyclomatic,
        branch_count,
        max_nest,
        callable_effective_loc(node, src, lang, code_lines, line_bounds),
    )
}

/// Effective LOC from the file-wide comment mask plus the Python docstring.
/// A line counts when it has a non-whitespace byte outside every comment node;
/// docstring bytes are excluded only when the whole line is inside the string.
fn callable_effective_loc(
    node: Node<'_>,
    src: &[u8],
    lang: Lang,
    code_lines: &[bool],
    line_bounds: &[(usize, usize)],
) -> usize {
    let base_row = node.start_position().row;
    let end_row = node.end_position().row;
    let mut effective = (base_row..=end_row)
        .filter(|row| code_lines.get(*row).copied().unwrap_or(true))
        .count();
    if lang != Lang::Python {
        return effective;
    }
    let Some(body) = node.child_by_field_name("body") else {
        return effective;
    };
    let mut c = body.walk();
    let first = body.named_children(&mut c).next();
    let Some(stmt) = first.filter(|s| s.kind() == "expression_statement") else {
        return effective;
    };
    let mut c2 = stmt.walk();
    let inner = stmt.named_children(&mut c2).next();
    let Some(doc) = inner.filter(|s| s.kind() == "string") else {
        return effective;
    };
    let doc_start = doc.start_byte();
    let doc_end = doc.end_byte();
    for row in doc.start_position().row..=doc.end_position().row {
        let Some(&(start, end)) = line_bounds.get(row) else {
            continue;
        };
        let has_code_outside_doc = src[start..end].iter().enumerate().any(|(i, byte)| {
            if byte.is_ascii_whitespace() {
                return false;
            }
            let pos = start + i;
            !(doc_start <= pos && pos < doc_end)
        });
        if !has_code_outside_doc && code_lines.get(row).copied().unwrap_or(true) {
            effective = effective.saturating_sub(1);
        }
    }
    effective
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
        .func_stack
        .last()
        .or_else(|| ctx.class_stack.last())
        .or_else(|| ctx.mod_stack.last())
        .cloned()
        .unwrap_or_else(|| fx.module.clone());
    let qualname = format!("{parent_qn}.{name}");
    let start = node.start_position().row + 1;
    let sym_idx = fx.symbols.len();
    let (cyc, br, nest, loc) = if node.child_by_field_name("body").is_some() {
        callable_metrics(
            node,
            src,
            fx.lang,
            &fx.code_lines,
            &ctx.line_bounds,
            &ctx.metrics,
        )
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
        is_test: fx.is_test_file
            || ctx.test_scope
            || crate::extract::is_test_name(&name)
            || (fx.lang == Lang::Rust && has_test_attr(node, src)),
        exported: matches!(fx.lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript)
            && node
                .parent()
                .is_some_and(|p| p.kind() == "export_statement"),
    });
    if matches!(fx.lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript)
        && ts_default_export_ancestor(node)
    {
        // `export default function work()` has no `worker.default` symbol;
        // bind `default` to the real exported identity so a default import
        // resolves to `worker.work`.
        fx.aliases.push((
            String::new(),
            "default".to_string(),
            ImportTarget::Symbol {
                module: fx.module.clone(),
                name: name.clone(),
            },
        ));
    }
    if let Some(params) = node.child_by_field_name("parameters") {
        collect_param_types(params, src, fx, &qualname, false);
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
        .or_else(|| ctx.mod_stack.last())
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
        is_test: fx.is_test_file || ctx.test_scope || crate::extract::is_test_name(&name),
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
                // `extends mixin(Base, Other)`: only the first identifier
                // argument; a call with no identifier argument is unresolved.
                let mut resolved = false;
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
                            resolved = true;
                            break;
                        }
                    }
                }
                if !resolved && matches!(fx.lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript) {
                    let name = n
                        .child_by_field_name("function")
                        .map(|f| text(f, src))
                        .unwrap_or_else(|| text(n, src));
                    fx.unresolved_inherits
                        .push((name, n.start_position().row + 1));
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
    let (cyc, br, nest, loc) = callable_metrics(
        node,
        src,
        fx.lang,
        &fx.code_lines,
        &ctx.line_bounds,
        &ctx.metrics,
    );
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
        is_test: fx.is_test_file || ctx.test_scope || crate::extract::is_test_name(&name),
        ..Default::default()
    });
    if let Some(params) = node.child_by_field_name("parameters") {
        collect_param_types(params, src, fx, &qualname, true);
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

/// True when a TS declaration is the target of `export default`.
fn ts_default_export_ancestor(node: Node<'_>) -> bool {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if n.kind() == "export_statement" {
            let mut cursor = n.walk();
            return n.children(&mut cursor).any(|c| c.kind() == "default");
        }
        match n.kind() {
            "variable_declarator"
            | "lexical_declaration"
            | "variable_declaration"
            | "parenthesized_expression" => cur = n.parent(),
            _ => return false,
        }
    }
    false
}

/// True when a TS declaration is wrapped by an `export` statement.
fn ts_exported_ancestor(node: Node<'_>) -> bool {
    let mut cur = node.parent();
    while let Some(n) = cur {
        match n.kind() {
            "export_statement" => return true,
            "variable_declarator"
            | "lexical_declaration"
            | "variable_declaration"
            | "parenthesized_expression" => cur = n.parent(),
            _ => return false,
        }
    }
    false
}

/// Module-level `const X = () => {...}` / `= function() {...}` -> function
/// symbol (React components and hooks live here).
fn record_ts_arrow_function(
    node: Node<'_>,
    src: &[u8],
    fx: &mut FileExtraction,
    ctx: &mut WalkCtx,
) -> bool {
    if !ctx.class_stack.is_empty() || !ctx.func_stack.is_empty() {
        return false; // nested functions are not indexed symbols
    }
    let Some(name_node) = node.child_by_field_name("name") else {
        return false;
    };
    let Some(value) = node.child_by_field_name("value") else {
        return false;
    };
    if !matches!(
        value.kind(),
        "arrow_function" | "function" | "function_expression"
    ) {
        return false;
    }
    let name = text(name_node, src);
    let qualname = format!("{}.{}", fx.module, name);
    let start = value.start_position().row + 1;
    let sym_idx = fx.symbols.len();
    let (cyc, br, nest, loc) = callable_metrics(
        value,
        src,
        fx.lang,
        &fx.code_lines,
        &ctx.line_bounds,
        &ctx.metrics,
    );
    fx.symbols.push(NewSymbol {
        kind: "function",
        name: name.clone(),
        qualname: qualname.clone(),
        file_idx: fx.file_idx,
        signature: signature_of(node, src),
        // Metrics and line range use the same arrow/function-expression node;
        // a multiline `const f =` header must not make loc and lines disagree.
        line_start: start,
        line_end: value.end_position().row + 1,
        cyclomatic: cyc,
        branches: br,
        nesting: nest,
        effective_loc: loc,
        is_test: fx.is_test_file || ctx.test_scope || crate::extract::is_test_name(&name),
        exported: ts_exported_ancestor(node),
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
    ctx.func_stack.push(qualname.clone());
    collect_arrow_params(value, src, fx, &qualname, None);
    walk_children(value, src, fx, ctx);
    ctx.func_stack.pop();
    true
}

/// TS `const value = new C()` receiver binding. A plain call
/// (`const value = factory()`) has no knowable constructed type and is ignored.
fn record_ts_variable_binding(
    node: Node<'_>,
    src: &[u8],
    fx: &mut FileExtraction,
    ctx: &mut WalkCtx,
) {
    if !matches!(fx.lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript) {
        return;
    }
    let Some(name) = node
        .child_by_field_name("name")
        .and_then(|n| pattern_name(n, src))
    else {
        return;
    };
    if let Some(ty) = node.child_by_field_name("type") {
        if let Some(chain) = dotted_chain(ty, src) {
            fx.params.push(ParamType {
                var: name.clone(),
                ann_dotted: chain,
                scope_func: ctx.scope_func(),
            });
        }
    }
    let Some(value) = node.child_by_field_name("value") else {
        return;
    };
    if value.kind() != "new_expression" {
        return;
    }
    let Some(chain) = value
        .child_by_field_name("constructor")
        .and_then(|c| dotted_chain(c, src))
    else {
        return;
    };
    fx.assignments.push(Assignment {
        var: name,
        target_dotted: chain,
        scope_func: ctx.scope_func(),
        site_line: node.start_position().row + 1,
    });
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
                        String::new(),
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
                                String::new(),
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
                            String::new(),
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

/// TS/JS source extensions in resolution order. `.mjs`/`.cjs` import
/// specifiers resolve to `.mts`/`.cts` sources under the same stem.
const TS_SOURCE_EXTENSIONS: &[&str] =
    &[".ts", ".tsx", ".js", ".jsx", ".mts", ".cts", ".mjs", ".cjs"];

/// Resolve a TS import source to a module path, relative to the FILE'S
/// DIRECTORY (stems may contain dots, so the directory — not the module
/// string — is the only trustworthy base). Module identities collapse the
/// source extension and a trailing `index`, matching `module_path_for`, so
/// `./x`, `./x.ts`, `./x.mjs`, and `./x/index.ts` all resolve to the same
/// indexed module. Bare sources map to dotted external names.
fn resolve_ts_module(source: &str, dir: &str) -> String {
    let stripped = TS_SOURCE_EXTENSIONS
        .iter()
        .find_map(|ext| source.strip_suffix(ext))
        .unwrap_or(source);
    let mut parts: Vec<&str> = if stripped.starts_with("./") || stripped.starts_with("../") {
        let mut base: Vec<&str> = if dir.is_empty() {
            Vec::new()
        } else {
            dir.split('.').collect()
        };
        let mut segs: Vec<&str> = stripped.split('/').collect();
        while segs.first() == Some(&"..") {
            segs.remove(0);
            base.pop();
        }
        if segs.first() == Some(&".") {
            segs.remove(0);
        }
        base
    } else {
        Vec::new()
    };
    let segs: Vec<&str> = stripped.split('/').filter(|s| !s.is_empty()).collect();
    for seg in segs {
        if seg == "." || seg == ".." {
            continue;
        }
        parts.push(seg);
    }
    if parts.last() == Some(&"index") {
        parts.pop();
    }
    if parts.is_empty() {
        // Root-level index.* is the repository root module.
        return "root".to_string();
    }
    parts.join(".")
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

#[derive(Clone, Copy)]
struct UseTreeCtx<'a> {
    src: &'a [u8],
    prefix: &'a [String],
    scope: &'a str,
    crate_root: Option<&'a str>,
    line: usize,
}

/// Rust use declarations: resolve the structural use tree (`use_list`,
/// `scoped_use_list`, `use_as_clause`, `use_wildcard`) to module/symbol
/// candidates. `crate::` maps to the file's top-level directory, `self::` to
/// the current module scope, and `super::` to its parent. Plain paths are
/// external crates in the 2018 edition and emit no edge.
fn record_use_decl(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, scope: &str) {
    let Some(arg) = node.child_by_field_name("argument") else {
        return;
    };
    let crate_root = fx.crate_root.clone();
    let ctx = UseTreeCtx {
        src,
        prefix: &[],
        scope,
        crate_root: crate_root.as_deref(),
        line: node.start_position().row + 1,
    };
    walk_use_tree(arg, ctx, fx);
}

fn walk_use_tree(node: Node<'_>, ctx: UseTreeCtx<'_>, fx: &mut FileExtraction) {
    match node.kind() {
        "use_list" => {
            let mut c = node.walk();
            for child in node.named_children(&mut c) {
                walk_use_tree(child, ctx, fx);
            }
        }
        "scoped_use_list" => {
            let Some(path) = node.child_by_field_name("path") else {
                return;
            };
            let Some(base) = resolve_use_path(path, ctx) else {
                return;
            };
            if !base.is_empty() {
                fx.import_edges.push(ImportEdgeCandidate {
                    line: ctx.line,
                    target: ImportTarget::Module {
                        path: base.join("."),
                    },
                });
            }
            if let Some(list) = node.child_by_field_name("list") {
                walk_use_tree(
                    list,
                    UseTreeCtx {
                        prefix: &base,
                        ..ctx
                    },
                    fx,
                );
            }
        }
        "use_as_clause" => {
            let Some(path) = node.child_by_field_name("path") else {
                return;
            };
            let alias = node.child_by_field_name("alias").map(|n| text(n, ctx.src));
            emit_use_leaf(path, ctx, alias, fx);
        }
        "use_wildcard" => {
            // tree-sitter-rust has no `path` field here: `crate::a::*` puts the
            // path in a named child, while a grouped `{ * }` has none.
            let mut c = node.walk();
            let path = node
                .child_by_field_name("path")
                .or_else(|| node.named_children(&mut c).next());
            let full = match path {
                Some(path) => resolve_use_path(path, ctx),
                None => Some(ctx.prefix.to_vec()),
            };
            let Some(full) = full.filter(|p| !p.is_empty()) else {
                return;
            };
            let qn = full.join(".");
            fx.stars.push((qn.clone(), ctx.line));
            fx.import_edges.push(ImportEdgeCandidate {
                line: ctx.line,
                target: ImportTarget::ModuleRoot { path: qn },
            });
        }
        _ => emit_use_leaf(node, ctx, None, fx),
    }
}

fn emit_use_leaf(
    node: Node<'_>,
    ctx: UseTreeCtx<'_>,
    alias: Option<String>,
    fx: &mut FileExtraction,
) {
    let Some(full) = resolve_use_path(node, ctx) else {
        return;
    };
    if full.is_empty() {
        return;
    }
    let qn = full.join(".");
    let raw = text(node, ctx.src);
    let marker_only = raw == "self";
    let after_marker = raw.split("::").skip(1).filter(|p| !p.is_empty()).count();
    let module_leaf = marker_only || (ctx.prefix.is_empty() && after_marker <= 1);
    if module_leaf {
        fx.import_edges.push(ImportEdgeCandidate {
            line: ctx.line,
            target: ImportTarget::Module { path: qn.clone() },
        });
        let binding = alias.or_else(|| {
            if node.kind() == "self" {
                None
            } else {
                full.last().cloned()
            }
        });
        if let Some(binding) = binding {
            if let Some((name, parent)) = full.split_last() {
                if parent.is_empty() {
                    fx.aliases.push((
                        ctx.scope.to_string(),
                        binding,
                        ImportTarget::ModuleRoot { path: qn },
                    ));
                } else {
                    fx.aliases.push((
                        ctx.scope.to_string(),
                        binding,
                        ImportTarget::Symbol {
                            module: parent.join("."),
                            name: name.clone(),
                        },
                    ));
                }
            }
        }
        return;
    }
    let Some((name, parent)) = full.split_last() else {
        return;
    };
    let module = parent.join(".");
    fx.import_edges.push(ImportEdgeCandidate {
        line: ctx.line,
        target: ImportTarget::Module {
            path: module.clone(),
        },
    });
    fx.import_edges.push(ImportEdgeCandidate {
        line: ctx.line,
        target: ImportTarget::Symbol {
            module: module.clone(),
            name: name.clone(),
        },
    });
    fx.aliases.push((
        ctx.scope.to_string(),
        alias.unwrap_or_else(|| name.clone()),
        ImportTarget::Symbol {
            module,
            name: name.clone(),
        },
    ));
}

/// Resolve one use-tree path node against the enclosing module scope.
/// `None` means the path is external/absolute and must not produce an edge.
fn resolve_use_path(node: Node<'_>, ctx: UseTreeCtx<'_>) -> Option<Vec<String>> {
    let raw = text(node, ctx.src);
    if raw.starts_with("::") {
        return None;
    }
    let mut parts = raw
        .split("::")
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    let first = parts.next()?;
    let rest: Vec<String> = parts.collect();
    let extra_supers = rest.iter().take_while(|p| p.as_str() == "super").count();
    let mut plain = true;
    let mut out: Vec<String> = match first.as_str() {
        "crate" => {
            plain = false;
            ctx.crate_root?
                .split('.')
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect()
        }
        "self" if !ctx.prefix.is_empty() => {
            plain = false;
            ctx.prefix.to_vec()
        }
        "self" => {
            plain = false;
            ctx.scope.split('.').map(str::to_string).collect()
        }
        "super" if !ctx.prefix.is_empty() => {
            plain = false;
            let mut scoped = ctx.prefix.to_vec();
            for _ in 0..=extra_supers {
                scoped.pop();
            }
            scoped
        }
        "super" => {
            plain = false;
            let mut scoped: Vec<String> = ctx.scope.split('.').map(str::to_string).collect();
            for _ in 0..=extra_supers {
                scoped.pop();
            }
            scoped
        }
        _ if ctx.prefix.is_empty() => return None,
        _ => ctx.prefix.to_vec(),
    };
    if plain {
        out.push(first);
    }
    out.extend(rest.into_iter().skip(extra_supers));
    Some(out)
}
