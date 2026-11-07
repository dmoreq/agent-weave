//! TypeScript/TSX/JavaScript extraction: classes, interfaces, methods,
//! arrow functions, exports, imports, and module resolution.

use super::*;

pub(super) fn collect_heritage(
    node: Node<'_>,
    src: &[u8],
    fx: &mut FileExtraction,
    class_idx: usize,
) {
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
            | "argument_list" => push_clause_children(n, &mut stack),
            "call_expression" => record_mixin_base(n, src, fx, class_idx),
            "generic_type"
            | "type_identifier"
            | "identifier"
            | "member_expression"
            | "nested_type_identifier"
            | "scoped_type_identifier"
            | "attribute"
            | "subscript" => record_leaf_base(n, src, fx, class_idx),
            _ => {}
        }
    }
}

/// The clause arms: push every named child back onto the heritage stack.
fn push_clause_children<'a>(n: Node<'a>, stack: &mut Vec<Node<'a>>) {
    let mut c = n.walk();
    for child in n.named_children(&mut c) {
        stack.push(child);
    }
}

/// The `call_expression` arm: `extends mixin(Base, Other)` records the mixin
/// arguments as bases; a non-resolvable mixin name stays unresolved.
fn record_mixin_base(n: Node<'_>, src: &[u8], fx: &mut FileExtraction, class_idx: usize) {
    if let Some(name) = mixin_bases(n, src, fx, class_idx) {
        if matches!(fx.lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript) {
            fx.unresolved_inherits
                .push((name, n.start_position().row + 1));
        }
    }
}

/// The leaf arms: the node IS a base name — resolve its dotted chain.
fn record_leaf_base(n: Node<'_>, src: &[u8], fx: &mut FileExtraction, class_idx: usize) {
    if let Some(chain) = dotted_chain(n, src) {
        fx.inherits.push((class_idx, chain));
    }
}

/// `extends mixin(Base, Other)`: only the first identifier argument. Returns
/// the fallback name when no identifier argument resolved.
fn mixin_bases(
    node: Node<'_>,
    src: &[u8],
    fx: &mut FileExtraction,
    class_idx: usize,
) -> Option<String> {
    if let Some(args) = node.child_by_field_name("arguments") {
        let mut cursor = args.walk();
        for arg in args.named_children(&mut cursor) {
            if matches!(
                arg.kind(),
                "identifier" | "type_identifier" | "member_expression" | "scoped_type_identifier"
            ) {
                if let Some(chain) = dotted_chain(arg, src) {
                    fx.inherits.push((class_idx, chain));
                }
                return None;
            }
        }
    }
    Some(
        node.child_by_field_name("function")
            .map(|f| text(f, src))
            .unwrap_or_else(|| text(node, src)),
    )
}

pub(super) fn define_method_ts<'a>(w: &mut Walker<'a, '_, '_>) {
    // method_definition: name field is a property_identifier inside a class body.
    let Some(name_node) = w.node.child_by_field_name("name") else {
        return;
    };
    let Some(class_qn) = w.ctx.class_stack.last().cloned() else {
        return;
    };
    let name = text(name_node, w.src);
    let qualname = format!("{class_qn}.{name}");
    let spec = w.callable_spec(SymbolKind::Method, &name, &qualname);
    define_callable(w, &spec);
}

/// Walk the TS wrapper chain. `on_export` decides the answer at the first
/// `export_statement`; the chain only continues through wrapper nodes.
fn ts_ancestor(node: Node<'_>, on_export: impl Fn(Node<'_>) -> bool) -> bool {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if n.kind() == "export_statement" {
            return on_export(n);
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

/// True when a TS declaration is the target of `export default`.
pub(super) fn ts_default_export_ancestor(node: Node<'_>) -> bool {
    ts_ancestor(node, |n| {
        let mut cursor = n.walk();
        let is_default = n.children(&mut cursor).any(|c| c.kind() == "default");
        is_default
    })
}

/// Module-level `const X = () => {...}` / `= function() {...}` -> function
/// symbol (React components and hooks live here).
pub(super) fn record_ts_arrow_function<'a>(w: &mut Walker<'a, '_, '_>) -> bool {
    if !w.ctx.class_stack.is_empty() || !w.ctx.func_stack.is_empty() {
        return false; // nested functions are not indexed symbols
    }
    let Some(name_node) = w.node.child_by_field_name("name") else {
        return false;
    };
    let Some(value) = w.node.child_by_field_name("value") else {
        return false;
    };
    if !matches!(
        value.kind(),
        "arrow_function" | "function" | "function_expression"
    ) {
        return false;
    }
    let name = text(name_node, w.src);
    let qualname = format!("{}.{}", w.fx.module, name);
    w.fx.push_symbol(NewSymbol::from_node(
        w,
        SymbolKind::Function,
        name.clone(),
        qualname.clone(),
        w.node,
        value,
        is_test_item(w.fx, w.ctx, &name),
    ));
    w.ctx.func_stack.push(qualname.clone());
    collect_arrow_params(w, value, &qualname, None);
    w.work.push(Step::PopFunc);
    push_children_rev(w.work, value);
    true
}

/// TS `const value = new C()` receiver binding. A plain call
/// (`const value = factory()`) has no knowable constructed type and is ignored.
pub(super) fn record_ts_variable_binding(
    node: Node<'_>,
    src: &[u8],
    fx: &mut FileExtraction,
    ctx: &mut WalkCtx,
) {
    let Some(name) = node
        .child_by_field_name("name")
        .and_then(|n| pattern_name(n, src))
    else {
        return;
    };
    if let Some(ty) = node.child_by_field_name("type") {
        push_param_type(fx, name.clone(), dotted_chain(ty, src), &ctx.scope_func());
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
    push_assignment(fx, name, chain, ctx, node.start_position().row + 1);
}

/// TS/TSX/JSX static ESM imports. Relative sources resolve by module path;
/// bare/aliased sources bind as EXTERNAL (calls into them drop with the
/// builtin counter, not the unresolved counter).
pub(super) fn record_export_ts(node: Node<'_>, src: &[u8], fx: &mut FileExtraction) {
    if let Some(source_node) = node.child_by_field_name("source") {
        push_ts_module_edge(fx, src, source_node, node.start_position().row + 1);
    }
}

/// Push one module edge for a TS/JS import source node and return its path.
pub(super) fn push_ts_module_edge(
    fx: &mut FileExtraction,
    src: &[u8],
    node: Node<'_>,
    line: usize,
) -> String {
    let raw = text(node, src);
    let source = raw.trim_matches(|c| c == '\'' || c == '"');
    let module = resolve_ts_module(source, &fx.dir);
    fx.import_edges
        .push(ImportEdgeCandidate::module(line, module.clone()));
    module
}

pub(super) fn record_import_ts(node: Node<'_>, src: &[u8], fx: &mut FileExtraction) {
    let line = node.start_position().row + 1;
    let Some(source_node) = node.child_by_field_name("source") else {
        return;
    };
    let module = push_ts_module_edge(fx, src, source_node, line);
    let ctx = ImportCtx {
        module: &module,
        line,
    };
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_clause" {
            import_clause_bindings(child, src, ctx, fx);
        }
    }
}

