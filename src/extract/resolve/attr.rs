//! Attribute-chain callee resolution: receiver typing, class-path and
//! method lookup, Rust keyword paths, and ubiquitous-method fallbacks.

use super::*;

pub(super) fn resolve_attr_callee(ctx: &CallCtx<'_>, chain: &[String]) -> Outcome {
    if chain.len() < 2 {
        return Outcome::Unresolved;
    }
    let method = chain[1].clone();
    // self/cls (Python) and this (TS) inside a class body.
    if let Some(out) = resolve_receiver_method(ctx, chain, &method) {
        return out;
    }
    // Receiver typing takes precedence over module interpretation
    // (Amendment 7b): a local binding shadows a same-named module.
    if let Some(out) = receiver_typing(ctx, chain, &method) {
        return out;
    }
    // A local binding shadows a module even in longer attribute chains.
    if is_local_binding(ctx.fctx, ctx.site, &chain[0]) {
        return Outcome::Unresolved;
    }
    // Class-attribute call: Class.m(...) or pkg.Class.m(...).
    if let Some(out) = class_path_lookup(ctx, chain, &method) {
        return out;
    }
    // Rust path keywords. `Self::assoc()` resolves through the
    // enclosing impl target; `self::f()`/`super::f()` walk the module
    // tree from the current or parent module.
    if let Some(out) = resolve_rust_keyword_path(ctx, chain) {
        return out;
    }
    // Module-qualified call through the alias map / module tree.
    let out = module_chain_outcome(ctx.t, ctx.fi, chain, &ctx.site.scope_func);
    if !matches!(out, Outcome::Unresolved) {
        return out;
    }
    // Unknown Rust receiver: fall back to a same-file method name only
    // when it is unique. Ubiquitous trait/stdlib names never fall
    // back, and several matches stay unresolved instead of emitting
    // a noisy AMBIGUOUS fan-out across unrelated impls.
    if ctx.fctx.lang == Lang::Rust && chain.len() == 2 {
        return rust_unknown_receiver_outcome(ctx.t, ctx.fi, &chain[1]);
    }
    out
}

/// A typed local receiver shadows a same-named module even when the method
/// miss leaves the call unresolved.
pub(super) fn receiver_typing(
    ctx: &CallCtx<'_>,
    chain: &[String],
    method: &str,
) -> Option<Outcome> {
    if chain.len() != 2 {
        return None;
    }
    let var_class = resolve_var_class(
        ctx.t,
        ctx.fi,
        &chain[0],
        &ctx.site.scope_func,
        ctx.site.line,
    )?;
    Some(mro_outcome(ctx.t, &var_class, method))
}

pub(super) fn class_path_lookup(
    ctx: &CallCtx<'_>,
    chain: &[String],
    method: &str,
) -> Option<Outcome> {
    // R1 fix: a module-bound first segment (import x as y / from pkg
    // import mod) must NOT enter the class branch — `mod.K()` is a
    // constructor through the module path, not a method lookup. And a
    // class-path miss falls through to the module path.
    if chain_is_module_start(ctx.t, ctx.fi, &chain[0], &ctx.site.scope_func) {
        return None;
    }
    if let Some(class_qn) = resolve_class_of_chain(ctx.t, ctx.fi, chain, &ctx.site.scope_func) {
        if chain.len() == 2 {
            let out = mro_outcome(ctx.t, &class_qn, method);
            if !matches!(out, Outcome::Unresolved) {
                return Some(out);
            }
        }
    }
    // Constructor receiver: `Base().m()` arrives as ["Base", "m"]
    // (Amendment 18). Resolve the single segment as a class, then the
    // method through its MRO.
    if chain.len() == 2 {
        if let Some(class_qn) = resolve_class_of_chain(
            ctx.t,
            ctx.fi,
            std::slice::from_ref(&chain[0]),
            &ctx.site.scope_func,
        ) {
            let out = mro_outcome(ctx.t, &class_qn, method);
            if !matches!(out, Outcome::Unresolved) {
                return Some(out);
            }
        }
    }
    None
}

