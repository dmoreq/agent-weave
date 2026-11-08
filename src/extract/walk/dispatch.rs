//! AST node traversal dispatch, language visitors, and symbol definitions.

use super::*;

/// One pending traversal step. `Visit` dispatches a node; the restore steps
/// run AFTER every step pushed above them, so a scope push in a match arm can
/// be paired with its pop without recursion: the arm pushes `Pop*` first and
/// the body's children second, and the LIFO order replays the old
/// push/walk/pop sequence (R6d Blocker 1, iterative form).
pub(crate) enum Step<'a> {
    Visit(Node<'a>),
    PopFunc,
    PopClass,
    PopMod,
    RestoreTest(bool),
}

/// Queue `node`'s children for visiting in source order (reversed push).
pub(super) fn push_children_rev<'a>(work: &mut Vec<Step<'a>>, node: Node<'a>) {
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
pub(crate) fn walk_children(
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
        }
    }
}

/// Dispatch one node by language, then handle the kinds every language shares.
/// The per-language matches keep the original arm precedence: overlapping
/// kinds (`import_statement`, `mod_item`) are separated by language here.
fn visit_node<'a>(w: &mut Walker<'a, '_, '_>) {
    let handled = match w.fx.lang {
        Lang::Rust => rust::visit_rust_node(w),
        Lang::Python => python::visit_python_node(w),
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

/// Define a class-shaped symbol and collect its heritage. Shared by the TS
/// family (`class_declaration`, `interface_declaration`) and Rust
/// (`struct_item`/`enum_item`/`trait_item`): `typescript::collect_heritage`
/// walks both the TS clause kinds and the Rust `trait_bounds` kind.
pub(super) fn define_class_symbol<'a>(w: &mut Walker<'a, '_, '_>, kind: SymbolKind) {
    let Some(name_node) = w.node.child_by_field_name("name") else {
        return;
    };
    let name = text(name_node, w.src);
    let parent_qn = w
        .ctx
        .class_stack
        .last()
        .or_else(|| w.ctx.func_stack.last())
        .or_else(|| w.ctx.mod_stack.last())
        .cloned()
        .unwrap_or_else(|| w.fx.module.clone());
    let qualname = format!("{parent_qn}.{name}");
    let sym_idx = w.fx.symbols.len();
    w.fx.push_symbol(NewSymbol::from_node(
        w,
        kind,
        name.clone(),
        qualname.clone(),
        w.node,
        w.node,
        is_test_item(w.fx, w.ctx, &name),
    ));
    // Heritage: extends X / implements Y / trait bounds.
    collect_type_heritage(w.node, w.src, w.fx, sym_idx);
    if let Some(body) = w.node.child_by_field_name("body") {
        w.ctx.class_stack.push(qualname);
        w.work.push(Step::PopClass);
        push_children_rev(w.work, body);
    }
}

fn collect_type_heritage(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, sym_idx: usize) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_heritage" | "implements_clause" | "extends_type_clause" => {
                typescript::collect_heritage(child, src, fx, sym_idx);
            }
            _ => {}
        }
    }
    if let Some(bounds) = node.child_by_field_name("bounds") {
        typescript::collect_heritage(bounds, src, fx, sym_idx);
    }
}

fn visit_ts_node<'a>(w: &mut Walker<'a, '_, '_>) -> bool {
    match w.node.kind() {
        "class_declaration" | "abstract_class_declaration" => {
            define_class_symbol(w, SymbolKind::Class);
        }
        "function_declaration" | "generator_function_declaration" => {
            define_function(w, SymbolKind::Function);
        }
        "interface_declaration" => define_class_symbol(w, SymbolKind::Interface),
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
            let node = w.node;
            let scope = w.ctx.scope_func();
            collect_arrow_params(w, node, &scope, Some((start, end)));
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
pub(super) fn push_scope_symbol(
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
pub(super) fn is_test_item(fx: &FileExtraction, ctx: &WalkCtx, name: &str) -> bool {
    fx.is_test_file || ctx.test_scope || crate::extract::is_test_name(name)
}

fn visit_internal_module<'a>(w: &mut Walker<'a, '_, '_>) {
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
    for child in children.into_iter().rev() {
        if child.child_by_field_name("body").is_some() || child.kind() == "statement_block" {
            push_children_rev(w.work, child);
        }
    }
}

fn visit_call<'a>(w: &mut Walker<'a, '_, '_>) {
    record_call(w.node, w.src, w.fx, w.ctx);
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
        w.work.push(Step::RestoreTest(was_test_scope));
        push_children_rev(w.work, w.node);
    } else {
        push_children_rev(w.work, w.node);
    }
}

pub(super) fn record_call(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, ctx: &mut WalkCtx) {
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
            if fx.lang == Lang::Rust && f.kind() == "scoped_identifier" {
                match chain.first().map(String::as_str) {
                    Some("self") => chain[0] = "self::".to_string(),
                    Some("super") => chain[0] = "super::".to_string(),
                    _ => {}
                }
            }
            Callee::Attr(chain)
        }
        None => return,
    };
    fx.calls.push(CallSite {
        line: node.start_position().row + 1,
        scope_func: ctx.scope_func(),
        scope_class: ctx.class_stack.last().cloned(),
        scope_mod: ctx.mod_stack.last().cloned(),
        callee,
    });
}

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
        .and_then(|args| args.named_child(0))
        .filter(|arg| arg.kind() == "string")
    else {
        return false;
    };
    typescript::push_ts_module_edge(fx, src, arg, node.start_position().row + 1);
    true
}

pub(super) fn define_function<'a>(w: &mut Walker<'a, '_, '_>, kind: SymbolKind) {
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

pub(super) fn define_callable<'a>(w: &mut Walker<'a, '_, '_>, spec: &CallableSpec<'_>) {
    w.push_callable(spec);
    w.finish_callable(spec);
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

fn bind_default_export<'a>(w: &mut Walker<'a, '_, '_>, name: &str) {
    if !matches!(w.fx.lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript) {
        return;
    }
    if !typescript::ts_default_export_ancestor(w.node) {
        return;
    }
    let module = w.fx.module.clone();
    w.fx.bindings.aliases.push((
        String::new(),
        "default".to_string(),
        ImportTarget::Symbol {
            module,
            name: name.to_string(),
        },
    ));
}
