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
}

impl WalkCtx {
    pub(super) fn for_file(_lang: Lang, _src: &[u8]) -> WalkCtx {
        WalkCtx {
            class_stack: Vec::new(),
            mod_stack: Vec::new(),
            func_stack: Vec::new(),
            test_scope: false,
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
/// so acronym-led identifiers match by full prefix only. Deliberately
/// separate from the memory-store tokenizer (`memory::mem_tokens`): the two
/// index different stores with different digit/camelCase boundaries
/// ("v2Fix" -> "v2 fix" here vs ["v","2","fix"] there), so a camelCase fix
/// must not silently change the other index.
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
        // Python `a.b` (field `attribute`), TS `a.b` (field `property`), and
        // Rust `a.b` (fields `value`/`field`) share the same chain rule.
        "attribute" | "member_expression" | "field_expression" => {
            let (obj_field, attr_field) = match node.kind() {
                "field_expression" => ("value", "field"),
                "attribute" => ("object", "attribute"),
                _ => ("object", "property"),
            };
            let obj = node.child_by_field_name(obj_field)?;
            let attr = node.child_by_field_name(attr_field)?;
            let mut chain = dotted_chain(obj, src)?;
            chain.push(text(attr, src));
            Some(chain)
        }
        // Rust scoped type paths in impl/trait positions: crate::Shape,
        // std::fmt::Display. Treat them like scoped_identifier so a qualified
        // impl target still produces an edge or unresolved row.
        "scoped_type_identifier" | "scoped_identifier" => scoped_chain(node, src),
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
        "generic_type" => generic_base(node, src),
        // Rust receiver and field access: `self.foo()` / `x.foo()`. The
        // field_expression carries `value` (receiver) and `field` (member);
        // `self` is its own node kind in the Rust grammar.
        "self" => Some(vec![text(node, src)]),
        "field_identifier" => Some(vec![text(node, src)]),
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

/// Rust scoped path (`a::b::c`): append `name` to the chain of `path`.
fn scoped_chain(node: Node<'_>, src: &[u8]) -> Option<Vec<String>> {
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

/// `Trait<T>`: use the first type identifier as the base name.
fn generic_base(node: Node<'_>, src: &[u8]) -> Option<Vec<String>> {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if n.kind() == "type_identifier" {
            return Some(vec![text(n, src)]);
        }
        let mut cursor = n.walk();
        let children: Vec<_> = n.children(&mut cursor).collect();
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
    None
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

/// Entry point for one subtree. The traversal is heap-bound: `run_steps` owns
/// the only loop, and `visit_node` must only QUEUE children on that same stack
/// (`push_children_rev`), never call back into this function. A recursive
/// re-entry would make depth stack-bound and abort the build on deeply nested
/// calls (stack overflow, not catchable by `catch_unwind`).
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
            Step::Visit(node) => {
                let mut w = Walker {
                    node,
                    src,
                    fx: &mut *fx,
                    ctx: &mut *ctx,
                    work: &mut *work,
                };
                visit_node(&mut w);
            }
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
                    if sym.kind == SymbolKind::Method {
                        fx.trait_impl_methods.push(sym.qualname.clone());
                    }
                }
            }
        }
    }
}

/// Dispatch one node by language, then handle the kinds every language shares.
/// The per-language matches keep the original arm precedence: overlapping
/// kinds (`import_statement`, `mod_item`) are separated by language here.
fn visit_node<'a>(w: &mut Walker<'a, '_, '_>) {
    let handled = match w.fx.lang {
        Lang::Rust => visit_rust_node(w),
        Lang::Python => visit_python_node(w),
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => visit_ts_node(w),
    };
    if handled {
        return;
    }
    match w.node.kind() {
        "call" | "call_expression" => visit_call(w),
        _ => push_children_rev(w.work, w.node),
    }
}

