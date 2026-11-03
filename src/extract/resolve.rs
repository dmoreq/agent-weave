//! Phase B: global symbol and edge resolution over all extracted files.
//!
//! Kept separate from extraction so the resolution rules (module trees, alias
//! chains, receiver typing, MRO) can be read without the AST walk.

use std::collections::{HashMap, HashSet};

use super::*;

struct FileCtx {
    module: String,
    lang: Lang,
    /// Crate-root dir segment for Rust files (e.g. "src") — maps crate:: paths.
    crate_root: Option<String>,
    module_sym_idx: usize,
    aliases: HashMap<String, ImportTarget>,
    stars: Vec<(String, usize)>,
    from_import_parents: HashSet<(usize, String)>,
    unresolved_inherits: Vec<(String, usize)>,
    /// (scope_func, var) -> constructor target chain; later assignment wins.
    assignments: HashMap<(String, String), Vec<String>>,
    /// (scope_func, var) -> annotation chain.
    params: HashMap<(String, String), Vec<String>>,
    /// (class qualname, field) -> constructor target chain.
    self_fields: HashMap<(String, String), Vec<String>>,
    calls: Vec<CallSite>,
    import_edges: Vec<ImportEdgeCandidate>,
    /// (src idx global, base chain)
    inherits: Vec<(usize, Vec<String>)>,
    /// (src idx global, base chain, impl line) — edge-only, no class_bases.
    impl_inherits: Vec<(usize, Vec<String>, usize)>,
}

struct Tables {
    symbols: Vec<NewSymbol>,
    existing_ids: Vec<Option<i64>>,
    edges: Vec<NewEdge>,
    /// Direct module children only (Python bare-name visibility).
    by_mod_name: HashMap<(String, String), Vec<usize>>,
    by_qualname: HashMap<String, Vec<usize>>,
    class_methods: HashMap<String, HashMap<String, Vec<usize>>>,
    /// class qualname -> resolved base qualnames
    class_bases: HashMap<String, Vec<String>>,
    module_paths: HashSet<String>,
    file_ctx: Vec<FileCtx>,
}

enum Outcome {
    Extracted(usize),
    Ambiguous(String),
    Dropped,
    Unresolved,
}

