//! TypeScript/TSX/JavaScript extraction: classes, interfaces, methods,
//! arrow functions, exports, imports, and module resolution.

use super::*;

pub(super) fn define_class_ts<'a>(
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
    if let Some(body) = node.child_by_field_name("body") {
        ctx.class_stack.push(qualname);
        work.push(Step::PopClass);
        push_children_rev(work, body);
    }
}

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

pub(super) fn define_method_ts<'a>(
    node: Node<'a>,
    src: &[u8],
    fx: &mut FileExtraction,
    ctx: &mut WalkCtx,
    work: &mut Vec<Step<'a>>,
) {
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
    let loc = callable_effective_loc(node, src, fx.lang, &fx.code_lines, &ctx.line_bounds);
    fx.symbols.push(NewSymbol {
        kind: "method",
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
    if let Some(body) = node.child_by_field_name("body") {
        ctx.func_stack.push(qualname);
        work.push(Step::PopFunc);
        push_children_rev(work, body);
    }
}

/// True when a TS declaration is the target of `export default`.
pub(super) fn ts_default_export_ancestor(node: Node<'_>) -> bool {
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
pub(super) fn ts_exported_ancestor(node: Node<'_>) -> bool {
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
pub(super) fn record_ts_arrow_function<'a>(
    node: Node<'a>,
    src: &[u8],
    fx: &mut FileExtraction,
    ctx: &mut WalkCtx,
    work: &mut Vec<Step<'a>>,
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
    let loc = callable_effective_loc(value, src, fx.lang, &fx.code_lines, &ctx.line_bounds);
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
    work.push(Step::PopFunc);
    push_children_rev(work, value);
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
pub(super) fn record_export_ts(node: Node<'_>, src: &[u8], fx: &mut FileExtraction) {
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

pub(super) fn record_import_ts(node: Node<'_>, src: &[u8], fx: &mut FileExtraction) {
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
pub(super) const TS_SOURCE_EXTENSIONS: &[&str] =
    &[".ts", ".tsx", ".js", ".jsx", ".mts", ".cts", ".mjs", ".cjs"];

/// Resolve a TS import source to a module path, relative to the FILE'S
/// DIRECTORY (stems may contain dots, so the directory — not the module
/// string — is the only trustworthy base). Module identities collapse the
/// source extension and a trailing `index`, matching `module_path_for`, so
/// `./x`, `./x.ts`, `./x.mjs`, and `./x/index.ts` all resolve to the same
/// indexed module. Bare sources map to dotted external names.
pub(super) fn resolve_ts_module(source: &str, dir: &str) -> String {
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