fn visit_rust_node<'a>(w: &mut Walker<'a, '_, '_>) -> bool {
    match w.node.kind() {
        "function_item" | "function_signature_item" => visit_function_item(w),
        "struct_item" | "enum_item" => typescript::define_class(w, SymbolKind::Class),
        "trait_item" => typescript::define_class(w, SymbolKind::Trait),
        "mod_item" if w.node.child_by_field_name("body").is_some() => visit_inline_mod(w),
        "impl_item" => visit_impl_item(w),
        "mod_item" => visit_rust_mod_item(w),
        "use_declaration" => {
            let scope = w
                .ctx
                .class_stack
                .last()
                .or_else(|| w.ctx.mod_stack.last())
                .cloned()
                .unwrap_or_else(|| w.fx.module.clone());
            rust::record_use_decl(w.node, w.src, w.fx, &scope);
            push_children_rev(w.work, w.node);
        }
        "let_declaration" => {
            rust::record_rust_let(w.node, w.src, w.fx, w.ctx);
            push_children_rev(w.work, w.node);
        }
        "macro_invocation" => {
            rust::record_macro_calls(w.node, w.src, w.fx, w.ctx);
            push_children_rev(w.work, w.node);
        }
        "assignment_expression" => {
            record_reassignment(w.node, w.src, w.fx, w.ctx);
            push_children_rev(w.work, w.node);
        }
        _ => return false,
    }
    true
}

fn visit_python_node<'a>(w: &mut Walker<'a, '_, '_>) -> bool {
    match w.node.kind() {
        "decorated_definition" => visit_decorated_definition(w),
        "function_definition" => visit_python_function(w),
        "class_definition" => visit_python_class(w),
        "import_statement" | "import_from_statement" => {
            python::record_import(w.node, w.src, w.fx, &w.ctx.scope_func());
        }
        "assignment" | "augmented_assignment" => {
            record_assignment(w.node, w.src, w.fx, w.ctx);
            push_children_rev(w.work, w.node);
        }
        _ => return false,
    }
    true
}

fn visit_ts_node<'a>(w: &mut Walker<'a, '_, '_>) -> bool {
    match w.node.kind() {
        "class_declaration" | "abstract_class_declaration" => {
            typescript::define_class(w, SymbolKind::Class);
        }
        "function_declaration" | "generator_function_declaration" => {
            define_function(w, SymbolKind::Function);
        }
        "interface_declaration" => typescript::define_class(w, SymbolKind::Interface),
        "method_definition" => typescript::define_method_ts(w),
        "variable_declarator" => {
            if !typescript::record_ts_arrow_function(w) {
                typescript::record_ts_variable_binding(w.node, w.src, w.fx, w.ctx);
                push_children_rev(w.work, w.node);
            }
        }
        "arrow_function" | "function_expression" => {
            let start = w.node.start_position().row + 1;
            let end = w.node.end_position().row + 1;
            collect_arrow_params(w.node, w.src, w.fx, &w.ctx.scope_func(), Some((start, end)));
            push_children_rev(w.work, w.node);
        }
        "new_expression" => {
            record_call(w.node, w.src, w.fx, w.ctx);
            push_children_rev(w.work, w.node);
        }
        "import_statement" => typescript::record_import_ts(w.node, w.src, w.fx),
        "export_statement" => {
            typescript::record_export_ts(w.node, w.src, w.fx);
            push_children_rev(w.work, w.node);
        }
        "internal_module" | "module" => visit_internal_module(w),
        "assignment_expression" => {
            record_reassignment(w.node, w.src, w.fx, w.ctx);
            push_children_rev(w.work, w.node);
        }
        _ => return false,
    }
    true
}

/// Register one module/namespace scope symbol.
fn push_scope_symbol(
    fx: &mut FileExtraction,
    node: Node<'_>,
    name: String,
    qualname: &str,
    is_test: bool,
) {
    fx.push_symbol(NewSymbol {
        kind: SymbolKind::Module,
        name,
        qualname: qualname.to_string(),
        file_idx: fx.file_idx,
        signature: String::new(),
        line_start: node.start_position().row + 1,
        line_end: node.end_position().row + 1,
        is_test,
    });
}

/// True when an item inherits the test flag from its file, scope, or name.
fn is_test_item(fx: &FileExtraction, ctx: &WalkCtx, name: &str) -> bool {
    fx.is_test_file || ctx.test_scope || crate::extract::is_test_name(name)
}

