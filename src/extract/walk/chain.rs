//! Identifier, dotted-chain, signature, and type-unwrapping helpers for AST walking.

use super::*;

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
            b'\n'
                if depth == 0
                    && !matches!(
                        prev_non_space,
                        b',' | b'(' | b'[' | b'{' | b'&' | b'|' | b'=' | b'>'
                    ) =>
            {
                break;
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

/// Split an identifier into space-separated search tokens (Amendment 12).
/// Lowercases everything, splits camelCase/PascalCase transitions and runs of
/// underscores, collapses runs, and strips non-alphanumeric noise.
pub fn split_ident(ident: &str) -> String {
    let mut out = String::with_capacity(ident.len() + 4);
    let mut chars = ident.chars().peekable();
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
        // Leaf kinds. Rust receiver and field access: `self.foo()` / `x.foo()`;
        // `self` is its own node kind in the Rust grammar, and
        // `field_identifier` is the accessed member.
        "identifier" | "this" | "super" | "crate" | "type_identifier" | "self"
        | "field_identifier" => Some(vec![text(node, src)]),
        // Python `a.b` (field `attribute`), TS `a.b` (field `property`), and
        // Rust `a.b` (fields `value`/`field`) share the same chain rule.
        "attribute" | "member_expression" | "field_expression" => member_chain(node, src),
        // Rust scoped type paths in impl/trait positions: crate::Shape,
        // std::fmt::Display. Treat them like scoped_identifier so a qualified
        // impl target still produces an edge or unresolved row.
        "scoped_type_identifier" | "scoped_identifier" => scoped_chain(node, src),
        // Annotation wrappers unwrap to the inner type's chain.
        "type" | "type_annotation" => unwrapped_chain(node, src),
        "generic_type" => generic_base(node, src),
        // Constructor-call receiver: `Base().m()` / `super().m()` /
        // `super(C, self).m()` (Amendment 18). Non-uppercase factory calls
        // are left unresolved to avoid manufacturing class edges.
        "call" => call_chain(node, src),
        _ => None,
    }
}

/// The member-access arm: one chain from the receiver plus the attribute
/// text. The two field names differ per grammar.
fn member_chain(node: Node<'_>, src: &[u8]) -> Option<Vec<String>> {
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

/// The annotation-wrapper arms: descend to the inner type. `type_annotation`
/// names its child with the `type` field; a plain `type` node has no such
/// field, so both fall back to the first named child.
fn unwrapped_chain(node: Node<'_>, src: &[u8]) -> Option<Vec<String>> {
    let inner = match node.child_by_field_name("type") {
        Some(ty) => Some(ty),
        None => node.named_child(0),
    };
    inner.and_then(|ch| dotted_chain(ch, src))
}

/// The constructor-call arm: only `super`-led or uppercase-first receivers
/// produce a chain.
fn call_chain(node: Node<'_>, src: &[u8]) -> Option<Vec<String>> {
    let func = node.child_by_field_name("function")?;
    let chain = dotted_chain(func, src)?;
    let first = chain.first()?;
    if first == "super" || first.chars().next().is_some_and(|c| c.is_uppercase()) {
        Some(chain)
    } else {
        None
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

/// Strip `&`, `&mut`, and raw-pointer wrappers from a Rust type node.
pub(super) fn unwrap_ref_type(mut node: Node<'_>) -> Node<'_> {
    while matches!(node.kind(), "reference_type" | "pointer_type") {
        match node.child_by_field_name("type") {
            Some(inner) => node = inner,
            None => break,
        }
    }
    node
}

/// Binding name of a parameter node: identifier, named pattern, or typed name.
pub(super) fn param_binding(node: Node<'_>, src: &[u8]) -> Option<String> {
    node.child_by_field_name("name")
        .or_else(|| node.child_by_field_name("pattern"))
        .and_then(|n| pattern_name(n, src))
        .or_else(|| pattern_name(node, src))
}

/// Binding name of a declarator pattern: `x`, `mut x`, `(a, b)` (first name).
pub(super) fn pattern_name(node: Node<'_>, src: &[u8]) -> Option<String> {
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
