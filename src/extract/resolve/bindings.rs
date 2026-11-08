//! Binding and class-chain resolution: alias following, MRO lookup,
//! variable/class chain typing, reexport chasing, and module-chain outcomes.

use super::*;

/// One `from m import n` binding under resolution: `(module, name)`. The same
/// pair doubles as the `seen` cycle-guard key, so the two values travel as one.
pub(super) type Binding<'a> = (&'a str, &'a str);

/// Follow `from m import n` bindings transitively through re-export chains
/// (module m has no def of n but itself imports it from elsewhere), cycle-safe.
pub(super) fn follow_binding<'a>(
    t: &'a Tables,
    binding: Binding<'a>,
    depth: usize,
    seen: &mut HashSet<Binding<'a>>,
    display: &str,
) -> Option<Outcome> {
    let (module, name) = binding;
    if !seen.insert((module, name)) {
        return None;
    }
    if let Some(out) = bare_candidates(
        t.by_mod_name
            .get(&(module.to_string(), name.to_string()))
            .map(|v| v.as_slice()),
        display,
    ) {
        return Some(out);
    }
    // Bounded re-export walk: a chain longer than 4 hops stays UNRESOLVED.
    // The cap is deliberate (bounded work per call site); `aw unresolved`
    // reports the resulting gaps rather than hiding them.
    if depth >= 4 {
        return None;
    }
    let file_idx = *t.file_by_module.get(module)?;
    let fctx = &t.file_ctx[file_idx];
    match file_alias(fctx, name) {
        Some(ImportTarget::Symbol {
            module: m2,
            name: n2,
        }) => follow_binding(t, (m2, n2), depth + 1, seen, display),
        _ => None,
    }
}

/// MRO walk, level by level: one match at the nearest level wins; several at
/// the same level are AMBIGUOUS; exhausted -> UNRESOLVED. Bounded at 6 levels
/// (deliberate work cap; deeper hierarchies stay UNRESOLVED and surface in
/// `aw unresolved`).
pub(super) fn mro_outcome(t: &Tables, class_qn: &str, method: &str) -> Outcome {
    let mut level = vec![class_qn.to_string()];
    let mut seen: HashSet<String> = HashSet::new();
    for _ in 0..6 {
        let mut next: Vec<String> = Vec::new();
        let mut found: Vec<usize> = Vec::new();
        for qn in &level {
            if !seen.insert(qn.clone()) {
                continue;
            }
            if let Some(idxs) = t.class_methods.get(qn).and_then(|m| m.get(method)) {
                found.extend_from_slice(idxs);
            } else if let Some(bases) = t.class_bases.get(qn) {
                next.extend(bases.iter().cloned());
            }
        }
        match found.len() {
            1 => return Outcome::Extracted(found[0]),
            0 if next.is_empty() => return Outcome::Unresolved,
            0 => level = next,
            // Two or more same-level definitions (e.g. diamond): AMBIGUOUS, as
            // documented — never silently drop the call site.
            _ => return Outcome::Ambiguous(format!("{class_qn}.{method}")),
        }
    }
    Outcome::Unresolved
}

/// Latest annotation or assignment at or before `site_line` in one scope.
/// `include_params` treats a parameter annotation as a binding at line 0.
pub(super) fn latest_binding<'a>(
    fctx: &'a FileCtx,
    scope: &str,
    var: &str,
    site_line: usize,
    include_params: bool,
) -> Option<&'a Vec<String>> {
    let mut best: Option<(usize, &'a Vec<String>)> = None;
    if include_params {
        if let Some(ann) = fctx
            .bindings
            .params
            .get(&(scope.to_string(), var.to_string()))
        {
            best = Some((0, ann));
        }
    }
    if let Some(events) = fctx
        .bindings
        .assignments
        .get(&(scope.to_string(), var.to_string()))
    {
        for (line, target) in events {
            if *line <= site_line && best.is_none_or(|(best_line, _)| *line >= best_line) {
                best = Some((*line, target));
            }
        }
    }
    best.map(|(_, target)| target)
}

/// Receiver typing at a call site: the latest annotation or assignment at or
/// before `site_line`. The exact function scope wins; module scope is the
/// fallback. A later assignment never changes an earlier call.
pub(super) fn resolve_var_class(
    t: &Tables,
    fi: usize,
    var: &str,
    scope: &str,
    site_line: usize,
) -> Option<String> {
    let fctx = &t.file_ctx[fi];
    if let Some(target) = latest_binding(fctx, scope, var, site_line, true) {
        // An unknown local assignment blocks the outer module binding; it
        // must not fall back to a stale class.
        if target.is_empty() {
            return None;
        }
        return resolve_class_of_chain(t, fi, target, scope);
    }
    if !scope.is_empty() {
        if let Some(target) = latest_binding(fctx, "", var, site_line, false) {
            if target.is_empty() {
                return None;
            }
            return resolve_class_of_chain(t, fi, target, scope);
        }
    }
    None
}