/// Record one assignment binding at `line`.
fn push_assignment(
    fx: &mut FileExtraction,
    var: String,
    target_dotted: Vec<String>,
    ctx: &WalkCtx,
    line: usize,
) {
    fx.assignments.push(Assignment {
        var,
        target_dotted,
        scope_func: ctx.scope_func(),
        site_line: line,
    });
}

fn visit_inline_mod<'a>(w: &mut Walker<'a, '_, '_>) {
    // Inline `mod name { ... }` is a scope, not just an alias (R7).
    let name = w
        .node
        .child_by_field_name("name")
        .map(|n| text(n, w.src))
        .unwrap_or_default();
    let parent = w.ctx.class_stack.last().or_else(|| w.ctx.mod_stack.last());
    let qualname = match parent {
        Some(parent) => format!("{parent}.{name}"),
        // A repo-root lib.rs/main.rs is the crate root; its inline
        // modules share the file-module namespace (`a`, not `lib.a`),
        // so `crate::a` resolves for file and inline modules alike.
        None if w.fx.crate_root.as_deref() == Some("") => name.clone(),
        None => format!("{}.{}", w.fx.module, name),
    };
    let has_test_attr = rust::has_test_attr(w.node, w.src);
    let item_is_test = is_test_item(w.fx, w.ctx, &name) || has_test_attr;
    push_scope_symbol(w.fx, w.node, name, &qualname, item_is_test);
    if let Some(body) = w.node.child_by_field_name("body") {
        w.ctx.mod_stack.push(qualname);
        // `#[cfg(test)] mod tests { ... }` (or any test-attributed
        // item module): every symbol inside inherits the test flag.
        let was_test_scope = w.ctx.test_scope;
        if has_test_attr {
            w.ctx.test_scope = true;
        }
        w.work.push(Step::PopMod);
        w.work.push(Step::RestoreTest(was_test_scope));
        push_children_rev(w.work, body);
    }
}

fn visit_rust_mod_item<'a>(w: &mut Walker<'a, '_, '_>) {
    // `mod db;` declares a child of the current module (inline module
    // or file module), not of the crate root.
    if let Some(name_node) = w.node.child_by_field_name("name") {
        let name = text(name_node, w.src);
        if w.node.child_by_field_name("body").is_none() {
            let scope = w
                .ctx
                .class_stack
                .last()
                .cloned()
                .unwrap_or_else(|| w.fx.module.clone());
            let path = if w.ctx.class_stack.is_empty() && w.fx.crate_root.as_deref() == Some("") {
                name.clone()
            } else {
                format!("{scope}.{name}")
            };
            w.fx.aliases.push((
                String::new(),
                name,
                ImportTarget::ModuleRoot { path: path.clone() },
            ));
            w.fx.import_edges.push(ImportEdgeCandidate::module(
                w.node.start_position().row + 1,
                path,
            ));
        }
    }
    push_children_rev(w.work, w.node);
}

fn visit_function_item<'a>(w: &mut Walker<'a, '_, '_>) {
    let kind = if w.ctx.class_stack.is_empty() {
        SymbolKind::Function
    } else {
        SymbolKind::Method
    };
    define_function(w, kind);
}

fn visit_internal_module<'a>(w: &mut Walker<'a, '_, '_>) {
    // TypeScript `namespace N { ... }` (R7).
    let name = w
        .node
        .child_by_field_name("name")
        .map(|n| text(n, w.src))
        .unwrap_or_default();
    let qualname = if w.ctx.class_stack.is_empty() {
        format!("{}.{}", w.fx.module, name)
    } else {
        format!("{}.{}", w.ctx.class_stack.last().unwrap(), name)
    };
    let item_is_test = is_test_item(w.fx, w.ctx, &name);
    push_scope_symbol(w.fx, w.node, name, &qualname, item_is_test);
    w.ctx.class_stack.push(qualname);
    let mut cursor = w.node.walk();
    let children: Vec<_> = w.node.children(&mut cursor).collect();
    w.work.push(Step::PopClass);
    // Old semantics: the body child itself is not dispatched as a
    // node; only its children are queued.
    for child in children.into_iter().rev() {
        if child.child_by_field_name("body").is_some() || child.kind() == "statement_block" {
            push_children_rev(w.work, child);
        }
    }
}