/// `self`/`cls`/`this` and `super` receivers. `None` means the chain is not
/// one of those receivers and the caller continues with class/module paths.
pub(super) fn resolve_receiver_method(
    ctx: &CallCtx<'_>,
    chain: &[String],
    method: &str,
) -> Option<Outcome> {
    if chain[0] == "self" || chain[0] == "cls" || chain[0] == "this" {
        // An explicit self/cls/this parameter of a nested function
        // shadows the owning method's implicit receiver.
        if is_local_binding(ctx.fctx, ctx.site, &chain[0]) {
            return Some(Outcome::Unresolved);
        }
        let Some(class_qn) = &ctx.site.scope_class else {
            return Some(Outcome::Unresolved);
        };
        if chain.len() == 2 {
            return Some(mro_outcome(ctx.t, class_qn, method));
        }
        if chain.len() == 3 && chain[0] == "self" {
            if let Some(field_chain) = ctx
                .fctx
                .bindings
                .self_fields
                .get(&(class_qn.clone(), chain[1].clone()))
            {
                if let Some(field_class) =
                    resolve_class_of_chain(ctx.t, ctx.fi, field_chain, &ctx.site.scope_func)
                {
                    return Some(mro_outcome(ctx.t, &field_class, &chain[2]));
                }
            }
        }
        return Some(Outcome::Unresolved);
    }
    // super.m(): skip the own class, resolve against its bases.
    if chain[0] == "super" && chain.len() == 2 {
        let Some(class_qn) = &ctx.site.scope_class else {
            return Some(Outcome::Unresolved);
        };
        if let Some(bases) = ctx.t.class_bases.get(class_qn) {
            for base in bases {
                let out = mro_outcome(ctx.t, base, method);
                if !matches!(out, Outcome::Unresolved) {
                    return Some(out);
                }
            }
        }
        return Some(Outcome::Unresolved);
    }
    None
}

/// Rust path keywords: `Self::`, `self::`, `super::`, and `crate::`. `None`
/// means the chain does not start with one of them.
pub(super) fn resolve_rust_keyword_path(ctx: &CallCtx<'_>, chain: &[String]) -> Option<Outcome> {
    if chain[0] == "Self" {
        if let Some(class_qn) = &ctx.site.scope_class {
            if chain.len() == 2 {
                return Some(mro_outcome(ctx.t, class_qn, &chain[1]));
            }
        }
        return Some(Outcome::Unresolved);
    }
    if chain[0] == "self::" || chain[0] == "super::" {
        let mut module = ctx
            .site
            .scope_mod
            .clone()
            .unwrap_or_else(|| ctx.fctx.module.to_string());
        let mut rest: &[String] = &chain[1..];
        if chain[0] == "super::" {
            loop {
                let Some((parent, _)) = module.rsplit_once('.') else {
                    return Some(Outcome::Unresolved);
                };
                module = parent.to_string();
                if rest.first().map(String::as_str) == Some("super") {
                    rest = &rest[1..];
                    continue;
                }
                break;
            }
        }
        return Some(module_scoped_call(ctx.t, &module, rest));
    }
    // Rust crate::path::call — map crate:: to the crate root dir.
    if chain[0] == "crate" {
        return Some(crate_keyword_path(ctx, chain));
    }
    None
}

/// Shared full-qualname lookup: exactly one match -> Extracted, several ->
/// Ambiguous, none -> Unresolved.
pub(super) fn qualname_outcome(t: &Tables, full: &str) -> Outcome {
    match t.by_qualname.get(full) {
        Some(cands) => match cands.len() {
            1 => Outcome::Extracted(cands[0]),
            0 => Outcome::Unresolved,
            _ => Outcome::Ambiguous(full.to_string()),
        },
        None => Outcome::Unresolved,
    }
}

pub(super) fn crate_keyword_path(ctx: &CallCtx<'_>, chain: &[String]) -> Outcome {
    let Some(root) = &ctx.fctx.crate_root else {
        return Outcome::Unresolved;
    };
    let out = module_chain_outcome(
        ctx.t,
        ctx.fi,
        &root_chain(root, chain),
        &ctx.site.scope_func,
    );
    if !matches!(out, Outcome::Unresolved) {
        return out;
    }
    // Intermediate segments may be plain namespaces (a src/ tree with no
    // mod.rs): look the whole path up as a qualname under the crate root
    // (Amendment 7a).
    let full = format!("{}.{}", root, chain[1..].join("."));
    qualname_outcome(ctx.t, &full)
}

/// Method names so common that a same-file match is not evidence of the
/// receiver's type.
pub(super) const UBIQUITOUS_METHODS: &[&str] = &[
    "new",
    "default",
    "clone",
    "into",
    "from",
    "iter",
    "len",
    "next",
    "fmt",
    "drop",
    "eq",
    "cmp",
    "to_string",
    "as_ref",
    "as_mut",
    "unwrap",
    "expect",
    "map",
    "and_then",
    "push",
    "pop",
    "get",
    "insert",
    "remove",
    "contains",
    "is_empty",
    "write",
    "read",
    "send",
    "recv",
    "poll",
    "main",
];

/// Conservatively resolve `receiver.method()` when the receiver type is
/// unknown: exactly one same-file method with that name wins. Common stdlib
/// and trait method names are excluded so they cannot manufacture edges.
pub(super) fn rust_unknown_receiver_outcome(t: &Tables, fi: usize, method: &str) -> Outcome {
    if UBIQUITOUS_METHODS.contains(&method) {
        return Outcome::Unresolved;
    }
    let mut found: Option<usize> = None;
    let Some(idxs) = t.methods_by_name.get(method) else {
        return Outcome::Unresolved;
    };
    for &idx in idxs {
        if t.symbols[idx].file_idx != fi {
            continue;
        }
        if found.is_some_and(|f| f != idx) {
            return Outcome::Unresolved;
        }
        found = Some(idx);
    }
    found.map(Outcome::Extracted).unwrap_or(Outcome::Unresolved)
}