/// Resolve a dotted chain to a class qualname via: local module -> alias map ->
/// absolute qualname.
pub(super) fn resolve_base_chain(
    t: &Tables,
    fi: usize,
    chain: &[String],
    inheriting_qn: &str,
) -> Option<String> {
    let mut parent = inheriting_qn
        .rsplit_once('.')
        .map(|(p, _)| p.to_string())
        .unwrap_or_default();
    loop {
        let candidate = if parent.is_empty() {
            chain.join(".")
        } else {
            format!("{parent}.{}", chain.join("."))
        };
        if unique_class_exists(t, &candidate) {
            return Some(candidate);
        }
        match parent.rsplit_once('.') {
            Some((p, _)) => parent = p.to_string(),
            None => break,
        }
    }
    resolve_class_of_chain(t, fi, chain, "")
}

pub(super) fn resolve_class_of_chain(
    t: &Tables,
    fi: usize,
    chain: &[String],
    scope: &str,
) -> Option<String> {
    if chain.is_empty() {
        return None;
    }
    let fctx = &t.file_ctx[fi];
    // `crate::Trait` in an impl target: map through the crate root first.
    if chain[0] == "crate" {
        if let Some(qn) = crate_class(t, fctx.crate_root.as_deref(), chain) {
            return Some(qn);
        }
    }
    // 1. Local module class (possibly nested: a.B.C).
    let local = format!("{}.{}", fctx.module, chain.join("."));
    if unique_class_exists(t, &local) {
        return Some(local);
    }
    // 2. Alias map.
    if let Some(qn) = alias_class(t, fctx, chain, scope) {
        return Some(qn);
    }
    // 3. Absolute qualname.
    let abs = chain.join(".");
    unique_class_exists(t, &abs).then_some(abs)
}

pub(super) fn crate_class(t: &Tables, root: Option<&str>, chain: &[String]) -> Option<String> {
    let root = root?;
    let mapped = if root.is_empty() {
        chain[1..].to_vec()
    } else {
        root_chain(root, chain)
    };
    if mapped.is_empty() {
        return None;
    }
    let qn = mapped.join(".");
    unique_class_exists(t, &qn).then_some(qn)
}

pub(super) fn alias_class(
    t: &Tables,
    fctx: &FileCtx,
    chain: &[String],
    scope: &str,
) -> Option<String> {
    let target = nearest_alias_scoped(fctx, scope, &chain[0]).map(|(_, target)| target)?;
    match target {
        ImportTarget::Symbol { module, name } => {
            let qn = format!("{module}.{name}");
            // Direct hit first; then follow package re-export chains.
            let base_qn = if unique_class_exists(t, &qn) {
                Some(qn)
            } else {
                follow_reexport(t, module, name)
            }?;
            if chain.len() == 1 {
                return Some(base_qn);
            }
            let nested = format!("{}.{}", base_qn, chain[1..].join("."));
            unique_class_exists(t, &nested).then_some(nested)
        }
        ImportTarget::ModuleRoot { path } => {
            let (module, rest) = longest_module_prefix(t, path, &chain[1..])?;
            if rest.is_empty() {
                return None;
            }
            let qn = format!("{}.{}", module, rest.join("."));
            unique_class_exists(t, &qn).then_some(qn)
        }
        ImportTarget::Module { .. } => None,
    }
}

/// Follow package re-export chains (`from pkg import Base` where pkg/__init__
/// re-exports it) to the defining class qualname.
pub(super) fn follow_reexport(t: &Tables, module: &str, name: &str) -> Option<String> {
    let mut seen: HashSet<(&str, &str)> = HashSet::new();
    match follow_binding(t, (module, name), 0, &mut seen, name) {
        Some(Outcome::Extracted(idx)) if t.symbols[idx].kind == SymbolKind::Class => {
            Some(t.symbols[idx].qualname.to_string())
        }
        _ => None,
    }
}

pub(super) fn unique_class_exists(t: &Tables, qualname: &str) -> bool {
    unique_class_index(t, qualname).is_some()
}