pub fn resolve(mut files: Vec<FileExtraction>, carried: CarriedIndex) -> Resolution {
    let mut t = Tables {
        symbols: Vec::new(),
        existing_ids: Vec::new(),
        edges: Vec::new(),
        by_mod_name: HashMap::new(),
        by_qualname: HashMap::new(),
        class_methods: HashMap::new(),
        class_bases: HashMap::new(),
        module_paths: HashSet::new(),
        file_ctx: Vec::new(),
    };

    for fx in files.drain(..) {
        let base = t.symbols.len();
        let module = fx.module.clone();
        t.module_paths.insert(module.clone());
        for s in fx.symbols {
            let idx = t.symbols.len();
            let qualname = s.qualname.clone();
            let kind = s.kind;
            let name = s.name.clone();
            if kind == "module" {
                t.module_paths.insert(qualname.clone());
            }
            t.symbols.push(s);
            t.existing_ids.push(None);
            t.by_qualname.entry(qualname.clone()).or_default().push(idx);
            let direct_child = is_direct_child(&module, &name, &qualname);
            // Rust inline `mod` members are scoped as methods, but their
            // parent is a module; they are importable/callable bare names.
            // The parent is already in `by_qualname` (symbols are processed in
            // order), so its kind is one lookup instead of a reverse scan.
            let parent_qn_kind = qualname.rsplit_once('.').map(|(parent, _)| {
                let parent_kind = t
                    .by_qualname
                    .get(parent)
                    .and_then(|v| v.last())
                    .map(|i| t.symbols[*i].kind)
                    .unwrap_or("");
                (parent.to_string(), parent_kind)
            });
            match parent_qn_kind {
                // Any direct child of a module is a bare-name candidate, even
                // when Rust inline `mod` scopes it as kind "method".
                Some((parent, "module")) => {
                    t.by_mod_name
                        .entry((parent, name.clone()))
                        .or_default()
                        .push(idx);
                }
                _ if direct_child && kind != "method" => {
                    t.by_mod_name
                        .entry((module.clone(), name.clone()))
                        .or_default()
                        .push(idx);
                }
                _ => {}
            }
            if kind == "method" {
                let class_qn = qualname
                    .rsplit_once('.')
                    .map(|(p, _)| p.to_string())
                    .unwrap_or_default();
                t.class_methods
                    .entry(class_qn)
                    .or_default()
                    .entry(name)
                    .or_default()
                    .push(idx);
            }
            if kind == "class" {
                t.class_methods.entry(qualname).or_default();
            }
        }
        for mut e in fx.edges {
            e.src_idx += base;
            if let Some(d) = e.dst_idx.as_mut() {
                *d += base;
            }
            t.edges.push(e);
        }
        let inherits = fx
            .inherits
            .into_iter()
            .map(|(src, chain)| (src + base, chain))
            .collect();
        let impl_inherits = fx
            .impl_inherits
            .into_iter()
            .map(|(src, chain, line)| (src + base, chain, line))
            .collect();
        let mut aliases = HashMap::new();
        for (alias, target) in fx.aliases {
            aliases.insert(alias, target); // later rebinding wins
        }
        let mut assignments = HashMap::new();
        for a in fx.assignments {
            assignments.insert((a.scope_func, a.var), a.target_dotted);
        }
        let mut params = HashMap::new();
        for p in fx.params {
            params.insert((p.scope_func, p.var), p.ann_dotted);
        }
        let mut self_fields = HashMap::new();
        for sf in fx.self_fields {
            self_fields.insert((sf.class_qn, sf.field), sf.target_dotted);
        }
        let crate_root = fx.crate_root.clone();
        t.file_ctx.push(FileCtx {
            module,
            lang: fx.lang,
            crate_root,
            module_sym_idx: base,
            aliases,
            stars: fx.stars,
            from_import_parents: fx.from_import_parents,
            unresolved_inherits: fx.unresolved_inherits,
            assignments,
            params,
            self_fields,
            calls: fx.calls,
            import_edges: fx.import_edges,
            inherits,
            impl_inherits,
        });
    }

    // Merge carried symbols (fast path): join the resolution tables; their
    // rows are already stored and are never re-inserted.
    let CarriedIndex {
        symbols: carried_syms,
        inherits: carried_inherits,
    } = carried;
    for c in carried_syms {
        let idx = t.symbols.len();
        t.module_paths.insert(c.module.clone());
        if c.kind == "module" {
            t.module_paths.insert(c.qualname.clone());
        }
        t.symbols.push(NewSymbol {
            kind: c.kind,
            name: c.name.clone(),
            qualname: c.qualname.clone(),
            file_idx: usize::MAX,
            line_start: 0,
            line_end: 0,
            signature: String::new(),

            ..Default::default()
        });
        t.existing_ids.push(Some(c.row_id));
        t.by_qualname
            .entry(c.qualname.clone())
            .or_default()
            .push(idx);
        let direct_child = is_direct_child(&c.module, &c.name, &c.qualname);
        let parent_qn_kind = c.qualname.rsplit_once('.').map(|(parent, _)| {
            let parent_kind = t
                .by_qualname
                .get(parent)
                .and_then(|v| v.last())
                .map(|i| t.symbols[*i].kind)
                .unwrap_or("");
            (parent.to_string(), parent_kind)
        });
        match parent_qn_kind {
            Some((parent, "module")) => {
                t.by_mod_name
                    .entry((parent, c.name.clone()))
                    .or_default()
                    .push(idx);
            }
            _ if direct_child && c.kind != "method" => {
                t.by_mod_name
                    .entry((c.module.clone(), c.name.clone()))
                    .or_default()
                    .push(idx);
            }
            _ => {}
        }
        if c.kind == "method" {
            let class_qn = c
                .qualname
                .rsplit_once('.')
                .map(|(p, _)| p.to_string())
                .unwrap_or_default();
            t.class_methods
                .entry(class_qn)
                .or_default()
                .entry(c.name)
                .or_default()
                .push(idx);
        }
        if c.kind == "class" {
            t.class_methods.entry(c.qualname).or_default();
        }
    }
    for (sub_qn, base_qn) in carried_inherits {
        t.class_bases.entry(sub_qn).or_default().push(base_qn);
    }

    let mut unresolved_calls: Vec<UnresolvedSite> = Vec::new();
    for (fi, fctx) in t.file_ctx.iter().enumerate() {
        for (name, site_line) in &fctx.unresolved_inherits {
            unresolved_calls.push(UnresolvedSite {
                file_idx: fi,
                name: name.clone(),
                kind: "inherit",
                site_line: *site_line,
            });
        }
    }

    // Resolve fresh base chains now that alias tables exist; emit inherits
    // edges for fresh classes (carried inherits edges were carried as rows).
    for fctx in &t.file_ctx {
        for (src_idx, chain) in &fctx.inherits {
            let class_file = t.symbols[*src_idx].file_idx;
            let src_qn = t.symbols[*src_idx].qualname.clone();
            let site = t.symbols[*src_idx].line_start;
            let base_qn = resolve_base_chain(&t, class_file, chain, &src_qn);
            let dsts = base_qn
                .as_ref()
                .and_then(|qn| t.by_qualname.get(qn))
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let class_dsts: Vec<usize> = dsts
                .iter()
                .copied()
                .filter(|i| matches!(t.symbols[*i].kind, "class" | "trait" | "interface"))
                .collect();
            if class_dsts.len() > 1 {
                t.edges.push(NewEdge {
                    src_idx: *src_idx,
                    dst_idx: None,
                    dst_name: base_qn.clone().unwrap_or_else(|| chain.join(".")),
                    kind: "inherits",
                    confidence: "AMBIGUOUS",
                    site_line: site,
                });
                unresolved_calls.push(UnresolvedSite {
                    file_idx: class_file,
                    name: base_qn.unwrap_or_else(|| chain.join(".")),
                    kind: "inherit",
                    site_line: site,
                });
            } else if let Some(&dst) = class_dsts.first() {
                let qn = t.symbols[dst].qualname.clone();
                t.class_bases.entry(src_qn).or_default().push(qn.clone());
                t.edges.push(NewEdge {
                    src_idx: *src_idx,
                    dst_idx: Some(dst),
                    dst_name: qn,
                    kind: "inherits",
                    confidence: "EXTRACTED",
                    site_line: site,
                });
            } else {
                unresolved_calls.push(UnresolvedSite {
                    file_idx: class_file,
                    name: base_qn.unwrap_or_else(|| chain.join(".")),
                    kind: "inherit",
                    site_line: site,
                });
            }
        }
        for (src_idx, chain, line) in &fctx.impl_inherits {
            let class_file = t.symbols[*src_idx].file_idx;
            let src_qn = t.symbols[*src_idx].qualname.clone();
            let base_qn = resolve_base_chain(&t, class_file, chain, &src_qn);
            let dsts = base_qn
                .as_ref()
                .and_then(|qn| t.by_qualname.get(qn))
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let class_dsts: Vec<usize> = dsts
                .iter()
                .copied()
                .filter(|i| matches!(t.symbols[*i].kind, "class" | "trait" | "interface"))
                .collect();
            if class_dsts.len() > 1 {
                t.edges.push(NewEdge {
                    src_idx: *src_idx,
                    dst_idx: None,
                    dst_name: base_qn.clone().unwrap_or_else(|| chain.join(".")),
                    kind: "inherits",
                    confidence: "AMBIGUOUS",
                    site_line: *line,
                });
                unresolved_calls.push(UnresolvedSite {
                    file_idx: class_file,
                    name: base_qn.unwrap_or_else(|| chain.join(".")),
                    kind: "inherit",
                    site_line: *line,
                });
            } else if let Some(&dst) = class_dsts.first() {
                t.edges.push(NewEdge {
                    src_idx: *src_idx,
                    dst_idx: Some(dst),
                    dst_name: t.symbols[dst].qualname.clone(),
                    kind: "inherits",
                    confidence: "EXTRACTED",
                    site_line: *line,
                });
            } else {
                unresolved_calls.push(UnresolvedSite {
                    file_idx: class_file,
                    name: base_qn.unwrap_or_else(|| chain.join(".")),
                    kind: "inherit",
                    site_line: *line,
                });
            }
        }
    }

    let mut stats = ExtractStats::default();
    let mut unresolved_per_file = vec![0usize; t.file_ctx.len()];

    // Import edges.
    for (fi, fctx) in t.file_ctx.iter().enumerate() {
        for cand in &fctx.import_edges {
            match &cand.target {
                ImportTarget::Symbol { module, name } => {
                    if let Some(dsts) = t.by_mod_name.get(&(module.clone(), name.clone())) {
                        if dsts.len() > 1 {
                            t.edges.push(NewEdge {
                                src_idx: fctx.module_sym_idx,
                                dst_idx: None,
                                dst_name: format!("{module}.{name}"),
                                kind: "imports",
                                confidence: "AMBIGUOUS",
                                site_line: cand.line,
                            });
                        } else if let Some(&dst) = dsts.first() {
                            if dst == fctx.module_sym_idx {
                                continue;
                            }
                            t.edges.push(NewEdge {
                                src_idx: fctx.module_sym_idx,
                                dst_idx: Some(dst),
                                dst_name: format!("{module}.{name}"),
                                kind: "imports",
                                confidence: "EXTRACTED",
                                site_line: cand.line,
                            });
                        }
                    } else if let Some(dsts) = t.by_qualname.get(&format!("{module}.{name}")) {
                        // `use crate::a::b` where b is a submodule: the symbol
                        // candidate resolves to the module symbol.
                        if let Some(&dst) = dsts.iter().find(|&&i| t.symbols[i].kind == "module") {
                            if dst != fctx.module_sym_idx {
                                t.edges.push(NewEdge {
                                    src_idx: fctx.module_sym_idx,
                                    dst_idx: Some(dst),
                                    dst_name: format!("{module}.{name}"),
                                    kind: "imports",
                                    confidence: "EXTRACTED",
                                    site_line: cand.line,
                                });
                            }
                        } else {
                            unresolved_calls.push(UnresolvedSite {
                                file_idx: fi,
                                name: format!("{module}.{name}"),
                                kind: "import",
                                site_line: cand.line,
                            });
                        }
                    } else {
                        unresolved_calls.push(UnresolvedSite {
                            file_idx: fi,
                            name: format!("{module}.{name}"),
                            kind: "import",
                            site_line: cand.line,
                        });
                    }
                }
                ImportTarget::Module { path } => {
                    if fctx
                        .stars
                        .iter()
                        .any(|(m, line)| m == path && *line == cand.line)
                    {
                        continue; // the star edge below is the canonical one
                    }
                    if let Some(dsts) = t.by_qualname.get(path) {
                        let module = dsts
                            .iter()
                            .copied()
                            .find(|&i| t.symbols[i].kind == "module");
                        let fallback = dsts
                            .iter()
                            .copied()
                            .find(|&i| t.symbols[i].kind != "module");
                        if let Some(dst) = module.or(fallback) {
                            if dst != fctx.module_sym_idx {
                                t.edges.push(NewEdge {
                                    src_idx: fctx.module_sym_idx,
                                    dst_idx: Some(dst),
                                    dst_name: path.clone(),
                                    kind: "imports",
                                    confidence: "EXTRACTED",
                                    site_line: cand.line,
                                });
                            }
                        }
                    } else {
                        unresolved_calls.push(UnresolvedSite {
                            file_idx: fi,
                            name: path.clone(),
                            kind: "import",
                            site_line: cand.line,
                        });
                    }
                }
                ImportTarget::ModuleRoot { path } => {
                    // Star imports use ModuleRoot as their edge target: AMBIGUOUS
                    // edge to the module symbol (spec). Plain `import x` emits no
                    // edge; resolution happens at call time.
                    let is_star = fctx
                        .stars
                        .iter()
                        .any(|(m, line)| m == path && *line == cand.line);
                    if is_star {
                        let dst = t.by_qualname.get(path).and_then(|dsts| {
                            dsts.iter()
                                .copied()
                                .find(|&i| t.symbols[i].kind == "module")
                        });
                        match dst {
                            Some(dst) if dst != fctx.module_sym_idx => {
                                t.edges.push(NewEdge {
                                    src_idx: fctx.module_sym_idx,
                                    dst_idx: Some(dst),
                                    dst_name: path.clone(),
                                    kind: "imports",
                                    confidence: "AMBIGUOUS",
                                    site_line: cand.line,
                                });
                            }
                            None => unresolved_calls.push(UnresolvedSite {
                                file_idx: fi,
                                name: path.clone(),
                                kind: "import",
                                site_line: cand.line,
                            }),
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    // Deduplicate import edges and reconcile unresolved import candidates:
    // a statement that resolved by any candidate is not unresolved.
    {
        let mut seen_edges: HashSet<(usize, Option<usize>, String, usize)> = HashSet::new();
        t.edges.retain(|e| {
            e.kind != "imports"
                || seen_edges.insert((e.src_idx, e.dst_idx, e.dst_name.clone(), e.site_line))
        });
        let mut resolved: HashSet<(usize, usize, String)> = HashSet::new();
        for e in &t.edges {
            if e.kind == "imports" {
                let fi = t.symbols[e.src_idx].file_idx;
                resolved.insert((fi, e.site_line, e.dst_name.clone()));
            }
        }
        unresolved_calls.retain(|u| {
            u.kind != "import" || !resolved.contains(&(u.file_idx, u.site_line, u.name.clone()))
        });
        let mut seen_unresolved: HashSet<(usize, String, &'static str, usize)> = HashSet::new();
        unresolved_calls
            .retain(|u| seen_unresolved.insert((u.file_idx, u.name.clone(), u.kind, u.site_line)));
    }

    // Python `from pkg import mod`: when the imported name itself resolves as a
    // module, the child edge `pkg.mod` is canonical and the parent `pkg` edge is
    // redundant. Keep the parent when the same statement also imports a
    // non-module symbol from it (`from pkg import mod, func`).
    {
        let mut module_children: HashSet<(usize, usize, String)> = HashSet::new();
        let mut symbol_children: HashSet<(usize, usize, String)> = HashSet::new();
        for e in &t.edges {
            if e.kind != "imports" {
                continue;
            }
            let fi = t.symbols[e.src_idx].file_idx;
            if t.file_ctx.get(fi).is_none_or(|f| f.lang != Lang::Python) {
                continue;
            }
            let Some(dst) = e.dst_idx else { continue };
            let Some((parent, _)) = e.dst_name.rsplit_once('.') else {
                continue;
            };
            let key = (fi, e.site_line, parent.to_string());
            if t.symbols[dst].kind == "module" {
                module_children.insert(key);
            } else {
                symbol_children.insert(key);
            }
        }
        t.edges.retain(|e| {
            if e.kind != "imports" {
                return true;
            }
            let fi = t.symbols[e.src_idx].file_idx;
            if t.file_ctx.get(fi).is_none_or(|f| f.lang != Lang::Python) {
                return true;
            }
            let Some(dst) = e.dst_idx else { return true };
            if t.symbols[dst].kind != "module" {
                return true;
            }
            let fctx = &t.file_ctx[fi];
            let key = (fi, e.site_line, e.dst_name.clone());
            let parent_candidate = fctx
                .from_import_parents
                .contains(&(e.site_line, e.dst_name.clone()));
            !parent_candidate || !module_children.contains(&key) || symbol_children.contains(&key)
        });
    }

    // Call sites.
    for (fi, fctx) in t.file_ctx.iter().enumerate() {
        for site in &fctx.calls {
            let outcome = resolve_call(&t, fi, site);
            let src_idx = if site.scope_func.is_empty() {
                fctx.module_sym_idx
            } else {
                t.by_qualname
                    .get(&site.scope_func)
                    .and_then(|v| v.first().copied())
                    .unwrap_or(fctx.module_sym_idx)
            };
            match outcome {
                Outcome::Extracted(dst) => {
                    // R6d A6: keep the edge to the class itself so
                    // `callers Class` and `dead` are correct, and add a
                    // second edge to its constructor when unique.
                    let dst_name = t.symbols[dst].qualname.clone();
                    t.edges.push(NewEdge {
                        src_idx,
                        dst_idx: Some(dst),
                        dst_name: dst_name.clone(),
                        kind: "calls",
                        confidence: "EXTRACTED",
                        site_line: site.line,
                    });
                    if t.symbols[dst].kind == "class" {
                        if let Some(ctor) = ctor_target(&t, dst) {
                            if ctor != dst {
                                t.edges.push(NewEdge {
                                    src_idx,
                                    dst_idx: Some(ctor),
                                    dst_name: t.symbols[ctor].qualname.clone(),
                                    kind: "calls",
                                    confidence: "EXTRACTED",
                                    site_line: site.line,
                                });
                            }
                        }
                    }
                }
                Outcome::Ambiguous(name) => t.edges.push(NewEdge {
                    src_idx,
                    dst_idx: None,
                    dst_name: name,
                    kind: "calls",
                    confidence: "AMBIGUOUS",
                    site_line: site.line,
                }),
                Outcome::Dropped => stats.builtin_dropped += 1,
                Outcome::Unresolved => {
                    stats.unresolved_callees += 1;
                    unresolved_per_file[fi] += 1;
                    let name = match &site.callee {
                        Callee::Name(n) => n.clone(),
                        Callee::Attr(parts) => parts.join("."),
                    };
                    unresolved_calls.push(UnresolvedSite {
                        file_idx: fi,
                        name,
                        kind: "call",
                        site_line: site.line,
                    });
                }
            }
        }
    }

    Resolution {
        symbols: t.symbols,
        existing_ids: t.existing_ids,
        edges: t.edges,
        stats,
        unresolved_per_file,
        unresolved_calls,
    }
}

fn resolve_call(t: &Tables, fi: usize, site: &CallSite) -> Outcome {
    raw_resolve_call(t, fi, site)
}

/// Unique constructor of a class (`__init__` / `constructor`), if any.
/// Instantiations emit edges to BOTH the class and this constructor (R6d A6).
fn ctor_target(t: &Tables, class_idx: usize) -> Option<usize> {
    let class_qn = t.symbols[class_idx].qualname.clone();
    for ctor in ["__init__", "constructor"] {
        match t.class_methods.get(&class_qn).and_then(|m| m.get(ctor)) {
            Some(inits) if inits.len() == 1 => return Some(inits[0]),
            Some(_) => return None, // ambiguous ctor: class edge only
            None => continue,
        }
    }
    None
}

fn raw_resolve_call(t: &Tables, fi: usize, site: &CallSite) -> Outcome {
    let fctx = &t.file_ctx[fi];
    match &site.callee {
        Callee::Name(name) => {
            // 0. Lexical function scope: a nested def is visible to its
            // enclosing function and to deeper nested functions (closure).
            // Class scope is not lexical in Python, so stop before it.
            if !site.scope_func.is_empty() {
                let mut scope = site.scope_func.clone();
                while Some(scope.as_str()) != site.scope_class.as_deref() && scope != fctx.module {
                    let candidate = format!("{scope}.{name}");
                    if let Some(out) = bare_candidates(t.by_qualname.get(&candidate), name) {
                        return out;
                    }
                    let Some((parent, _)) = scope.rsplit_once('.') else {
                        break;
                    };
                    if Some(parent) == site.scope_class.as_deref() || parent == fctx.module {
                        break;
                    }
                    scope = parent.to_string();
                }
            }
            // 1. Local module definition.
            if let Some(out) = bare_candidates(
                t.by_mod_name.get(&(fctx.module.clone(), name.clone())),
                name,
            ) {
                return out;
            }
            // 2. Import alias map (follows re-export chains transitively).
            match fctx.aliases.get(name) {
                Some(ImportTarget::Symbol { module, name: n }) => {
                    let mut seen: HashSet<(String, String)> = HashSet::new();
                    if let Some(out) = follow_binding(t, module, n, 0, &mut seen, name) {
                        return out;
                    }
                    // Binding to a module that is not indexed at all = an
                    // external library (react, pydantic, os.path, ...): drop.
                    if !t.module_paths.contains(module) {
                        return Outcome::Dropped;
                    }
                }
                Some(ImportTarget::ModuleRoot { path }) => {
                    // A root-level `from .. import fn` binds a function, while a
                    // module alias is not callable; try the same-named child.
                    if let Some(out) =
                        bare_candidates(t.by_mod_name.get(&(path.clone(), name.clone())), name)
                    {
                        return out;
                    }
                }
                Some(ImportTarget::Module { .. }) => {
                    // Calling a bare module alias is invalid Python; fall through.
                }
                None => {}
            }
            // Star imports: defer to each star module's top-level names.
            for (m, _) in &fctx.stars {
                if let Some(out) =
                    bare_candidates(t.by_mod_name.get(&(m.clone(), name.clone())), name)
                {
                    return out;
                }
            }
            // 4. Builtin table.
            if BUILTINS.contains(&name.as_str()) {
                return Outcome::Dropped;
            }
            Outcome::Unresolved
        }
        Callee::Attr(chain) => {
            if chain.len() < 2 {
                return Outcome::Unresolved;
            }
            let method = chain[1].clone();
            // self/cls (Python) and this (TS) inside a class body.
            if chain[0] == "self" || chain[0] == "cls" || chain[0] == "this" {
                let Some(class_qn) = &site.scope_class else {
                    return Outcome::Unresolved;
                };
                if chain.len() == 2 {
                    return mro_outcome(t, class_qn, &method);
                }
                if chain.len() == 3 && chain[0] == "self" {
                    if let Some(field_chain) =
                        fctx.self_fields.get(&(class_qn.clone(), chain[1].clone()))
                    {
                        if let Some(field_class) = resolve_class_of_chain(t, fi, field_chain) {
                            return mro_outcome(t, &field_class, &chain[2]);
                        }
                    }
                }
                return Outcome::Unresolved;
            }
            // super.m(): skip the own class, resolve against its bases.
            if chain[0] == "super" && chain.len() == 2 {
                let Some(class_qn) = &site.scope_class else {
                    return Outcome::Unresolved;
                };
                if let Some(bases) = t.class_bases.get(class_qn) {
                    for base in bases {
                        let out = mro_outcome(t, base, &method);
                        if !matches!(out, Outcome::Unresolved) {
                            return out;
                        }
                    }
                }
                return Outcome::Unresolved;
            }
            // Receiver typing takes precedence over module interpretation
            // (Amendment 7b): a local binding shadows a same-named module.
            if chain.len() == 2 {
                if let Some(var_class) = resolve_var_class(t, fi, &chain[0], &site.scope_func) {
                    let out = mro_outcome(t, &var_class, &method);
                    if !matches!(out, Outcome::Unresolved) {
                        return out;
                    }
                }
            }
            // Class-attribute call: Class.m(...) or pkg.Class.m(...).
            // R1 fix: a module-bound first segment (import x as y / from pkg
            // import mod) must NOT enter the class branch — `mod.K()` is a
            // constructor through the module path, not a method lookup. And a
            // class-path miss falls through to the module path.
            if !chain_is_module_start(t, fi, &chain[0]) {
                if let Some(class_qn) = resolve_class_of_chain(t, fi, chain) {
                    if chain.len() == 2 {
                        let out = mro_outcome(t, &class_qn, &method);
                        if !matches!(out, Outcome::Unresolved) {
                            return out;
                        }
                    }
                }
                // Constructor receiver: `Base().m()` arrives as ["Base", "m"]
                // (Amendment 18). Resolve the single segment as a class, then
                // the method through its MRO.
                if chain.len() == 2 {
                    if let Some(class_qn) =
                        resolve_class_of_chain(t, fi, std::slice::from_ref(&chain[0]))
                    {
                        let out = mro_outcome(t, &class_qn, &method);
                        if !matches!(out, Outcome::Unresolved) {
                            return out;
                        }
                    }
                }
                // Receiver typed by constructor assignment or annotation.
                if chain.len() == 2 {
                    if let Some(var_class) = resolve_var_class(t, fi, &chain[0], &site.scope_func) {
                        return mro_outcome(t, &var_class, &method);
                    }
                }
            }
            // Rust path keywords. `Self::assoc()` resolves through the
            // enclosing impl target; `self::f()`/`super::f()` walk the module
            // tree from the current or parent module.
            if chain[0] == "Self" {
                if let Some(class_qn) = &site.scope_class {
                    if chain.len() == 2 {
                        return mro_outcome(t, class_qn, &chain[1]);
                    }
                }
                return Outcome::Unresolved;
            }
            if chain[0] == "self::" || chain[0] == "super::" {
                let mut module = fctx.module.clone();
                let mut rest: &[String] = &chain[1..];
                if chain[0] == "super::" {
                    loop {
                        let Some((parent, _)) = module.rsplit_once('.') else {
                            return Outcome::Unresolved;
                        };
                        module = parent.to_string();
                        if rest.first().map(String::as_str) == Some("super") {
                            rest = &rest[1..];
                            continue;
                        }
                        break;
                    }
                }
                return module_scoped_call(t, &module, rest);
            }
            // Rust crate::path::call — map crate:: to the crate root dir.
            if chain[0] == "crate" {
                if let Some(root) = &fctx.crate_root {
                    let out = module_chain_outcome(t, fi, &root_chain(root, chain));
                    if !matches!(out, Outcome::Unresolved) {
                        return out;
                    }
                    // Intermediate segments may be plain namespaces (a src/
                    // tree with no mod.rs): look the whole path up as a
                    // qualname under the crate root (Amendment 7a).
                    let full = format!("{}.{}", root, chain[1..].join("."));
                    if let Some(cands) = t.by_qualname.get(&full) {
                        return match cands.len() {
                            1 => Outcome::Extracted(cands[0]),
                            0 => Outcome::Unresolved,
                            _ => Outcome::Ambiguous(full),
                        };
                    }
                }
                return Outcome::Unresolved;
            }
            // Module-qualified call through the alias map / module tree.
            let out = module_chain_outcome(t, fi, chain);
            if !matches!(out, Outcome::Unresolved) {
                return out;
            }
            // Unknown Rust receiver: fall back to a same-file method name only
            // when it is unique. Ubiquitous trait/stdlib names never fall
            // back, and several matches stay unresolved instead of emitting
            // a noisy AMBIGUOUS fan-out across unrelated impls.
            if fctx.lang == Lang::Rust && chain.len() == 2 {
                return rust_unknown_receiver_outcome(t, fi, &chain[1]);
            }
            out
        }
    }
}

/// Conservatively resolve `receiver.method()` when the receiver type is
/// unknown: exactly one same-file method with that name wins. Common stdlib
/// and trait method names are excluded so they cannot manufacture edges.
fn rust_unknown_receiver_outcome(t: &Tables, fi: usize, method: &str) -> Outcome {
    const UBIQUITOUS: &[&str] = &[
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
    if UBIQUITOUS.contains(&method) {
        return Outcome::Unresolved;
    }
    let mut found: Option<usize> = None;
    for methods in t.class_methods.values() {
        let Some(idxs) = methods.get(method) else {
            continue;
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
    }
    found.map(Outcome::Extracted).unwrap_or(Outcome::Unresolved)
}

/// Module-qualified call from `self::`/`super::`. The parent directory may
/// not have its own module symbol (src/ layout without mod.rs), so a direct
/// qualname lookup is the fallback after the module tree walk.
fn module_scoped_call(t: &Tables, module: &str, rest: &[String]) -> Outcome {
    let out = chain_from_module(t, module.to_string(), rest);
    if !matches!(out, Outcome::Unresolved) {
        return out;
    }
    let full = format!("{module}.{}", rest.join("."));
    match t.by_qualname.get(&full) {
        Some(v) if v.len() == 1 => Outcome::Extracted(v[0]),
        Some(v) if v.len() > 1 => Outcome::Ambiguous(full),
        _ => Outcome::Unresolved,
    }
}

/// Replace a leading `crate` segment with the crate root dir segment.
fn root_chain(root: &str, chain: &[String]) -> Vec<String> {
    let mut out = vec![root.to_string()];
    out.extend_from_slice(&chain[1..]);
    out
}

/// Does the first chain segment bind to a module (alias or bare indexed name)?
fn chain_is_module_start(t: &Tables, fi: usize, first: &str) -> bool {
    let fctx = &t.file_ctx[fi];
    match fctx.aliases.get(first) {
        Some(ImportTarget::ModuleRoot { .. }) | Some(ImportTarget::Module { .. }) => true,
        Some(ImportTarget::Symbol { module, name }) => {
            t.module_paths.contains(&format!("{module}.{name}"))
        }
        None => t.module_paths.contains(first),
    }
}

/// Candidate outcome for a bare-name lookup across one namespace.
/// by_mod_name already holds only direct module children (function/class).
fn bare_candidates(cands: Option<&Vec<usize>>, display: &str) -> Option<Outcome> {
    let v = cands?;
    match v.len() {
        1 => Some(Outcome::Extracted(v[0])),
        0 => None,
        _ => Some(Outcome::Ambiguous(display.to_string())),
    }
}

/// Follow `from m import n` bindings transitively through re-export chains
/// (module m has no def of n but itself imports it from elsewhere), cycle-safe.
fn follow_binding(
    t: &Tables,
    module: &str,
    name: &str,
    depth: usize,
    seen: &mut HashSet<(String, String)>,
    display: &str,
) -> Option<Outcome> {
    if !seen.insert((module.to_string(), name.to_string())) {
        return None;
    }
    if let Some(out) = bare_candidates(
        t.by_mod_name.get(&(module.to_string(), name.to_string())),
        display,
    ) {
        return Some(out);
    }
    if depth >= 4 {
        return None;
    }
    let file_idx = t.file_ctx.iter().position(|f| f.module == module)?;
    let fctx = &t.file_ctx[file_idx];
    match fctx.aliases.get(name) {
        Some(ImportTarget::Symbol {
            module: m2,
            name: n2,
        }) => follow_binding(t, m2, n2, depth + 1, seen, display),
        _ => None,
    }
}

/// MRO walk, level by level: one match at the nearest level wins; several at
/// the same level are AMBIGUOUS; exhausted -> UNRESOLVED.
fn mro_outcome(t: &Tables, class_qn: &str, method: &str) -> Outcome {
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

/// Receiver typing: annotation first, then constructor assignment (exact
/// function scope, then module scope).
fn resolve_var_class(t: &Tables, fi: usize, var: &str, scope: &str) -> Option<String> {
    let fctx = &t.file_ctx[fi];
    if let Some(ann) = fctx.params.get(&(scope.to_string(), var.to_string())) {
        if let Some(qn) = resolve_class_of_chain(t, fi, ann) {
            return Some(qn);
        }
    }
    for s in [scope, ""] {
        if let Some(target) = fctx.assignments.get(&(s.to_string(), var.to_string())) {
            if let Some(qn) = resolve_class_of_chain(t, fi, target) {
                return Some(qn);
            }
        }
    }
    None
}

/// Resolve a dotted chain to a class qualname via: local module -> alias map ->
/// absolute qualname.
fn resolve_base_chain(
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
        if class_exists(t, &candidate) {
            return Some(candidate);
        }
        match parent.rsplit_once('.') {
            Some((p, _)) => parent = p.to_string(),
            None => break,
        }
    }
    resolve_class_of_chain(t, fi, chain)
}

fn resolve_class_of_chain(t: &Tables, fi: usize, chain: &[String]) -> Option<String> {
    if chain.is_empty() {
        return None;
    }
    let fctx = &t.file_ctx[fi];
    // `crate::Trait` in an impl target: map through the crate root first.
    if chain[0] == "crate" {
        if let Some(root) = &fctx.crate_root {
            let mapped = if root.is_empty() {
                chain[1..].to_vec()
            } else {
                root_chain(root, chain)
            };
            if !mapped.is_empty() {
                let qn = mapped.join(".");
                if class_exists(t, &qn) {
                    return Some(qn);
                }
            }
        }
    }
    // 1. Local module class (possibly nested: a.B.C).
    let local = format!("{}.{}", fctx.module, chain.join("."));
    if class_exists(t, &local) {
        return Some(local);
    }
    // 2. Alias map.
    if let Some(target) = fctx.aliases.get(&chain[0]) {
        match target {
            ImportTarget::Symbol { module, name } => {
                let qn = format!("{module}.{name}");
                // Direct hit first; then follow package re-export chains
                // (`from pkg import Base` where pkg/__init__ re-exports it).
                let mut base_qn: Option<String> = None;
                if class_exists(t, &qn) {
                    base_qn = Some(qn.clone());
                } else {
                    let mut seen: HashSet<(String, String)> = HashSet::new();
                    if let Some(Outcome::Extracted(idx)) =
                        follow_binding(t, module, name, 0, &mut seen, name)
                    {
                        if t.symbols[idx].kind == "class" {
                            base_qn = Some(t.symbols[idx].qualname.clone());
                        }
                    }
                }
                if let Some(bq) = base_qn {
                    if chain.len() == 1 {
                        return Some(bq);
                    }
                    let nested = format!("{}.{}", bq, chain[1..].join("."));
                    if class_exists(t, &nested) {
                        return Some(nested);
                    }
                }
            }
            ImportTarget::ModuleRoot { path } => {
                if let Some((module, rest)) = longest_module_prefix(t, path, &chain[1..]) {
                    if !rest.is_empty() {
                        let qn = format!("{}.{}", module, rest.join("."));
                        if class_exists(t, &qn) {
                            return Some(qn);
                        }
                    }
                }
            }
            ImportTarget::Module { .. } => {}
        }
    }
    // 3. Absolute qualname.
    let abs = chain.join(".");
    if class_exists(t, &abs) {
        return Some(abs);
    }
    None
}

fn class_exists(t: &Tables, qualname: &str) -> bool {
    t.by_qualname.get(qualname).is_some_and(|v| {
        v.iter()
            .any(|&i| matches!(t.symbols[i].kind, "class" | "trait" | "interface"))
    })
}

/// Walk the module tree: consume chain segments while the prefix names an
/// indexed module; return (module, remaining segments).
fn longest_module_prefix(
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
fn module_chain_outcome(t: &Tables, fi: usize, chain: &[String]) -> Outcome {
    let fctx = &t.file_ctx[fi];
    let start: Option<String> = match fctx.aliases.get(&chain[0]) {
        Some(ImportTarget::ModuleRoot { path }) => Some(path.clone()),
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

fn chain_from_module(t: &Tables, module: String, rest: &[String]) -> Outcome {
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
        if let Some(cands) = t.by_qualname.get(&full) {
            if cands.len() == 1 {
                return Outcome::Extracted(cands[0]);
            }
            if cands.len() > 1 {
                return Outcome::Ambiguous(full);
            }
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

#[cfg(test)]
mod gt_repro_tests {
    use super::*;
    use crate::parse::parse_source;
    use crate::parse::Lang;

    fn extract_src(src: &str) -> FileExtraction {
        let parsed = parse_source(src.as_bytes(), Lang::Python).unwrap();
        extract_file(0, "m.py", src.as_bytes(), &parsed)
    }

    #[test]
    fn typed_param_receiver_call_is_recorded() {
        let fx = extract_src("class A:\n    def f(self, h: Handler):\n        h.handle(1)\n");
        let calls: Vec<_> = fx
            .calls
            .iter()
            .map(|c| (&c.callee, &c.scope_func))
            .collect();
        assert_eq!(calls.len(), 1, "calls recorded: {:?}", calls.len());
        assert!(matches!(calls[0].0, Callee::Attr(_)));
        assert_eq!(fx.params.len(), 1, "params: {:?}", fx.params.len());
    }

    #[test]
    fn call_in_try_block_is_recorded() {
        let fx = extract_src(
            "def g():\n    try:\n        x = foo()\n    except ValueError:\n        pass\n",
        );
        assert_eq!(fx.calls.len(), 1, "calls: {}", fx.calls.len());
    }

    #[test]
    fn module_path_collapses_rust_and_ts_markers() {
        assert_eq!(module_path_for("src/lib.rs"), ("src".to_string(), true));
        assert_eq!(module_path_for("src/a/mod.rs"), ("src.a".to_string(), true));
        assert_eq!(module_path_for("src/a.rs"), ("src.a".to_string(), false));
        assert_eq!(module_path_for("lib.rs"), ("lib".to_string(), true));
        assert_eq!(module_path_for("pkg/index.mts"), ("pkg".to_string(), true));
        assert_eq!(
            module_path_for("pkg/mod.cjs"),
            ("pkg.mod".to_string(), false)
        );
    }

    #[test]
    fn rust_use_tree_emits_grouped_alias_self_and_super_candidates() {
        let src = "use crate::a::{f, S as T, self};\nuse super::util::u;\n";
        let parsed = parse_source(src.as_bytes(), Lang::Rust).unwrap();
        let fx = extract_file(0, "src/use_tree.rs", src.as_bytes(), &parsed);
        let mut edges: Vec<_> = fx
            .import_edges
            .iter()
            .map(|c| (c.line, format!("{:?}", c.target)))
            .collect();
        edges.sort();
        edges.dedup();
        assert_eq!(
            edges,
            vec![
                (1, "Module { path: \"src.a\" }".to_string()),
                (1, "Symbol { module: \"src.a\", name: \"S\" }".to_string()),
                (1, "Symbol { module: \"src.a\", name: \"f\" }".to_string()),
                (2, "Module { path: \"src.util\" }".to_string()),
                (
                    2,
                    "Symbol { module: \"src.util\", name: \"u\" }".to_string()
                ),
            ],
            "use-tree candidates"
        );
        assert!(fx.aliases.iter().any(|(a, _)| a == "T"), "alias T");
        assert!(fx.aliases.iter().any(|(a, _)| a == "u"), "alias u");
    }

    #[test]
    fn self_field_assignment_recorded() {
        let fx = extract_src("class C:\n    def __init__(self):\n        self.res = Resolver()\n");
        assert_eq!(
            fx.self_fields.len(),
            1,
            "self_fields: {}",
            fx.self_fields.len()
        );
    }
}

#[cfg(test)]
mod alias_repro {
    use super::*;
    use crate::parse::{parse_source, Lang};

    #[test]
    fn aliased_from_import_binds_and_emits_edge_candidate() {
        let src = "from ..knowledge.marshal_io import default_artifacts_dir as knowledge_artifacts_dir\n\ndef g():\n    return knowledge_artifacts_dir()\n";
        let parsed = parse_source(src.as_bytes(), Lang::Python).unwrap();
        let fx = extract_file(0, "vihap/validators/gso_loader.py", src.as_bytes(), &parsed);
        assert!(
            fx.aliases
                .iter()
                .any(|(a, _)| a == "knowledge_artifacts_dir"),
            "aliases: {:?}",
            fx.aliases
        );
        assert_eq!(
            fx.import_edges.len(),
            3,
            "import_edges: {} (module, child module, symbol)",
            fx.import_edges.len()
        );
        assert_eq!(fx.calls.len(), 1, "calls: {}", fx.calls.len());
    }
}