fn visit_impl_item<'a>(w: &mut Walker<'a, '_, '_>) {
    // impl Type { ... } — scope methods under the type name.
    let type_name = rust::impl_type_name(w.node, w.src);
    let parent = w.ctx.class_stack.last().or_else(|| w.ctx.mod_stack.last());
    let qualname = match parent {
        Some(parent) => format!("{parent}.{type_name}"),
        None => format!("{}.{}", w.fx.module, type_name),
    };
    let trait_impl = w
        .node
        .child_by_field_name("trait")
        .and_then(|trait_node| dotted_chain(trait_node, w.src));
    if let Some(chain) = &trait_impl {
        w.fx.rust_impls.push((
            qualname.clone(),
            chain.clone(),
            w.node.start_position().row + 1,
        ));
    }
    let method_start = w.fx.symbols.len();
    w.ctx.class_stack.push(qualname);
    w.work.push(Step::MarkTraitImpls { method_start });
    w.work.push(Step::PopClass);
    push_children_rev(w.work, w.node);
}

fn visit_decorated_definition<'a>(w: &mut Walker<'a, '_, '_>) {
    let mut cursor = w.node.walk();
    let mut decorators = Vec::new();
    for child in w.node.children(&mut cursor) {
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
                record_call(n, w.src, w.fx, w.ctx);
            }
            let mut c = n.walk();
            for child in n.children(&mut c) {
                stack.push(child);
            }
        }
    }
    if let Some(def) = w.node.child_by_field_name("definition") {
        w.work.push(Step::Visit(def));
    }
}

fn visit_python_function<'a>(w: &mut Walker<'a, '_, '_>) {
    let Some(name_node) = w.node.child_by_field_name("name") else {
        return;
    };
    let name = text(name_node, w.src);
    // Innermost lexical scope wins: class > enclosing function when
    // the class is nested in that function, and function > class when
    // a nested function sits inside a method.
    let parent_qn = innermost_scope(w.ctx, &w.fx.module);
    let kind = if w.ctx.class_stack.last() == Some(&parent_qn) {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };
    let qualname = format!("{parent_qn}.{name}");
    let mut spec = w.callable_spec(kind, &name, &qualname);
    spec.always_push_scope = true;
    define_callable(w, &spec);
}

fn visit_python_class<'a>(w: &mut Walker<'a, '_, '_>) {
    let Some(name_node) = w.node.child_by_field_name("name") else {
        return;
    };
    let name = text(name_node, w.src);
    // Innermost lexical scope wins: a class nested in a function
    // belongs to that function; a class nested in a method belongs to
    // the method.
    let parent_qn = innermost_scope(w.ctx, &w.fx.module);
    let qualname = format!("{parent_qn}.{name}");
    let start = w.node.start_position().row + 1;
    let sym_idx = w.fx.symbols.len();
    w.fx.push_symbol(NewSymbol {
        kind: SymbolKind::Class,
        name: name.clone(),
        qualname: qualname.clone(),
        file_idx: w.fx.file_idx,
        signature: signature_of(w.node, w.src),
        line_start: start,
        line_end: w.node.end_position().row + 1,

        is_test: is_test_item(w.fx, w.ctx, &name),
    });
    if let Some(bases) = w.node.child_by_field_name("superclasses") {
        let mut cursor = bases.walk();
        for b in bases.named_children(&mut cursor) {
            if let Some(chain) = dotted_chain(b, w.src) {
                w.fx.inherits.push((sym_idx, chain));
            }
        }
    }
    w.ctx.class_stack.push(qualname);
    if let Some(body) = w.node.child_by_field_name("body") {
        w.work.push(Step::PopClass);
        push_children_rev(w.work, body);
    } else {
        w.ctx.class_stack.pop();
    }
}