/// The single class/trait/interface index for a qualname, when exactly one
/// exists. The `is_some()` of this function is also the existence test.
pub(super) fn unique_class_index(t: &Tables, qualname: &str) -> Option<usize> {
    t.by_qualname.get(qualname).and_then(|v| {
        let mut classes = v
            .iter()
            .copied()
            .filter(|&i| is_class_kind(t.symbols[i].kind));
        let first = classes.next()?;
        classes.next().is_none().then_some(first)
    })
}

pub(super) fn class_exists(t: &Tables, qualname: &str) -> bool {
    t.by_qualname
        .get(qualname)
        .is_some_and(|v| v.iter().any(|&i| is_class_kind(t.symbols[i].kind)))
}

/// Walk the module tree: consume chain segments while the prefix names an
/// indexed module; return (module, remaining segments).
pub(super) fn longest_module_prefix(
    t: &Tables,
    start: &str,
    rest: &[String],
) -> Option<(String, Vec<String>)> {
    if !t.module_paths.contains(start) {
        // PEP 420 namespace root (R6d I13): the package itself has no
        // __init__.py module symbol, but a deeper module may be indexed
        // (pkg -> pkg.mod). Resolve against the longest indexed prefix.
        let mut probe = start.to_string();
        let mut best: Option<(String, usize)> = None;
        for (i, seg) in rest.iter().enumerate() {
            probe = format!("{probe}.{seg}");
            if t.module_paths.contains(&probe) {
                best = Some((probe.clone(), i + 1));
            }
        }
        let (m, k) = best?;
        return Some((m, rest[k..].to_vec()));
    }
    let mut cur = start.to_string();
    let mut consumed = 0usize;
    for seg in rest {
        let next = format!("{cur}.{seg}");
        if t.module_paths.contains(&next) {
            cur = next;
            consumed += 1;
        } else {
            break;
        }
    }
    Some((cur, rest[consumed..].to_vec()))
}

/// `module.func(...)`, `pkg.mod.Class.m(...)` — resolve through the module tree.
pub(super) fn module_chain_outcome(
    t: &Tables,
    fi: usize,
    chain: &[String],
    scope: &str,
) -> Outcome {
    let fctx = &t.file_ctx[fi];
    let start: Option<String> =
        match nearest_alias_scoped(fctx, scope, &chain[0]).map(|(_, target)| target) {
            Some(ImportTarget::ModuleRoot { path }) => Some(path.to_string()),
            Some(ImportTarget::Module { .. }) => None,
            Some(ImportTarget::Symbol { module, name }) => {
                let qn = format!("{module}.{name}");
                if t.module_paths.contains(&qn) {
                    return chain_from_module(t, qn, &chain[1..]);
                }
                // Imported class: ImportedClass.method(...)
                if class_exists(t, &qn) && chain.len() == 2 {
                    return mro_outcome(t, &qn, &chain[1]);
                }
                return Outcome::Unresolved;
            }
            None => {
                if t.module_paths.contains(&chain[0]) {
                    Some(chain[0].clone())
                } else {
                    None
                }
            }
        };
    let Some(start_module) = start else {
        return Outcome::Unresolved;
    };
    chain_from_module(t, start_module, &chain[1..])
}

pub(super) fn chain_from_module(t: &Tables, module: String, rest: &[String]) -> Outcome {
    if rest.is_empty() {
        return Outcome::Unresolved; // calling a bare module
    }
    if let Some((m, r)) = longest_module_prefix(t, &module, rest) {
        if r.is_empty() {
            return Outcome::Unresolved;
        }
        if r.len() == 1 {
            if let Some(cands) = t.by_mod_name.get(&(m.clone(), r[0].clone())) {
                return match cands.len() {
                    1 => Outcome::Extracted(cands[0]),
                    0 => Outcome::Unresolved,
                    _ => Outcome::Ambiguous(r.join(".")),
                };
            }
        }
        // Dotted path into the module: module.func or module.Class.method.
        let full = format!("{}.{}", m, r.join("."));
        let out = qualname_outcome(t, &full);
        if !matches!(out, Outcome::Unresolved) {
            return out;
        }
        if r.len() >= 2 {
            let class_qn = format!("{}.{}", m, r[..r.len() - 1].join("."));
            if class_exists(t, &class_qn) {
                return mro_outcome(t, &class_qn, r.last().unwrap());
            }
        }
        if STDLIB_MODULES.contains(&m.split('.').next().unwrap_or(&m)) {
            return Outcome::Dropped;
        }
        return Outcome::Unresolved;
    }
    // Start module itself is external (import os; os.path.join(...)).
    if STDLIB_MODULES.contains(&module.split('.').next().unwrap_or(&module)) {
        return Outcome::Dropped;
    }
    Outcome::Unresolved
}