/// Module-qualified call from `self::`/`super::`. The parent directory may
/// not have its own module symbol (src/ layout without mod.rs), so a direct
/// qualname lookup is the fallback after the module tree walk.
pub(super) fn module_scoped_call(t: &Tables, module: &str, rest: &[String]) -> Outcome {
    let out = chain_from_module(t, module.to_string(), rest);
    if !matches!(out, Outcome::Unresolved) {
        return out;
    }
    let full = format!("{module}.{}", rest.join("."));
    qualname_outcome(t, &full)
}

/// Replace a leading `crate` segment with the crate root dir segment.
pub(super) fn root_chain(root: &str, chain: &[String]) -> Vec<String> {
    let mut out = vec![root.to_string()];
    out.extend_from_slice(&chain[1..]);
    out
}

/// Does the first chain segment bind to a module (alias or bare indexed name)?
pub(super) fn chain_is_module_start(t: &Tables, fi: usize, first: &str, scope: &str) -> bool {
    let fctx = &t.file_ctx[fi];
    match nearest_alias_scoped(fctx, scope, first).map(|(_, target)| target) {
        Some(ImportTarget::ModuleRoot { .. }) | Some(ImportTarget::Module { .. }) => true,
        Some(ImportTarget::Symbol { module, name }) => {
            t.module_paths.contains(&format!("{module}.{name}"))
        }
        None => t.module_paths.contains(first),
    }
}

/// Candidate outcome for a bare-name lookup across one namespace.
/// by_mod_name already holds only direct module children (function/class).
pub(super) fn bare_candidates(cands: Option<&[usize]>, display: &str) -> Option<Outcome> {
    let v = cands?;
    match v.len() {
        1 => Some(Outcome::Extracted(v[0])),
        0 => None,
        _ => Some(Outcome::Ambiguous(display.to_string())),
    }
}

/// Nearest import binding for `name` at `scope`: an exact local scope (or a
/// nested function under it) beats a file-level binding.
pub(super) fn nearest_alias_scoped<'a>(
    fctx: &'a FileCtx,
    scope: &str,
    name: &str,
) -> Option<(&'a str, &'a ImportTarget)> {
    let entries = fctx.bindings.aliases.get(name)?;
    let mut best: Option<&'a (String, ImportTarget)> = None;
    for entry in entries {
        let binding_scope = entry.0.as_str();
        let visible = binding_scope.is_empty()
            || scope == binding_scope
            || scope.starts_with(&format!("{binding_scope}."));
        if visible && best.is_none_or(|b| binding_scope.len() >= b.0.len()) {
            best = Some(entry);
        }
    }
    best.map(|(binding_scope, target)| (binding_scope.as_str(), target))
}

pub(super) fn file_alias<'a>(fctx: &'a FileCtx, name: &str) -> Option<&'a ImportTarget> {
    fctx.bindings
        .aliases
        .get(name)?
        .iter()
        .rev()
        .find(|(scope, _)| scope.is_empty())
        .map(|(_, target)| target)
}

/// A parameter or local variable in the current or an enclosing function
/// shadows module and file-level bindings. Python class scope is not lexical.
pub(super) fn is_local_binding(fctx: &FileCtx, site: &CallSite, name: &str) -> bool {
    // Anonymous callback parameters shadow only inside their line range.
    if fctx.bindings.arrow_params.iter().any(|p| {
        (p.scope_func == site.scope_func
            || site.scope_func.starts_with(&format!("{}.", p.scope_func)))
            && p.var == name
            && p.start_line <= site.line
            && site.line <= p.end_line
    }) {
        return true;
    }
    let mut scope = site.scope_func.as_str();
    while !scope.is_empty() && scope != fctx.module {
        let key = (scope.to_string(), name.to_string());
        if fctx.bindings.locals.contains(&key) {
            return true;
        }
        // An assignment shadows only from its own line onward; a later
        // assignment must not erase an earlier valid bare call.
        if let Some(events) = fctx.bindings.assignments.get(&key) {
            if events.iter().any(|(line, _)| *line <= site.line) {
                return true;
            }
        }
        let Some((parent, _)) = scope.rsplit_once('.') else {
            break;
        };
        if Some(parent) == site.scope_class.as_deref() {
            break;
        }
        scope = parent;
    }
    false
}

pub(super) fn local_alias_outcome(t: &Tables, target: &ImportTarget, name: &str) -> Outcome {
    alias_target_outcome(t, target, name).unwrap_or(Outcome::Unresolved)
}