fn visit_call<'a>(w: &mut Walker<'a, '_, '_>) {
    record_call(w.node, w.src, w.fx, w.ctx);
    // TS/JS test frameworks: declarations inside describe/it callbacks
    // are test-scoped (schema v8 is_test).
    let ts_test_call = matches!(w.fx.lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript)
        && matches!(
            w.node
                .child_by_field_name("function")
                .map(|f| text(f, w.src))
                .as_deref(),
            Some("describe" | "it")
        );
    if ts_test_call {
        let was_test_scope = w.ctx.test_scope;
        w.ctx.test_scope = true;
        // Queue the restore BEFORE the children so it pops after them
        // (LIFO), keeping the walk on one iterative stack.
        w.work.push(Step::RestoreTest(was_test_scope));
        push_children_rev(w.work, w.node);
    } else {
        push_children_rev(w.work, w.node);
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

/// Symbol fields shared by the callable constructors (function/method).
pub(super) struct CallableSpec<'a> {
    pub kind: SymbolKind,
    pub name: &'a str,
    pub qualname: &'a str,
    pub start: usize,
    pub is_test: bool,
    /// Python pushes the scope before the body check and pops it when there
    /// is no body; Rust/TS push it only when a body exists.
    pub always_push_scope: bool,
}

/// One node's mutable walk state, so the callable helpers need only a spec.
struct Walker<'a, 's, 'b> {
    node: Node<'a>,
    src: &'s [u8],
    fx: &'b mut FileExtraction,
    ctx: &'b mut WalkCtx,
    work: &'b mut Vec<Step<'a>>,
}

impl<'a, 's> Walker<'a, 's, '_> {
    /// Push the symbol row.
    fn push_callable(&mut self, spec: &CallableSpec<'_>) {
        self.fx.push_symbol(NewSymbol {
            kind: spec.kind,
            name: spec.name.to_string(),
            qualname: spec.qualname.to_string(),
            file_idx: self.fx.file_idx,
            signature: signature_of(self.node, self.src),
            line_start: spec.start,
            line_end: self.node.end_position().row + 1,
            is_test: spec.is_test,
        });
    }

    /// Record parameter types, then queue the body with its scope pop.
    fn finish_callable(&mut self, spec: &CallableSpec<'_>) {
        if let Some(params) = self.node.child_by_field_name("parameters") {
            collect_param_types(
                params,
                self.src,
                self.fx,
                spec.qualname,
                spec.kind == SymbolKind::Method,
            );
        }
        if spec.always_push_scope || self.node.child_by_field_name("body").is_some() {
            self.ctx.func_stack.push(spec.qualname.to_string());
        }
        if let Some(body) = self.node.child_by_field_name("body") {
            self.work.push(Step::PopFunc);
            push_children_rev(self.work, body);
        } else if spec.always_push_scope {
            self.ctx.func_stack.pop();
        }
    }
}

/// Push one callable symbol and queue its body under the new scope.
fn define_callable<'a>(w: &mut Walker<'a, '_, '_>, spec: &CallableSpec<'_>) {
    w.push_callable(spec);
    w.finish_callable(spec);
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
        push_param_local(p, src, fx, scope, is_method_receiver);
        push_rust_param(p, src, fx, scope);
        push_ts_param(p, src, fx, scope);
        push_typed_param(p, src, fx, scope);
    }
}

/// Binding-name locals. Only the method's own implicit receiver is skipped; an
/// explicit self/cls/this parameter of a nested function shadows the class
/// receiver.
fn push_param_local(
    p: Node<'_>,
    src: &[u8],
    fx: &mut FileExtraction,
    scope: &str,
    is_method_receiver: bool,
) {
    let Some(var) = param_binding(p, src) else {
        return;
    };
    if (var == "self" || var == "cls" || var == "this") && is_method_receiver {
        return;
    }
    fx.locals.push((scope.to_string(), var));
}

