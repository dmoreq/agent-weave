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

pub(super) use super::rust_use::*;

pub(super) fn visit_rust_node<'a>(w: &mut Walker<'a, '_, '_>) -> bool {
    match w.node.kind() {
        "function_item" | "function_signature_item" => visit_function_item(w),
        "struct_item" | "enum_item" => define_class_symbol(w, SymbolKind::Class),
        "trait_item" => define_class_symbol(w, SymbolKind::Trait),
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
            record_use_decl(w.node, w.src, w.fx, &scope);
            push_children_rev(w.work, w.node);
        }
        "let_declaration" => {
            record_rust_let(w.node, w.src, w.fx, w.ctx);
            push_children_rev(w.work, w.node);
        }
        "macro_invocation" => {
            record_macro_calls(w.node, w.src, w.fx, w.ctx);
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

fn visit_inline_mod<'a>(w: &mut Walker<'a, '_, '_>) {
    let name = w
        .node
        .child_by_field_name("name")
        .map(|n| text(n, w.src))
        .unwrap_or_default();
    let parent = w.ctx.class_stack.last().or_else(|| w.ctx.mod_stack.last());
    let qualname = match parent {
        Some(parent) => format!("{parent}.{name}"),
        None if w.fx.crate_root.as_deref() == Some("") => name.clone(),
        None => format!("{}.{}", w.fx.module, name),
    };
    let has_test_attr = has_test_attr(w.node, w.src);
    let item_is_test = is_test_item(w.fx, w.ctx, &name) || has_test_attr;
    push_scope_symbol(w.fx, w.node, name, &qualname, item_is_test);
    if let Some(body) = w.node.child_by_field_name("body") {
        w.ctx.mod_stack.push(qualname);
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
            w.fx.bindings.aliases.push((
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

fn visit_impl_item<'a>(w: &mut Walker<'a, '_, '_>) {
    let type_name = impl_type_name(w.node, w.src);
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
    w.ctx.class_stack.push(qualname);
    w.work.push(Step::PopClass);
    push_children_rev(w.work, w.node);
}
