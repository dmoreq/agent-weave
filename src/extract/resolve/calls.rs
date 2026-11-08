//! Call-site resolution: raw name and attribute callee lookup, lexical
//! scope and alias binding searches, and edge/unresolved-site emission.

use super::*;

/// Resolve every recorded call site into an edge or an unresolved site.
pub(super) fn resolve_call_sites(
    ctx: EmitCtx<'_>,
    unresolved_per_file: &mut [usize],
    stats: &mut ExtractStats,
) {
    // Call sites.
    for (fi, fctx) in ctx.t.file_ctx.iter().enumerate() {
        for site in &fctx.calls {
            let outcome = raw_resolve_call(ctx.t, fi, site);
            let src_idx = source_symbol_idx(ctx.t, fctx, site);
            // Type-only reference (Python): a typed receiver `x.m()` depends
            // on the class of `x`, not just on the method the call resolves
            // to. Emit a `type_ref` edge to that class regardless of the call
            // outcome, so the incremental dirty closure re-extracts this file
            // when the class's branch changes — otherwise a new MRO hit (a
            // method added on an ancestor) stays hidden behind a carried edge.
            if let Some(edge) = type_ref_edge(ctx.t, fi, site, src_idx) {
                ctx.t.edges.push(edge);
            }
            match outcome {
                Outcome::Extracted(dst) => {
                    let (dst_name, ctor) = extracted_edges(ctx.t, dst);
                    emit_extracted_call(&mut ctx.t.edges, src_idx, dst, dst_name, ctor, site.line);
                }
                Outcome::Ambiguous(name) => {
                    ctx.t.edges.push(ambiguous_edge(src_idx, name, site.line));
                }
                Outcome::Dropped => stats.builtin_dropped += 1,
                Outcome::Unresolved => {
                    stats.unresolved_callees += 1;
                    unresolved_per_file[fi] += 1;
                    ctx.unresolved_calls.push(UnresolvedSite {
                        file_idx: fi,
                        name: unresolved_site_name(site),
                        kind: EdgeSiteKind::Call,
                        site_line: site.line,
                    });
                }
            }
        }
    }
}

/// The `Outcome::Extracted` emit pair (R6d A6): keep the edge to the class
/// itself so `callers Class` and `dead` are correct, and add a second edge to
/// its constructor when unique. Like [`push_call_edge`], takes `edges`
/// separately from `Tables` because the caller iterates `t.file_ctx` while
/// pushing, which only borrows-split at field level.
pub(super) fn emit_extracted_call(
    edges: &mut Vec<NewEdge>,
    src_idx: usize,
    dst: usize,
    dst_name: String,
    ctor: Option<(usize, String)>,
    site_line: usize,
) {
    push_call_edge(edges, src_idx, Some(dst), dst_name, site_line);
    if let Some((ctor_idx, ctor_name)) = ctor {
        push_call_edge(edges, src_idx, Some(ctor_idx), ctor_name, site_line);
    }
}

/// Push the extracted-confidence call edge shared by the class edge and the
/// unique-constructor edge (R6d A6); the pair differs only in destination.
/// Takes `edges` separately from `Tables` because the caller iterates
/// `t.file_ctx` while pushing, which only borrows-split at field level.
pub(super) fn push_call_edge(
    edges: &mut Vec<NewEdge>,
    src_idx: usize,
    dst_idx: Option<usize>,
    dst_name: String,
    site_line: usize,
) {
    edges.push(NewEdge {
        src_idx,
        dst_idx,
        dst_name,
        kind: EdgeKind::Calls,
        confidence: Confidence::Extracted,
        site_line,
    });
}

pub(super) fn ambiguous_edge(src_idx: usize, name: String, site_line: usize) -> NewEdge {
    NewEdge {
        src_idx,
        dst_idx: None,
        dst_name: name,
        kind: EdgeKind::Calls,
        confidence: Confidence::Ambiguous,
        site_line,
    }
}

pub(super) fn source_symbol_idx(t: &Tables, fctx: &FileCtx, site: &CallSite) -> usize {
    if site.scope_func.is_empty() {
        return fctx.module_sym_idx;
    }
    t.by_qualname
        .get(&site.scope_func)
        .and_then(|v| v.first().copied())
        .unwrap_or(fctx.module_sym_idx)
}

pub(super) fn type_ref_edge(
    t: &Tables,
    fi: usize,
    site: &CallSite,
    src_idx: usize,
) -> Option<NewEdge> {
    let (class_qn, class_idx) = python_type_ref(t, fi, site)?;
    Some(NewEdge {
        src_idx,
        dst_idx: Some(class_idx),
        dst_name: class_qn,
        kind: EdgeKind::TypeRef,
        confidence: Confidence::Extracted,
        site_line: site.line,
    })
}

/// The class edge and, when the target is a class with a unique constructor,
/// the constructor edge. `None` when the class itself is the constructor.
pub(super) fn extracted_edges(t: &Tables, dst: usize) -> (String, Option<(usize, String)>) {
    let dst_name = t.symbols[dst].qualname.clone();
    let ctor = if t.symbols[dst].kind == SymbolKind::Class {
        ctor_target(t, dst)
            .filter(|c| *c != dst)
            .map(|c| (c, t.symbols[c].qualname.clone()))
    } else {
        None
    };
    (dst_name, ctor)
}