/// Rust: `x: Foo`, `x: &Foo`, `x: &mut Foo`. `self`/`&self` parameters
/// carry no type and are skipped.
fn push_rust_param(p: Node<'_>, src: &[u8], fx: &mut FileExtraction, scope: &str) {
    if p.kind() != "parameter" {
        return;
    }
    let (Some(pattern), Some(ty)) = (
        p.child_by_field_name("pattern"),
        p.child_by_field_name("type"),
    ) else {
        return;
    };
    let Some(var) = pattern_name(pattern, src) else {
        return;
    };
    let Some(chain) = dotted_chain(unwrap_ref_type(ty), src) else {
        return;
    };
    push_param_type(fx, var, Some(chain), scope);
}

/// TS: required_parameter / optional_parameter carry pattern + type.
fn push_ts_param(p: Node<'_>, src: &[u8], fx: &mut FileExtraction, scope: &str) {
    if !matches!(p.kind(), "required_parameter" | "optional_parameter") {
        return;
    }
    let Some(ty) = p.child_by_field_name("type") else {
        return;
    };
    let Some(ident) = p.child_by_field_name("pattern") else {
        return;
    };
    push_param_type(fx, text(ident, src), dotted_chain(ty, src), scope);
}

/// typed_parameter has NO 'name' field (positional identifier);
/// typed_default_parameter does. The annotation lives in the 'type' field,
/// wrapped in a 'type' node.
fn push_typed_param(p: Node<'_>, src: &[u8], fx: &mut FileExtraction, scope: &str) {
    if !matches!(p.kind(), "typed_parameter" | "typed_default_parameter") {
        return;
    }
    let ident = p.child_by_field_name("name").or_else(|| {
        let mut c = p.walk();
        let found = p.children(&mut c).find(|ch| ch.kind() == "identifier");
        found
    });
    let ty = p.child_by_field_name("type");
    let (Some(ident), Some(ty)) = (ident, ty) else {
        return;
    };
    push_param_type(fx, text(ident, src), dotted_chain(ty, src), scope);
}

/// Record one typed parameter when its annotation resolves to a dotted chain.
fn push_param_type(fx: &mut FileExtraction, var: String, chain: Option<Vec<String>>, scope: &str) {
    if let Some(ann_dotted) = chain {
        fx.params.push(ParamType {
            var,
            ann_dotted,
            scope_func: scope.to_string(),
        });
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
    let Some(target) = direct_call_target(right, src) else {
        // A simple `x = <unknown>` still overwrites an earlier constructor
        // type; record it as an unknown binding instead of leaving the old
        // assignment in place.
        if left.kind() == "identifier" {
            push_assignment(
                fx,
                text(left, src),
                Vec::new(),
                ctx,
                node.start_position().row + 1,
            );
        }
        return;
    };
    match left.kind() {
        "identifier" => push_assignment(
            fx,
            text(left, src),
            target,
            ctx,
            node.start_position().row + 1,
        ),
        "attribute" => record_self_field_assignment(left, target, ctx, fx, src),
        _ => {}
    }
}

/// `self.field = Ctor(...)` -> class-wide receiver typing.
fn record_self_field_assignment(
    left: Node<'_>,
    target: Vec<String>,
    ctx: &WalkCtx,
    fx: &mut FileExtraction,
    src: &[u8],
) {
    let Some(field) = left
        .child_by_field_name("object")
        .zip(left.child_by_field_name("attribute"))
        .and_then(|(obj, attr)| self_field_target(obj, attr, src))
    else {
        return;
    };
    let Some(class_qn) = ctx.class_stack.last().cloned() else {
        return;
    };
    fx.self_fields.push(SelfField {
        class_qn,
        field,
        target_dotted: target,
    });
}

/// `x = Ctor(...)` / `x = a or Ctor(...)`: the constructor chain, if any.
fn direct_call_target(right: Node<'_>, src: &[u8]) -> Option<Vec<String>> {
    let direct = |call: Node<'_>| -> Option<Vec<String>> {
        let f = call
            .child_by_field_name("function")
            .or_else(|| call.child_by_field_name("constructor"))?;
        dotted_chain(f, src)
    };
    if matches!(right.kind(), "call" | "call_expression" | "new_expression") {
        direct(right)
    } else if matches!(right.kind(), "boolean_operator" | "bool_op") {
        // `x = a or B(...)`: the last top-level operand that is a direct call.
        let mut cursor = right.walk();
        right
            .named_children(&mut cursor)
            .filter(|o| matches!(o.kind(), "call" | "call_expression"))
            .last()
            .and_then(direct)
    } else {
        None
    }
}

