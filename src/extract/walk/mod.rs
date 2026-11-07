//! AST walk: symbol, call, import, and inheritance extraction for one file.
//!
//! The public entry (`extract_file`) lives in the parent; this module owns the
//! node dispatch and the per-file walking state.

use super::*;

mod python;
mod rust;
mod typescript;

pub(super) struct WalkCtx {
    class_stack: Vec<String>, // class/impl qualnames
    mod_stack: Vec<String>,   // inline `mod` qualnames (Rust)
    func_stack: Vec<String>,  // qualnames
    /// Test ancestry (schema v8): inside a Rust `#[test]`/`#[cfg(test)]` item
    /// or a TS `describe(...)`/`it(...)` callback. Save/restore per scope.
    test_scope: bool,
    /// Per-line byte bounds for the Python docstring adjustment, computed once
    /// per file instead of per callable.
    line_bounds: Vec<(usize, usize)>,
}

impl WalkCtx {
    pub(super) fn for_file(lang: Lang, src: &[u8]) -> WalkCtx {
        WalkCtx {
            class_stack: Vec::new(),
            mod_stack: Vec::new(),
            func_stack: Vec::new(),
            test_scope: false,
            line_bounds: if lang == Lang::Python {
                crate::parse::line_byte_bounds(src)
            } else {
                Vec::new()
            },
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

/// One pending traversal step. `Visit` dispatches a node; the restore steps
/// run AFTER every step pushed above them, so a scope push in a match arm can
/// be paired with its pop without recursion: the arm pushes `Pop*` first and
/// the body's children second, and the LIFO order replays the old
/// push/walk/pop sequence (R6d Blocker 1, iterative form).
enum Step<'a> {
    Visit(Node<'a>),
    PopFunc,
    PopClass,
    PopMod,
    RestoreTest(bool),
    /// impl_item post-work: mark methods defined inside a trait impl.
    MarkTraitImpls {
        method_start: usize,
    },
}

/// Queue `node`'s children for visiting in source order (reversed push).
fn push_children_rev<'a>(work: &mut Vec<Step<'a>>, node: Node<'a>) {
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();
    for child in children.into_iter().rev() {
        work.push(Step::Visit(child));
    }
}

/// Walk a subtree with an explicit work stack: traversal depth is heap-bound
/// (bounded by the 1 MiB file cap), never stack-bound, so parse workers run on
/// the default thread stack (R6d Blocker 1, iterative form).
pub(super) fn walk_children(
    node: Node<'_>,
    src: &[u8],
    fx: &mut FileExtraction,
    ctx: &mut WalkCtx,
) {
    let mut work: Vec<Step<'_>> = Vec::new();
    push_children_rev(&mut work, node);
    run_steps(&mut work, src, fx, ctx);
}

fn run_steps<'a>(work: &mut Vec<Step<'a>>, src: &[u8], fx: &mut FileExtraction, ctx: &mut WalkCtx) {
    while let Some(step) = work.pop() {
        match step {
            Step::Visit(node) => visit_node(node, src, fx, ctx, work),
            Step::PopFunc => {
                ctx.func_stack.pop();
            }
            Step::PopClass => {
                ctx.class_stack.pop();
            }
            Step::PopMod => {
                ctx.mod_stack.pop();
            }
            Step::RestoreTest(was) => {
                ctx.test_scope = was;
            }
            Step::MarkTraitImpls { method_start } => {
                for sym in &fx.symbols[method_start..] {
                    if sym.kind == "method" {
                        fx.trait_impl_methods.push(sym.qualname.clone());
                    }
                }
            }
        }
    }
}

fn visit_node<'a>(
    node: Node<'a>,
    src: &[u8],
    fx: &mut FileExtraction,
    ctx: &mut WalkCtx,
    work: &mut Vec<Step<'a>>,
) {
    match node.kind() {
        // ---- Rust definitions (P2.5) ----
        "function_item" => {
            define_function(
                node,
                src,
                fx,
                ctx,
                work,
                if ctx.class_stack.is_empty() {
                    "function"
                } else {
                    "method"
                },
            );
        }
        "struct_item" | "enum_item" => {
            typescript::define_class_ts(node, src, fx, ctx, work, "class");
        }
        "trait_item" => {
            typescript::define_class_ts(node, src, fx, ctx, work, "trait");
        }
        "function_signature_item" => {
            define_function(
                node,
                src,
                fx,
                ctx,
                work,
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
                || rust::has_test_attr(node, src);
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
            if let Some(body) = node.child_by_field_name("body") {
                ctx.mod_stack.push(qualname);
                // `#[cfg(test)] mod tests { ... }` (or any test-attributed
                // item module): every symbol inside inherits the test flag.
                let was_test_scope = ctx.test_scope;
                if rust::has_test_attr(node, src) {
                    ctx.test_scope = true;
                }
                work.push(Step::PopMod);
                work.push(Step::RestoreTest(was_test_scope));
                push_children_rev(work, body);
            }
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
            let children: Vec<_> = node.children(&mut cursor).collect();
            work.push(Step::PopClass);
            // Old semantics: the body child itself is not dispatched as a
            // node; only its children are queued.
            for child in children.into_iter().rev() {
                if child.child_by_field_name("body").is_some() || child.kind() == "statement_block"
                {
                    push_children_rev(work, child);
                }
            }
        }
        "impl_item" => {
            // impl Type { ... } — scope methods under the type name.
            let type_name = rust::impl_type_name(node, src);
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
            work.push(Step::MarkTraitImpls { method_start });
            work.push(Step::PopClass);
            push_children_rev(work, node);
        }
        // ---- TypeScript/TSX/JSX definitions (P2) ----
        "function_declaration" | "generator_function_declaration" => {
            define_function(node, src, fx, ctx, work, "function");
        }
        "class_declaration" | "abstract_class_declaration" => {
            typescript::define_class_ts(node, src, fx, ctx, work, "class");
        }
        "interface_declaration" => {
            typescript::define_class_ts(node, src, fx, ctx, work, "interface");
        }
        "method_definition" => {
            typescript::define_method_ts(node, src, fx, ctx, work);
        }
        "variable_declarator" => {
            if !typescript::record_ts_arrow_function(node, src, fx, ctx, work) {
                typescript::record_ts_variable_binding(node, src, fx, ctx);
                walk_children(node, src, fx, ctx);
            }
        }
        "arrow_function" | "function_expression" if fx.lang != Lang::Python => {
            let start = node.start_position().row + 1;
            let end = node.end_position().row + 1;
            collect_arrow_params(node, src, fx, &ctx.scope_func(), Some((start, end)));
            push_children_rev(work, node);
        }
        "new_expression" => {
            record_call(node, src, fx, ctx);
            push_children_rev(work, node);
        }
        "import_statement" if fx.lang != Lang::Python => {
            typescript::record_import_ts(node, src, fx);
        }
        "export_statement" if fx.lang != Lang::Python => {
            typescript::record_export_ts(node, src, fx);
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
            push_children_rev(work, node);
        }
        "use_declaration" if fx.lang == Lang::Rust => {
            let scope = ctx
                .class_stack
                .last()
                .or_else(|| ctx.mod_stack.last())
                .cloned()
                .unwrap_or_else(|| fx.module.clone());
            rust::record_use_decl(node, src, fx, &scope);
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
                work.push(Step::Visit(def));
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
            let loc = callable_effective_loc(node, src, fx.lang, &fx.code_lines, &ctx.line_bounds);
            fx.symbols.push(NewSymbol {
                kind,
                name: name.clone(),
                qualname: qualname.clone(),
                file_idx: fx.file_idx,
                signature: signature_of(node, src),
                line_start: start,
                line_end: node.end_position().row + 1,
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
                work.push(Step::PopFunc);
                push_children_rev(work, body);
            } else {
                ctx.func_stack.pop();
            }
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
                work.push(Step::PopClass);
                push_children_rev(work, body);
            } else {
                ctx.class_stack.pop();
            }
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
            rust::record_rust_let(node, src, fx, ctx);
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
            rust::record_macro_calls(node, src, fx, ctx);
            walk_children(node, src, fx, ctx);
        }
        "import_statement" | "import_from_statement" => {
            python::record_import(node, src, fx, &ctx.scope_func());
        }
        _ => push_children_rev(work, node),
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
        Lang::Rust => rust::rust_constructor_target(right, src, ctx),
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
                        let module = typescript::resolve_ts_module(source, &fx.dir);
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

fn define_function<'a>(
    node: Node<'a>,
    src: &[u8],
    fx: &mut FileExtraction,
    ctx: &mut WalkCtx,
    work: &mut Vec<Step<'a>>,
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
    let loc = if node.child_by_field_name("body").is_some() {
        callable_effective_loc(node, src, fx.lang, &fx.code_lines, &ctx.line_bounds)
    } else {
        0
    };
    fx.symbols.push(NewSymbol {
        kind,
        name: name.clone(),
        qualname: qualname.clone(),
        file_idx: fx.file_idx,
        signature: signature_of(node, src),
        line_start: start,
        line_end: node.end_position().row + 1,
        effective_loc: loc,
        is_test: fx.is_test_file
            || ctx.test_scope
            || crate::extract::is_test_name(&name)
            || (fx.lang == Lang::Rust && rust::has_test_attr(node, src)),
        exported: matches!(fx.lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript)
            && node
                .parent()
                .is_some_and(|p| p.kind() == "export_statement"),
    });
    if matches!(fx.lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript)
        && typescript::ts_default_export_ancestor(node)
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
    if let Some(body) = node.child_by_field_name("body") {
        ctx.func_stack.push(qualname);
        work.push(Step::PopFunc);
        push_children_rev(work, body);
    }
}