pub(super) fn unresolved_site_name(site: &CallSite) -> String {
    match &site.callee {
        Callee::Name(n) => n.clone(),
        Callee::Attr(parts) => parts.join("."),
    }
}

/// Unique constructor of a class (`__init__` / `constructor`), if any.
/// Instantiations emit edges to BOTH the class and this constructor (R6d A6).
pub(super) fn ctor_target(t: &Tables, class_idx: usize) -> Option<usize> {
    let class_qn = t.symbols[class_idx].qualname.to_string();
    for ctor in ["__init__", "constructor"] {
        match t.class_methods.get(&class_qn).and_then(|m| m.get(ctor)) {
            Some(inits) if inits.len() == 1 => return Some(inits[0]),
            Some(_) => return None, // ambiguous ctor: class edge only
            None => continue,
        }
    }
    None
}

/// Borrowed call-site context shared by the name/attribute resolvers.
pub(super) struct CallCtx<'a> {
    pub(super) t: &'a Tables,
    pub(super) fi: usize,
    pub(super) fctx: &'a FileCtx,
    pub(super) site: &'a CallSite,
}

pub(super) fn raw_resolve_call(t: &Tables, fi: usize, site: &CallSite) -> Outcome {
    let ctx = CallCtx {
        t,
        fi,
        fctx: &t.file_ctx[fi],
        site,
    };
    match &site.callee {
        Callee::Name(name) => resolve_name_callee(&ctx, name),
        Callee::Attr(chain) => resolve_attr_callee(&ctx, chain),
    }
}

pub(super) fn resolve_name_callee(ctx: &CallCtx<'_>, name: &str) -> Outcome {
    // 0. Lexical function scope: a nested def is visible to its
    // enclosing function and to deeper nested functions (closure).
    // Class scope is not lexical in Python, so stop before it.
    if let Some(out) = lexical_scope_lookup(ctx, name) {
        return out;
    }
    // 0b. Function-local import binding: a local `from a import f`
    // shadows the file-wide binding for this function.
    if let Some((alias_scope, target)) = nearest_alias_scoped(ctx.fctx, &ctx.site.scope_func, name)
    {
        if !alias_scope.is_empty() {
            return local_alias_outcome(ctx.t, target, name);
        }
    }
    // 0c. A parameter/local variable shadows module and file-level
    // names; it is not a static call target.
    if is_local_binding(ctx.fctx, ctx.site, name) {
        return Outcome::Unresolved;
    }
    // 1. Local module definition.
    if let Some(out) = bare_candidates(
        ctx.t
            .by_mod_name
            .get(&(ctx.fctx.module.to_string(), name.to_string())),
        name,
    ) {
        return out;
    }
    // 2. Import alias map (follows re-export chains transitively).
    if let Some(out) = alias_binding_lookup(ctx, name) {
        return out;
    }
    // Star imports: defer to each star module's top-level names.
    for (m, _) in &ctx.fctx.bindings.stars {
        if let Some(out) =
            bare_candidates(ctx.t.by_mod_name.get(&(m.clone(), name.to_string())), name)
        {
            return out;
        }
    }
    // 4. Builtin table.
    if BUILTINS.contains(&name) {
        return Outcome::Dropped;
    }
    Outcome::Unresolved
}

pub(super) fn lexical_scope_lookup(ctx: &CallCtx<'_>, name: &str) -> Option<Outcome> {
    if ctx.site.scope_func.is_empty() {
        return None;
    }
    let mut scope = ctx.site.scope_func.clone();
    while Some(scope.as_str()) != ctx.site.scope_class.as_deref() && scope != ctx.fctx.module {
        let candidate = format!("{scope}.{name}");
        if let Some(out) = bare_candidates(ctx.t.by_qualname.get(&candidate), name) {
            return Some(out);
        }
        let Some((parent, _)) = scope.rsplit_once('.') else {
            break;
        };
        if Some(parent) == ctx.site.scope_class.as_deref() || parent == ctx.fctx.module {
            break;
        }
        scope = parent.to_string();
    }
    None
}

pub(super) fn alias_binding_lookup(ctx: &CallCtx<'_>, name: &str) -> Option<Outcome> {
    alias_target_outcome(ctx.t, file_alias(ctx.fctx, name)?, name)
}

/// Shared alias resolution for the Symbol and ModuleRoot import targets:
/// follow the binding through re-exports, drop bindings to modules that are
/// not indexed, and try the same-named child for a root-level binding.
/// `None` leaves the fall-through value to the caller.
pub(super) fn alias_target_outcome(
    t: &Tables,
    target: &ImportTarget,
    name: &str,
) -> Option<Outcome> {
    match target {
        ImportTarget::Symbol { module, name: n } => {
            let mut seen: HashSet<(&str, &str)> = HashSet::new();
            if let Some(out) = follow_binding(t, (module, n), 0, &mut seen, name) {
                return Some(out);
            }
            // Binding to a module that is not indexed at all = an
            // external library (react, pydantic, os.path, ...): drop.
            if !t.module_paths.contains(module) {
                return Some(Outcome::Dropped);
            }
            None
        }
        ImportTarget::ModuleRoot { path } => bare_candidates(
            t.by_mod_name.get(&(path.to_string(), name.to_string())),
            name,
        ),
        ImportTarget::Module { .. } => None,
    }
}
