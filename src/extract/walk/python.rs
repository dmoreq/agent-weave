//! Python-specific extraction: import statements and their module targets.

use super::*;

pub(super) fn record_import(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, scope: &str) {
    let line = node.start_position().row + 1;
    match node.kind() {
        "import_statement" => record_plain_import(node, src, fx, scope, line),
        "import_from_statement" => record_from_import(node, src, fx, scope, line),
        _ => {}
    }
}

/// Push one `Module` import edge per dotted prefix (`a`, `a.b`, `a.b.c`).
fn push_module_prefix_edges(fx: &mut FileExtraction, line: usize, path: &str) {
    let parts: Vec<&str> = path.split('.').collect();
    for i in 1..=parts.len() {
        fx.import_edges
            .push(ImportEdgeCandidate::module(line, parts[..i].join(".")));
    }
}

/// The `name`/`alias` field pair of an aliased import node; `None` when the
/// grammar shape lacks either field.
fn aliased_import_pair(node: Node<'_>) -> Option<(Node<'_>, Node<'_>)> {
    Some((
        node.child_by_field_name("name")?,
        node.child_by_field_name("alias")?,
    ))
}

/// `import a.b`, `import a.b as c`: bind the root (or alias) and emit one
/// module edge per prefix.
fn record_plain_import(
    node: Node<'_>,
    src: &[u8],
    fx: &mut FileExtraction,
    scope: &str,
    line: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "dotted_name" => {
                let path = text(child, src);
                let root = path.split('.').next().unwrap_or(&path).to_string();
                fx.bindings.aliases.push((
                    scope.to_string(),
                    root.clone(),
                    ImportTarget::ModuleRoot { path: root },
                ));
                push_module_prefix_edges(fx, line, &path);
            }
            // `import a.b as c` wraps in aliased_import; a bare
            // "alias" node appears in some grammar versions.
            "aliased_import" | "alias" => {
                let Some((target, alias)) = aliased_import_pair(child) else {
                    continue;
                };
                let target_path = text(target, src);
                fx.bindings.aliases.push((
                    scope.to_string(),
                    text(alias, src),
                    ImportTarget::ModuleRoot {
                        path: target_path.clone(),
                    },
                ));
                push_module_prefix_edges(fx, line, &target_path);
            }
            _ => {}
        }
    }
}

/// `from <m> import ...`: emit the parent module edge plus one binding and
/// edge per imported name (wildcard, dotted, aliased).
fn record_from_import(
    node: Node<'_>,
    src: &[u8],
    fx: &mut FileExtraction,
    scope: &str,
    line: usize,
) {
    let module_node = node.child_by_field_name("module_name");
    let module_abs = module_node.map(|m| resolve_from_module(m, src, &fx.module, fx.is_package));
    let Some(m) = module_abs else { return };
    if !m.is_empty() {
        fx.import_edges
            .push(ImportEdgeCandidate::module(line, m.clone()));
        fx.bindings.from_import_parents.insert((line, m.clone()));
    }
    let ctx = FromImportCtx {
        scope,
        module: &m,
        line,
    };
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if Some(child) == module_node {
            continue;
        }
        match child.kind() {
            "wildcard_import" => {
                fx.bindings.stars.push((m.clone(), line));
                fx.import_edges
                    .push(ImportEdgeCandidate::module_root(line, m.clone()));
            }
            "dotted_name" => {
                let name = text(child, src);
                push_from_binding(fx, ctx, &name, None);
            }
            // `from m import n as y` wraps the binding in an
            // aliased_import node (import_statement uses "alias").
            "aliased_import" | "alias" => {
                let Some((target, alias)) = aliased_import_pair(child) else {
                    continue;
                };
                let name = text(target, src);
                push_from_binding(fx, ctx, &name, Some(text(alias, src)));
            }
            _ => {}
        }
    }
}

/// Stable context for every binding of one `from <m> import ...` statement,
/// mirroring the TS `ImportCtx` clump.
#[derive(Clone, Copy)]
struct FromImportCtx<'a> {
    scope: &'a str,
    module: &'a str,
    line: usize,
}

/// One imported name: bind `bound` (the alias text, or the same name when
/// `None`) to the module/symbol alias and emit the module edge plus (for a
/// non-empty parent) the symbol edge.
fn push_from_binding(
    fx: &mut FileExtraction,
    ctx: FromImportCtx<'_>,
    name: &str,
    bound: Option<String>,
) {
    // `from .. import X` at a first-level package has an empty parent: bind
    // the root child directly.
    let alias = if ctx.module.is_empty() {
        ImportTarget::ModuleRoot {
            path: name.to_string(),
        }
    } else {
        ImportTarget::Symbol {
            module: ctx.module.to_string(),
            name: name.to_string(),
        }
    };
    fx.bindings.aliases.push((
        ctx.scope.to_string(),
        bound.unwrap_or_else(|| name.to_string()),
        alias,
    ));
    fx.import_edges.push(ImportEdgeCandidate::module(
        ctx.line,
        if ctx.module.is_empty() {
            name.to_string()
        } else {
            format!("{}.{}", ctx.module, name)
        },
    ));
    if !ctx.module.is_empty() {
        fx.import_edges.push(ImportEdgeCandidate::symbol(
            ctx.line,
            ctx.module.to_string(),
            name.to_string(),
        ));
    }
}

/// `from <m> import ...`: resolve m to an absolute module path.
/// Relative levels: for a plain module file, level 1 = its package; for a
/// package file, level 1 = the package itself.
pub(super) fn resolve_from_module(
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

pub(super) fn visit_python_node<'a>(w: &mut Walker<'a, '_, '_>) -> bool {
    match w.node.kind() {
        "decorated_definition" => visit_decorated_definition(w),
        "function_definition" => visit_python_function(w),
        "class_definition" => visit_python_class(w),
        "import_statement" | "import_from_statement" => {
            record_import(w.node, w.src, w.fx, &w.ctx.scope_func());
        }
        "assignment" | "augmented_assignment" => {
            record_assignment(w);
            push_children_rev(w.work, w.node);
        }
        _ => return false,
    }
    true
}

fn visit_decorated_definition<'a>(w: &mut Walker<'a, '_, '_>) {
    let mut cursor = w.node.walk();
    let mut decorators = Vec::new();
    for child in w.node.children(&mut cursor) {
        if child.kind() == "decorator" {
            decorators.push(child);
        }
    }
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
    let parent_qn = innermost_scope(w.ctx, &w.fx.module);
    let qualname = format!("{parent_qn}.{name}");
    let sym_idx = w.fx.symbols.len();
    w.fx.push_symbol(NewSymbol::from_node(
        w,
        SymbolKind::Class,
        name.clone(),
        qualname.clone(),
        w.node,
        w.node,
        is_test_item(w.fx, w.ctx, &name),
    ));
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