/// The field name when the assignment target is `self.<field>`.
fn self_field_target(obj: Node<'_>, attr: Node<'_>, src: &[u8]) -> Option<String> {
    (obj.kind() == "identifier" && text(obj, src) == "self").then(|| text(attr, src))
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
        push_assignment(fx, text(node, src), Vec::new(), ctx, site_line);
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
    push_assignment(
        fx,
        text(left, src),
        target.unwrap_or_default(),
        ctx,
        node.start_position().row + 1,
    );
}

fn record_call(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, ctx: &mut WalkCtx) {
    // call_expression uses field "function"; TS new_expression uses "constructor".
    let f = node
        .child_by_field_name("function")
        .or_else(|| node.child_by_field_name("constructor"));
    let Some(f) = f else { return };
    if try_ts_module_import(node, f, src, fx) {
        return;
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

/// TS/JS `require("...")` / `import("...")`: one static module edge. Returns
/// true when the call was fully handled as a module import.
fn try_ts_module_import(node: Node<'_>, f: Node<'_>, src: &[u8], fx: &mut FileExtraction) -> bool {
    if !matches!(fx.lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript) {
        return false;
    }
    let callee = text(f, src);
    if callee != "require" && callee != "import" {
        return false;
    }
    let Some(arg) = node
        .child_by_field_name("arguments")
        .and_then(|args| {
            let mut c = args.walk();
            let first = args.named_children(&mut c).next();
            first
        })
        .filter(|arg| arg.kind() == "string")
    else {
        return false;
    };
    typescript::push_ts_module_edge(fx, src, arg, node.start_position().row + 1);
    true
}

impl Walker<'_, '_, '_> {
    /// Shared CallableSpec constructor: start line and the
    /// base is-test flag all come from the walked node.
    fn callable_spec<'a>(
        &self,
        kind: SymbolKind,
        name: &'a str,
        qualname: &'a str,
    ) -> CallableSpec<'a> {
        CallableSpec {
            kind,
            name,
            qualname,
            start: self.node.start_position().row + 1,
            is_test: is_test_item(self.fx, self.ctx, name),
            always_push_scope: false,
        }
    }
}

fn define_function<'a>(w: &mut Walker<'a, '_, '_>, kind: SymbolKind) {
    let Some(name_node) = w.node.child_by_field_name("name") else {
        return;
    };
    let name = text(name_node, w.src);
    let qualname = parent_qualname(w.ctx, w.fx, &name);
    let mut spec = w.callable_spec(kind, &name, &qualname);
    if w.fx.lang == Lang::Rust && rust::has_test_attr(w.node, w.src) {
        spec.is_test = true;
    }
    w.push_callable(&spec);
    bind_default_export(w, &name);
    w.finish_callable(&spec);
}

fn parent_qualname(ctx: &WalkCtx, fx: &FileExtraction, name: &str) -> String {
    let parent = ctx
        .func_stack
        .last()
        .or_else(|| ctx.class_stack.last())
        .or_else(|| ctx.mod_stack.last())
        .cloned()
        .unwrap_or_else(|| fx.module.clone());
    format!("{parent}.{name}")
}

/// `export default function work()` has no `worker.default` symbol; bind
/// `default` to the real exported identity so a default import resolves to
/// `worker.work`.
fn bind_default_export<'a>(w: &mut Walker<'a, '_, '_>, name: &str) {
    if !matches!(w.fx.lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript) {
        return;
    }
    if !typescript::ts_default_export_ancestor(w.node) {
        return;
    }
    let module = w.fx.module.clone();
    w.fx.aliases.push((
        String::new(),
        "default".to_string(),
        ImportTarget::Symbol {
            module,
            name: name.to_string(),
        },
    ));
}