#[derive(Clone, Copy)]
struct ImportCtx<'a> {
    module: &'a str,
    line: usize,
}

fn import_clause_bindings(
    clause: Node<'_>,
    src: &[u8],
    ctx: ImportCtx<'_>,
    fx: &mut FileExtraction,
) {
    let mut cursor = clause.walk();
    for part in clause.children(&mut cursor) {
        match part.kind() {
            "identifier" => default_import_binding(part, src, ctx, fx),
            "namespace_import" => namespace_import_binding(part, src, ctx, fx),
            "named_imports" => named_import_bindings(part, src, ctx, fx),
            _ => {}
        }
    }
}

/// default import: import X from "..."
fn default_import_binding(part: Node<'_>, src: &[u8], ctx: ImportCtx<'_>, fx: &mut FileExtraction) {
    let name = text(part, src);
    fx.bindings.aliases.push((
        String::new(),
        name,
        ImportTarget::Symbol {
            module: ctx.module.to_string(),
            name: "default".to_string(),
        },
    ));
}

/// * as ns — bind ns to the module itself
fn namespace_import_binding(
    part: Node<'_>,
    src: &[u8],
    ctx: ImportCtx<'_>,
    fx: &mut FileExtraction,
) {
    let mut cursor = part.walk();
    for child in part.children(&mut cursor) {
        if child.kind() == "identifier" {
            fx.bindings.aliases.push((
                String::new(),
                text(child, src),
                ImportTarget::ModuleRoot {
                    path: ctx.module.to_string(),
                },
            ));
        }
    }
}

fn named_import_bindings(part: Node<'_>, src: &[u8], ctx: ImportCtx<'_>, fx: &mut FileExtraction) {
    let mut cursor = part.walk();
    for spec in part.children(&mut cursor) {
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
        fx.bindings.aliases.push((
            String::new(),
            bound,
            ImportTarget::Symbol {
                module: ctx.module.to_string(),
                name: name.clone(),
            },
        ));
        fx.import_edges.push(ImportEdgeCandidate::symbol(
            ctx.line,
            ctx.module.to_string(),
            name,
        ));
    }
}

/// TS/JS source extensions in resolution order, derived from the master
/// extension table (`parse::SOURCE_EXTENSIONS`) so a new TS-family extension
/// needs no edit here. `.mjs`/`.cjs` import specifiers resolve to `.mts`/
/// `.cts` sources under the same stem.
fn ts_source_extensions() -> impl Iterator<Item = &'static str> {
    crate::parse::SOURCE_EXTENSIONS
        .iter()
        .filter(|(_, lang)| lang.is_ts_js())
        .map(|(ext, _)| *ext)
}

/// Resolve a TS import source to a module path, relative to the FILE'S
/// DIRECTORY (stems may contain dots, so the directory — not the module
/// string — is the only trustworthy base). Module identities collapse the
/// source extension and a trailing `index`, matching `module_path_for`, so
/// `./x`, `./x.ts`, `./x.mjs`, and `./x/index.ts` all resolve to the same
/// indexed module. Bare sources map to dotted external names.
pub(super) fn resolve_ts_module(source: &str, dir: &str) -> String {
    let stripped = ts_source_extensions()
        .find_map(|ext| source.strip_suffix(ext))
        .unwrap_or(source);
    let mut parts: Vec<&str> = if stripped.starts_with("./") || stripped.starts_with("../") {
        if dir.is_empty() {
            Vec::new()
        } else {
            dir.split('.').collect()
        }
    } else {
        Vec::new()
    };
    // One normalization pass: a `..` pops the directory base only while it
    // still leads the segment stream (`..` after the first non-`..` segment
    // is dropped without popping, and `.` is always dropped from the output).
    let mut leading = true;
    for seg in stripped.split('/').filter(|s| !s.is_empty()) {
        if seg == ".." && leading {
            parts.pop();
            continue;
        }
        leading = false;
        if seg != "." {
            parts.push(seg);
        }
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
