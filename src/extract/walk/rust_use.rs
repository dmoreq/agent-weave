//! Rust `use`-tree parsing: structural resolution of use declarations, lists,
//! grouped aliases, wildcard imports, and path prefixing.

use super::*;

#[derive(Clone, Copy)]
pub(super) struct UseTreeCtx<'a> {
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
pub(super) fn record_use_decl(node: Node<'_>, src: &[u8], fx: &mut FileExtraction, scope: &str) {
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

pub(super) fn walk_use_tree(node: Node<'_>, ctx: UseTreeCtx<'_>, fx: &mut FileExtraction) {
    match node.kind() {
        "use_list" => {
            let mut c = node.walk();
            for child in node.named_children(&mut c) {
                walk_use_tree(child, ctx, fx);
            }
        }
        "scoped_use_list" => emit_scoped_use_list(node, ctx, fx),
        "use_as_clause" => {
            let Some(path) = node.child_by_field_name("path") else {
                return;
            };
            let alias = node.child_by_field_name("alias").map(|n| text(n, ctx.src));
            emit_use_leaf(path, ctx, alias, fx);
        }
        "use_wildcard" => emit_use_wildcard(node, ctx, fx),
        _ => emit_use_leaf(node, ctx, None, fx),
    }
}

fn emit_scoped_use_list(node: Node<'_>, ctx: UseTreeCtx<'_>, fx: &mut FileExtraction) {
    let Some(path) = node.child_by_field_name("path") else {
        return;
    };
    let Some(base) = resolve_use_path(path, ctx) else {
        return;
    };
    if !base.is_empty() {
        fx.import_edges
            .push(ImportEdgeCandidate::module(ctx.line, base.join(".")));
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

fn emit_use_wildcard(node: Node<'_>, ctx: UseTreeCtx<'_>, fx: &mut FileExtraction) {
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
    fx.bindings.stars.push((qn.clone(), ctx.line));
    fx.import_edges
        .push(ImportEdgeCandidate::module_root(ctx.line, qn));
}

pub(super) fn emit_use_leaf(
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
    if emit_module_leaf(node, ctx, &full, &alias, fx) {
        return;
    }
    let Some((name, parent)) = full.split_last() else {
        return;
    };
    let module = parent.join(".");
    fx.import_edges
        .push(ImportEdgeCandidate::module(ctx.line, module.clone()));
    fx.import_edges.push(ImportEdgeCandidate::symbol(
        ctx.line,
        module.clone(),
        name.clone(),
    ));
    fx.bindings.aliases.push((
        ctx.scope.to_string(),
        alias.unwrap_or_else(|| name.clone()),
        ImportTarget::Symbol {
            module,
            name: name.clone(),
        },
    ));
}

/// A module-root leaf (`use a::b` where `b` is a module, or `use self`) emits a
/// module edge plus an optional binding. Returns false for a symbol leaf.
fn emit_module_leaf(
    node: Node<'_>,
    ctx: UseTreeCtx<'_>,
    full: &[String],
    alias: &Option<String>,
    fx: &mut FileExtraction,
) -> bool {
    let qn = full.join(".");
    let raw = text(node, ctx.src);
    let marker_only = raw == "self";
    let after_marker = raw.split("::").skip(1).filter(|p| !p.is_empty()).count();
    let module_leaf = marker_only || (ctx.prefix.is_empty() && after_marker <= 1);
    if !module_leaf {
        return false;
    }
    fx.import_edges
        .push(ImportEdgeCandidate::module(ctx.line, qn.clone()));
    let binding = alias.clone().or_else(|| {
        if node.kind() == "self" {
            None
        } else {
            full.last().cloned()
        }
    });
    if let Some(binding) = binding {
        if let Some((name, parent)) = full.split_last() {
            if parent.is_empty() {
                fx.bindings.aliases.push((
                    ctx.scope.to_string(),
                    binding,
                    ImportTarget::ModuleRoot { path: qn },
                ));
            } else {
                fx.bindings.aliases.push((
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
    true
}

/// Resolve one use-tree path node against the enclosing module scope.
/// `None` means the path is external/absolute and must not produce an edge.
pub(super) fn resolve_use_path(node: Node<'_>, ctx: UseTreeCtx<'_>) -> Option<Vec<String>> {
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
    let (mut out, plain) = first_segment_prefix(&first, extra_supers, ctx)?;
    if plain {
        out.push(first);
    }
    out.extend(rest.into_iter().skip(extra_supers));
    Some(out)
}

/// Resolve the first `::` segment against the crate root, scope, or prefix.
/// The flag is true when the segment is a plain name to append after it.
fn first_segment_prefix(
    first: &str,
    extra_supers: usize,
    ctx: UseTreeCtx<'_>,
) -> Option<(Vec<String>, bool)> {
    let (out, plain) = match first {
        "crate" => (
            ctx.crate_root?
                .split('.')
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect(),
            false,
        ),
        "self" if !ctx.prefix.is_empty() => (ctx.prefix.to_vec(), false),
        "self" => (ctx.scope.split('.').map(str::to_string).collect(), false),
        "super" if !ctx.prefix.is_empty() => {
            (pop_segments(ctx.prefix.to_vec(), extra_supers), false)
        }
        "super" => (
            pop_segments(
                ctx.scope.split('.').map(str::to_string).collect(),
                extra_supers,
            ),
            false,
        ),
        _ if ctx.prefix.is_empty() => return None,
        _ => (ctx.prefix.to_vec(), true),
    };
    Some((out, plain))
}

fn pop_segments(mut segments: Vec<String>, extra_supers: usize) -> Vec<String> {
    for _ in 0..=extra_supers {
        segments.pop();
    }
    segments
}
