//! Rust-specific extraction: let-declaration locals, constructor receivers,
//! test attributes, macro-call sites, and `use`-tree imports.

use super::*;

/// Rust local receiver typing: `let x: Foo` (annotation) and
/// `let x = Foo::new()` / `let x = Foo { .. }` (construction). Only a direct
/// constructor-shaped value types the binding; factory calls stay unknown.
pub(super) fn record_rust_let(
    node: Node<'_>,
    src: &[u8],
    fx: &mut FileExtraction,
    ctx: &mut WalkCtx,
) {
    let Some(pattern) = node.child_by_field_name("pattern") else {
        return;
    };
    let Some(var) = pattern_name(pattern, src) else {
        return;
    };
    if let Some(ty) = node.child_by_field_name("type") {
        push_param_type(
            fx,
            var,
            dotted_chain(unwrap_ref_type(ty), src),
            &ctx.scope_func(),
        );
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
    push_assignment(fx, var, chain, ctx, node.start_position().row + 1);
}

/// Constructor-shaped Rust value: `Foo::new()`, `Self::new()`, or a struct
/// literal. An arbitrary associated function (`A::make() -> B`) says nothing
/// about the returned type and must not type `A`.
pub(super) fn rust_constructor_target(
    value: Node<'_>,
    src: &[u8],
    ctx: &WalkCtx,
) -> Option<Vec<String>> {
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

/// Rust `#[test]` / `#[cfg(test)]` on the item itself: attributes attach as
/// preceding `attribute_item` siblings, so scan backwards past comments.
pub(super) fn has_test_attr(node: Node<'_>, src: &[u8]) -> bool {
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
pub(super) const MACRO_CALL_FAMILIES: &[&str] = &[
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
];

pub(super) const MAX_MACRO_TOKEN_TEXT: usize = 64 * 1024;

pub(super) const MAX_MACRO_CALLS_PER_SITE: usize = 128;

pub(super) fn record_macro_calls(
    node: Node<'_>,
    src: &[u8],
    fx: &mut FileExtraction,
    ctx: &mut WalkCtx,
) {
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

/// Extend `name` across the `::`-separated path segments starting at `i`
/// (`a::b::c`); returns the extended name and the new cursor. A bare `::`
/// with no following identifier stops the walk.
fn scan_path_suffix(text: &str, bytes: &[u8], mut name: String, mut i: usize) -> (String, usize) {
    let n = bytes.len();
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
    (name, i)
}

/// Byte scanner over macro token text: captures identifier and `::`-path
/// calls, skips string/char literals, and ignores method-call receivers
/// (`x.f(`) whose receiver expression a text scan cannot reconstruct.
pub(super) fn scan_macro_call_names(raw: &str) -> Vec<String> {
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
                // `a::b::c(` — walk the path to its final segment.
                let (name, next) = scan_path_suffix(text, bytes, text[start..i].to_string(), i);
                i = next;
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

pub(super) fn is_ident_start(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphabetic()
}

pub(super) fn is_ident_char(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphanumeric()
}

pub(super) fn skip_quoted(bytes: &[u8], start: usize, quote: u8) -> usize {
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
pub(super) fn skip_char_or_lifetime(bytes: &[u8], start: usize) -> usize {
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

pub(super) fn impl_type_name(node: Node<'_>, src: &[u8]) -> String {
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
