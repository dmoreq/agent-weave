//! Phase B: global symbol and edge resolution over all extracted files.
//!
//! Kept separate from extraction so the resolution rules (module trees, alias
//! chains, receiver typing, MRO) can be read without the AST walk.

use std::collections::{HashMap, HashSet};

use super::*;

/// (scope_func, var) -> ordered (site_line, constructor target chain).
type BindingEvents = HashMap<(String, String), Vec<(usize, Vec<String>)>>;

struct FileCtx {
    module: String,
    lang: Lang,
    /// Crate-root dir segment for Rust files (e.g. "src") — maps crate:: paths.
    crate_root: Option<String>,
    module_sym_idx: usize,
    aliases: HashMap<String, Vec<(String, ImportTarget)>>,
    /// (scope_func, var) parameters/locals that shadow outer names.
    locals: HashSet<(String, String)>,
    /// Anonymous callback parameters, scoped by source line range.
    arrow_params: Vec<ArrowParam>,
    stars: Vec<(String, usize)>,
    from_import_parents: HashSet<(usize, String)>,
    unresolved_inherits: Vec<(String, usize)>,
    /// (scope_func, var) -> ordered (site_line, constructor target chain).
    assignments: BindingEvents,
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
    /// Every registered method index keyed by bare name, so the Rust
    /// unknown-receiver fallback is one lookup instead of a scan over all
    /// classes (`class_methods` keeps the per-class view for MRO walks).
    methods_by_name: HashMap<String, Vec<usize>>,
    /// class qualname -> resolved base qualnames
    class_bases: HashMap<String, Vec<String>>,
    module_paths: HashSet<String>,
    file_ctx: Vec<FileCtx>,
    /// module -> file_ctx index (first fresh file wins, matching the linear
    /// search it replaced), for the re-export binding walk.
    file_by_module: HashMap<String, usize>,
}

enum Outcome {
    Extracted(usize),
    Ambiguous(String),
    Dropped,
    Unresolved,
}

/// Register one symbol in every resolution table. Shared by the fresh and
/// carried merge paths; `existing_id` is `Some(row_id)` for a carried row and
/// `None` for a freshly extracted one.
fn register_symbol(t: &mut Tables, sym: NewSymbol, module: &str, existing_id: Option<i64>) {
    let idx = t.symbols.len();
    let qualname = sym.qualname.clone();
    let kind = sym.kind;
    let name = sym.name.clone();
    if kind == SymbolKind::Module {
        t.module_paths.insert(qualname.clone());
    }
    t.symbols.push(sym);
    t.existing_ids.push(existing_id);
    t.by_qualname.entry(qualname.clone()).or_default().push(idx);
    index_parent_module(t, idx, module, &name, &qualname, kind);
    index_method_class(t, idx, &name, &qualname, kind);
}

/// Index one symbol under its parent module (Python bare-name visibility).
/// Rust inline `mod` members are scoped as methods, but their parent is a
/// module; they are importable/callable bare names. The parent is already
/// in `by_qualname` (symbols are processed in order), so its kind is one
/// lookup instead of a reverse scan.
fn index_parent_module(
    t: &mut Tables,
    idx: usize,
    module: &str,
    name: &str,
    qualname: &str,
    kind: SymbolKind,
) {
    let direct_child = is_direct_child(module, name, qualname);
    let parent_qn_kind = qualname.rsplit_once('.').map(|(parent, _)| {
        let parent_kind = t
            .by_qualname
            .get(parent)
            .and_then(|v| v.last())
            .map(|i| t.symbols[*i].kind);
        (parent.to_string(), parent_kind)
    });
    match parent_qn_kind {
        // Any direct child of a module is a bare-name candidate, even when
        // Rust inline `mod` scopes it as a method.
        Some((parent, Some(SymbolKind::Module))) => {
            t.by_mod_name
                .entry((parent, name.to_string()))
                .or_default()
                .push(idx);
        }
        _ if direct_child && kind != SymbolKind::Method => {
            t.by_mod_name
                .entry((module.to_string(), name.to_string()))
                .or_default()
                .push(idx);
        }
        _ => {}
    }
}

/// Index one method under its bare name and its class (the MRO walks), and
/// make sure every class qualname owns a method map.
fn index_method_class(t: &mut Tables, idx: usize, name: &str, qualname: &str, kind: SymbolKind) {
    if kind == SymbolKind::Method {
        let class_qn = qualname
            .rsplit_once('.')
            .map(|(p, _)| p.to_string())
            .unwrap_or_default();
        t.methods_by_name
            .entry(name.to_string())
            .or_default()
            .push(idx);
        t.class_methods
            .entry(class_qn)
            .or_default()
            .entry(name.to_string())
            .or_default()
            .push(idx);
    }
    if kind == SymbolKind::Class {
        t.class_methods.entry(qualname.to_string()).or_default();
    }
}

/// The kinds that can carry an inheritance edge: classes, traits, and
/// interfaces share the base/MRO machinery.
fn is_class_kind(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Class | SymbolKind::Trait | SymbolKind::Interface
    )
}

/// Emit the AMBIGUOUS inherits edge (no single destination) together with its
/// unresolved site. One function so the edge/site pair cannot drift apart.
fn push_ambiguous_inherit(
    t: &mut Tables,
    unresolved_calls: &mut Vec<UnresolvedSite>,
    src_idx: usize,
    class_file: usize,
    name: String,
    site_line: usize,
) {
    t.edges.push(NewEdge {
        src_idx,
        dst_idx: None,
        dst_name: name.clone(),
        kind: EdgeKind::Inherits,
        confidence: Confidence::Ambiguous,
        site_line,
    });
    unresolved_calls.push(UnresolvedSite {
        file_idx: class_file,
        name,
        kind: EdgeSiteKind::Inherit,
        site_line,
    });
}

/// Resolve one base chain and emit its `inherits` edge or its unresolved
/// site. Returns the resolved base qualname when the edge is unambiguous;
/// callers decide whether it joins the MRO table (`class_bases`: fresh
/// declarations do, interface implements lists do not).
fn emit_inherit_edge(
    t: &mut Tables,
    unresolved_calls: &mut Vec<UnresolvedSite>,
    src_idx: usize,
    chain: &[String],
    site_line: usize,
) -> Option<String> {
    let class_file = t.symbols[src_idx].file_idx;
    let src_qn = t.symbols[src_idx].qualname.clone();
    let base_qn = resolve_base_chain(t, class_file, chain, &src_qn);
    let dsts = base_qn
        .as_ref()
        .and_then(|qn| t.by_qualname.get(qn))
        .map(|v| v.as_slice())
        .unwrap_or(&[]);
    let class_dsts: Vec<usize> = dsts
        .iter()
        .copied()
        .filter(|i| is_class_kind(t.symbols[*i].kind))
        .collect();
    if class_dsts.len() > 1 {
        let name = base_qn.clone().unwrap_or_else(|| chain.join("."));
        push_ambiguous_inherit(t, unresolved_calls, src_idx, class_file, name, site_line);
        None
    } else if let Some(&dst) = class_dsts.first() {
        let qn = t.symbols[dst].qualname.clone();
        t.edges.push(NewEdge {
            src_idx,
            dst_idx: Some(dst),
            dst_name: qn.clone(),
            kind: EdgeKind::Inherits,
            confidence: Confidence::Extracted,
            site_line,
        });
        Some(qn)
    } else {
        unresolved_calls.push(UnresolvedSite {
            file_idx: class_file,
            name: base_qn.unwrap_or_else(|| chain.join(".")),
            kind: EdgeSiteKind::Inherit,
            site_line,
        });
        None
    }
}

/// Phase-ordered resolver. Every phase mutates the same `Tables`, and the
/// intermediate state has no meaning outside this function; the phase comments
/// below are the navigational boundary.
pub fn resolve(mut files: Vec<FileExtraction>, carried: CarriedIndex) -> Resolution {
    let mut t = Tables {
        symbols: Vec::new(),
        existing_ids: Vec::new(),
        edges: Vec::new(),
        by_mod_name: HashMap::new(),
        by_qualname: HashMap::new(),
        class_methods: HashMap::new(),
        methods_by_name: HashMap::new(),
        class_bases: HashMap::new(),
        module_paths: HashSet::new(),
        file_ctx: Vec::new(),
        file_by_module: HashMap::new(),
    };
    merge_fresh(&mut t, &mut files);
    merge_carried(&mut t, carried);
    let mut unresolved_calls = seed_unresolved_inherits(&t);
    emit_inherit_phase(&mut t, &mut unresolved_calls);
    let mut stats = ExtractStats::default();
    let mut unresolved_per_file = vec![0usize; t.file_ctx.len()];
    emit_import_edges(&mut t, &mut unresolved_calls);
    dedup_import_edges(&mut t, &mut unresolved_calls);
    reconcile_python_imports(&mut t);
    resolve_call_sites(
        &mut t,
        &mut unresolved_calls,
        &mut unresolved_per_file,
        &mut stats,
    );
    Resolution {
        symbols: t.symbols,
        existing_ids: t.existing_ids,
        edges: t.edges,
        stats,
        unresolved_per_file,
        unresolved_calls,
    }
}

/// Merge fresh extractions: register symbols and build each file's
/// resolution context in one pass.
fn merge_fresh(t: &mut Tables, files: &mut Vec<FileExtraction>) {
    for mut fx in files.drain(..) {
        let maps = fold_binding_maps(&mut fx);
        let base = t.symbols.len();
        let module = fx.module.clone();
        t.module_paths.insert(module.clone());
        for s in fx.symbols {
            register_symbol(t, s, &module, None);
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
        let file_idx = t.file_ctx.len();
        // First file wins: carried symbols carry no FileCtx, so the module
        // -> fresh-file map mirrors the old linear `position()` search.
        t.file_by_module.entry(module.clone()).or_insert(file_idx);
        t.file_ctx.push(FileCtx {
            module,
            lang: fx.lang,
            crate_root: fx.crate_root,
            module_sym_idx: base,
            aliases: maps.aliases,
            locals: maps.locals,
            arrow_params: fx.arrow_params,
            stars: fx.stars,
            from_import_parents: fx.from_import_parents,
            unresolved_inherits: fx.unresolved_inherits,
            assignments: maps.assignments,
            params: maps.params,
            self_fields: maps.self_fields,
            calls: fx.calls,
            import_edges: fx.import_edges,
            inherits,
            impl_inherits,
        });
    }
}

struct BindingMaps {
    aliases: HashMap<String, Vec<(String, ImportTarget)>>,
    locals: HashSet<(String, String)>,
    assignments: BindingEvents,
    params: HashMap<(String, String), Vec<String>>,
    self_fields: HashMap<(String, String), Vec<String>>,
}

/// Fold the per-symbol binding records into the lookup keys resolution uses.
fn fold_binding_maps(fx: &mut FileExtraction) -> BindingMaps {
    let mut aliases: HashMap<String, Vec<(String, ImportTarget)>> = HashMap::new();
    for (scope, alias, target) in std::mem::take(&mut fx.aliases) {
        aliases.entry(alias).or_default().push((scope, target));
    }
    let locals: HashSet<(String, String)> = std::mem::take(&mut fx.locals).into_iter().collect();
    let mut assignments: BindingEvents = HashMap::new();
    for a in std::mem::take(&mut fx.assignments) {
        assignments
            .entry((a.scope_func, a.var))
            .or_default()
            .push((a.site_line, a.target_dotted));
    }
    for events in assignments.values_mut() {
        events.sort_by_key(|(line, _)| *line);
    }
    let mut params = HashMap::new();
    for p in std::mem::take(&mut fx.params) {
        params.insert((p.scope_func, p.var), p.ann_dotted);
    }
    let mut self_fields = HashMap::new();
    for sf in std::mem::take(&mut fx.self_fields) {
        self_fields.insert((sf.class_qn, sf.field), sf.target_dotted);
    }
    BindingMaps {
        aliases,
        locals,
        assignments,
        params,
        self_fields,
    }
}

/// Merge carried symbols (fast path): join the resolution tables; their rows
/// are already stored and are never re-inserted.
fn merge_carried(t: &mut Tables, carried: CarriedIndex) {
    let CarriedIndex {
        symbols: carried_syms,
        inherits: carried_inherits,
    } = carried;
    for c in carried_syms {
        t.module_paths.insert(c.module.clone());
        let sym = NewSymbol {
            kind: c.kind,
            name: c.name.clone(),
            qualname: c.qualname.clone(),
            file_idx: usize::MAX,
            line_start: 0,
            line_end: 0,
            signature: String::new(),

            ..Default::default()
        };
        register_symbol(t, sym, &c.module, Some(c.row_id));
    }
    for (sub_qn, base_qn) in carried_inherits {
        t.class_bases.entry(sub_qn).or_default().push(base_qn);
    }
}

/// Seed the unresolved-site list from each file's unresolved base chains.
fn seed_unresolved_inherits(t: &Tables) -> Vec<UnresolvedSite> {
    let mut unresolved_calls: Vec<UnresolvedSite> = Vec::new();
    for (fi, fctx) in t.file_ctx.iter().enumerate() {
        for (name, site_line) in &fctx.unresolved_inherits {
            unresolved_calls.push(UnresolvedSite {
                file_idx: fi,
                name: name.clone(),
                kind: EdgeSiteKind::Inherit,
                site_line: *site_line,
            });
        }
    }
    unresolved_calls
}

/// Resolve fresh base chains and emit inherits edges (carried rows were
/// carried as edges). Per-file order (inherits, then impl_inherits) is kept.
fn emit_inherit_phase(t: &mut Tables, unresolved_calls: &mut Vec<UnresolvedSite>) {
    // Resolve fresh base chains now that alias tables exist; emit inherits
    // edges for fresh classes (carried inherits edges were carried as rows).
    // Take each file's lists so the emitter can own `&mut Tables` without a
    // whole-corpus snapshot; per-file order (inherits, then impl_inherits)
    // is preserved and nothing re-reads these fields afterwards.
    for fi in 0..t.file_ctx.len() {
        let (inherits, impls) = {
            let f = &mut t.file_ctx[fi];
            (
                std::mem::take(&mut f.inherits),
                std::mem::take(&mut f.impl_inherits),
            )
        };
        for (src_idx, chain) in &inherits {
            let site = t.symbols[*src_idx].line_start;
            // Fresh class declarations populate the MRO table.
            if let Some(base_qn) = emit_inherit_edge(t, unresolved_calls, *src_idx, chain, site) {
                let src_qn = t.symbols[*src_idx].qualname.clone();
                t.class_bases.entry(src_qn).or_default().push(base_qn);
            }
        }
        for (src_idx, chain, line) in &impls {
            // Interface implements lists resolve edges but stay out of the MRO
            // table.
            emit_inherit_edge(t, unresolved_calls, *src_idx, chain, *line);
        }
    }
}

/// Resolve import candidates into edges (or unresolved sites) and update the
/// per-kind counters.
struct ImportEmitter<'a> {
    t: &'a mut Tables,
    unresolved_calls: &'a mut Vec<UnresolvedSite>,
    fi: usize,
    module_sym_idx: usize,
    stars: Vec<(String, usize)>,
}

fn emit_import_edges(t: &mut Tables, unresolved_calls: &mut Vec<UnresolvedSite>) {
    // Take each file's import work one file at a time so the emitter can own
    // `&mut Tables` without a whole-corpus snapshot. `import_edges` is never
    // read again; `stars` is needed later by call resolution, so it is put
    // back after the file's edges are emitted.
    for fi in 0..t.file_ctx.len() {
        let (module_sym_idx, stars, edges) = {
            let f = &mut t.file_ctx[fi];
            (
                f.module_sym_idx,
                std::mem::take(&mut f.stars),
                std::mem::take(&mut f.import_edges)
                    .into_iter()
                    .map(|c| (c.target, c.line))
                    .collect::<Vec<_>>(),
            )
        };
        let mut emitter = ImportEmitter {
            t: &mut *t,
            unresolved_calls: &mut *unresolved_calls,
            fi,
            module_sym_idx,
            stars,
        };
        for (target, line) in edges {
            match &target {
                ImportTarget::Symbol { module, name } => emitter.symbol(module, name, line),
                ImportTarget::Module { path } => emitter.module(path, line),
                ImportTarget::ModuleRoot { path } => emitter.module_root(path, line),
            }
        }
        t.file_ctx[fi].stars = emitter.stars;
    }
}

impl ImportEmitter<'_> {
    /// Push one import edge (resolved or ambiguous).
    fn push_edge(
        &mut self,
        dst: Option<usize>,
        dst_name: &str,
        confidence: Confidence,
        line: usize,
    ) {
        self.t.edges.push(NewEdge {
            src_idx: self.module_sym_idx,
            dst_idx: dst,
            dst_name: dst_name.to_string(),
            kind: EdgeKind::Imports,
            confidence,
            site_line: line,
        });
    }

    /// Record one import site that resolved to no symbol.
    fn push_unresolved(&mut self, name: &str, line: usize) {
        self.unresolved_calls.push(UnresolvedSite {
            file_idx: self.fi,
            name: name.to_string(),
            kind: EdgeSiteKind::Import,
            site_line: line,
        });
    }

    fn symbol(&mut self, module: &str, name: &str, line: usize) {
        let dst_name = format!("{module}.{name}");
        // Decide from the borrowed table first: `dsts` borrows `by_mod_name`
        // and the push helpers borrow the whole emitter.
        enum Decision {
            Skip,
            Edge(Option<usize>, Confidence),
            Fallback,
        }
        let decision = match self
            .t
            .by_mod_name
            .get(&(module.to_string(), name.to_string()))
        {
            Some(dsts) if dsts.is_empty() => Decision::Skip,
            Some(dsts) if dsts.len() > 1 => Decision::Edge(None, Confidence::Ambiguous),
            Some(dsts) => match dsts.first() {
                Some(&dst) => Decision::Edge(Some(dst), Confidence::Extracted),
                None => Decision::Skip,
            },
            None => Decision::Fallback,
        };
        match decision {
            Decision::Skip => {}
            Decision::Edge(dst, confidence) => {
                if dst != Some(self.module_sym_idx) {
                    self.push_edge(dst, &dst_name, confidence, line);
                }
            }
            Decision::Fallback => {
                // `use crate::a::b` where b is a submodule: the symbol
                // candidate resolves to the module symbol.
                let module_dst = self.t.by_qualname.get(&dst_name).and_then(|dsts| {
                    dsts.iter()
                        .copied()
                        .find(|&i| self.t.symbols[i].kind == SymbolKind::Module)
                });
                match module_dst {
                    Some(dst) if dst != self.module_sym_idx => {
                        self.push_edge(Some(dst), &dst_name, Confidence::Extracted, line);
                    }
                    Some(_) => {}
                    None => self.push_unresolved(&dst_name, line),
                }
            }
        }
    }

    fn is_star(&self, path: &str, site_line: usize) -> bool {
        self.stars
            .iter()
            .any(|(m, line)| m == path && *line == site_line)
    }

    fn module(&mut self, path: &str, site_line: usize) {
        if self.is_star(path, site_line) {
            return; // the star edge below is the canonical one
        }
        if let Some(dsts) = self.t.by_qualname.get(path) {
            let modules: Vec<usize> = dsts
                .iter()
                .copied()
                .filter(|&i| self.t.symbols[i].kind == SymbolKind::Module)
                .collect();
            let others: Vec<usize> = dsts
                .iter()
                .copied()
                .filter(|&i| self.t.symbols[i].kind != SymbolKind::Module)
                .collect();
            let candidates = if modules.is_empty() {
                &others
            } else {
                &modules
            };
            match candidates.as_slice() {
                [dst] if *dst != self.module_sym_idx => {
                    self.push_edge(Some(*dst), path, Confidence::Extracted, site_line);
                }
                [] => {}
                _ => {
                    // Duplicate module/symbol candidates are
                    // genuinely ambiguous; never pick one by
                    // fresh/carried insertion order.
                    self.push_edge(None, path, Confidence::Ambiguous, site_line);
                }
            }
        } else {
            self.push_unresolved(path, site_line);
        }
    }

    fn module_root(&mut self, path: &str, site_line: usize) {
        // Star imports use ModuleRoot as their edge target: AMBIGUOUS
        // edge to the module symbol (spec). Plain `import x` emits no
        // edge; resolution happens at call time.
        if !self.is_star(path, site_line) {
            return;
        }
        let dst = self.t.by_qualname.get(path).and_then(|dsts| {
            dsts.iter()
                .copied()
                .find(|&i| self.t.symbols[i].kind == SymbolKind::Module)
        });
        match dst {
            Some(dst) if dst != self.module_sym_idx => {
                self.push_edge(Some(dst), path, Confidence::Ambiguous, site_line);
            }
            None => self.push_unresolved(path, site_line),
            _ => {}
        }
    }
}

/// Deduplicate import edges and reconcile unresolved import candidates: a
/// statement that resolved by any candidate is not unresolved.
fn dedup_import_edges(t: &mut Tables, unresolved_calls: &mut Vec<UnresolvedSite>) {
    let mut seen_edges: HashSet<(usize, Option<usize>, String, usize)> = HashSet::new();
    t.edges.retain(|e| {
        e.kind != EdgeKind::Imports
            || seen_edges.insert((e.src_idx, e.dst_idx, e.dst_name.to_string(), e.site_line))
    });
    let mut resolved: HashSet<(usize, usize, String)> = HashSet::new();
    for e in &t.edges {
        if e.kind == EdgeKind::Imports {
            let fi = t.symbols[e.src_idx].file_idx;
            resolved.insert((fi, e.site_line, e.dst_name.to_string()));
        }
    }
    unresolved_calls.retain(|u| {
        u.kind != EdgeSiteKind::Import
            || !resolved.contains(&(u.file_idx, u.site_line, u.name.to_string()))
    });
    let mut seen_unresolved: HashSet<(usize, String, EdgeSiteKind, usize)> = HashSet::new();
    unresolved_calls
        .retain(|u| seen_unresolved.insert((u.file_idx, u.name.to_string(), u.kind, u.site_line)));
}

/// Python `from pkg import mod`: the child edge `pkg.mod` is canonical when
/// the imported name itself resolves as a module, so drop the redundant
/// parent `pkg` edge. Keep the parent when the same statement also imports a
/// non-module symbol from it (`from pkg import mod, func`).
fn reconcile_python_imports(t: &mut Tables) {
    let mut module_children: HashSet<(usize, usize, String)> = HashSet::new();
    let mut symbol_children: HashSet<(usize, usize, String)> = HashSet::new();
    for e in &t.edges {
        if e.kind != EdgeKind::Imports {
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
        if t.symbols[dst].kind == SymbolKind::Module {
            module_children.insert(key);
        } else {
            symbol_children.insert(key);
        }
    }
    t.edges.retain(|e| {
        if e.kind != EdgeKind::Imports {
            return true;
        }
        let fi = t.symbols[e.src_idx].file_idx;
        if t.file_ctx.get(fi).is_none_or(|f| f.lang != Lang::Python) {
            return true;
        }
        let Some(dst) = e.dst_idx else { return true };
        if t.symbols[dst].kind != SymbolKind::Module {
            return true;
        }
        let fctx = &t.file_ctx[fi];
        let key = (fi, e.site_line, e.dst_name.to_string());
        let parent_candidate = fctx
            .from_import_parents
            .contains(&(e.site_line, e.dst_name.to_string()));
        !parent_candidate || !module_children.contains(&key) || symbol_children.contains(&key)
    });
}

/// Python typed receiver: the class of `x` in `x.m()`, when it resolves.
/// Emitting a `type_ref` edge keeps the incremental dirty closure honest even
/// when the call itself resolves through an inherited method.
fn python_type_ref(t: &Tables, fi: usize, site: &CallSite) -> Option<(String, usize)> {
    if t.file_ctx[fi].lang != Lang::Python {
        return None;
    }
    let Callee::Attr(parts) = &site.callee else {
        return None;
    };
    if parts.len() < 2 {
        return None;
    }
    let class_qn = resolve_var_class(t, fi, &parts[0], &site.scope_func, site.line)?;
    let class_idx = unique_class_index(t, &class_qn)?;
    Some((class_qn, class_idx))
}

/// Resolve every recorded call site into an edge or an unresolved site.
fn resolve_call_sites(
    t: &mut Tables,
    unresolved_calls: &mut Vec<UnresolvedSite>,
    unresolved_per_file: &mut [usize],
    stats: &mut ExtractStats,
) {
    // Call sites.
    for (fi, fctx) in t.file_ctx.iter().enumerate() {
        for site in &fctx.calls {
            let outcome = raw_resolve_call(t, fi, site);
            let src_idx = source_symbol_idx(t, fctx, site);
            // Type-only reference (Python): a typed receiver `x.m()` depends
            // on the class of `x`, not just on the method the call resolves
            // to. Emit a `type_ref` edge to that class regardless of the call
            // outcome, so the incremental dirty closure re-extracts this file
            // when the class's branch changes — otherwise a new MRO hit (a
            // method added on an ancestor) stays hidden behind a carried edge.
            if let Some(edge) = type_ref_edge(t, fi, site, src_idx) {
                t.edges.push(edge);
            }
            match outcome {
                Outcome::Extracted(dst) => {
                    // R6d A6: keep the edge to the class itself so
                    // `callers Class` and `dead` are correct, and add a
                    // second edge to its constructor when unique.
                    let (dst_name, ctor) = extracted_edges(t, dst);
                    push_call_edge(&mut t.edges, src_idx, Some(dst), dst_name, site.line);
                    if let Some((ctor_idx, ctor_name)) = ctor {
                        push_call_edge(&mut t.edges, src_idx, Some(ctor_idx), ctor_name, site.line);
                    }
                }
                Outcome::Ambiguous(name) => {
                    t.edges.push(ambiguous_edge(src_idx, name, site.line));
                }
                Outcome::Dropped => stats.builtin_dropped += 1,
                Outcome::Unresolved => {
                    stats.unresolved_callees += 1;
                    unresolved_per_file[fi] += 1;
                    unresolved_calls.push(UnresolvedSite {
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

/// Push the extracted-confidence call edge shared by the class edge and the
/// unique-constructor edge (R6d A6); the pair differs only in destination.
/// Takes `edges` separately from `Tables` because the caller iterates
/// `t.file_ctx` while pushing, which only borrows-split at field level.
fn push_call_edge(
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

fn ambiguous_edge(src_idx: usize, name: String, site_line: usize) -> NewEdge {
    NewEdge {
        src_idx,
        dst_idx: None,
        dst_name: name,
        kind: EdgeKind::Calls,
        confidence: Confidence::Ambiguous,
        site_line,
    }
}

fn source_symbol_idx(t: &Tables, fctx: &FileCtx, site: &CallSite) -> usize {
    if site.scope_func.is_empty() {
        return fctx.module_sym_idx;
    }
    t.by_qualname
        .get(&site.scope_func)
        .and_then(|v| v.first().copied())
        .unwrap_or(fctx.module_sym_idx)
}

fn type_ref_edge(t: &Tables, fi: usize, site: &CallSite, src_idx: usize) -> Option<NewEdge> {
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
fn extracted_edges(t: &Tables, dst: usize) -> (String, Option<(usize, String)>) {
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

fn unresolved_site_name(site: &CallSite) -> String {
    match &site.callee {
        Callee::Name(n) => n.clone(),
        Callee::Attr(parts) => parts.join("."),
    }
}

/// Unique constructor of a class (`__init__` / `constructor`), if any.
/// Instantiations emit edges to BOTH the class and this constructor (R6d A6).
fn ctor_target(t: &Tables, class_idx: usize) -> Option<usize> {
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
struct CallCtx<'a> {
    t: &'a Tables,
    fi: usize,
    fctx: &'a FileCtx,
    site: &'a CallSite,
}

fn raw_resolve_call(t: &Tables, fi: usize, site: &CallSite) -> Outcome {
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

fn resolve_name_callee(ctx: &CallCtx<'_>, name: &str) -> Outcome {
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
    for (m, _) in &ctx.fctx.stars {
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

fn lexical_scope_lookup(ctx: &CallCtx<'_>, name: &str) -> Option<Outcome> {
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

fn alias_binding_lookup(ctx: &CallCtx<'_>, name: &str) -> Option<Outcome> {
    alias_target_outcome(ctx.t, file_alias(ctx.fctx, name)?, name)
}

/// Shared alias resolution for the Symbol and ModuleRoot import targets:
/// follow the binding through re-exports, drop bindings to modules that are
/// not indexed, and try the same-named child for a root-level binding.
/// `None` leaves the fall-through value to the caller.
fn alias_target_outcome(t: &Tables, target: &ImportTarget, name: &str) -> Option<Outcome> {
    match target {
        ImportTarget::Symbol { module, name: n } => {
            let mut seen: HashSet<(String, String)> = HashSet::new();
            if let Some(out) = follow_binding(t, module, n, 0, &mut seen, name) {
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

fn resolve_attr_callee(ctx: &CallCtx<'_>, chain: &[String]) -> Outcome {
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
fn receiver_typing(ctx: &CallCtx<'_>, chain: &[String], method: &str) -> Option<Outcome> {
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

fn class_path_lookup(ctx: &CallCtx<'_>, chain: &[String], method: &str) -> Option<Outcome> {
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
fn resolve_receiver_method(ctx: &CallCtx<'_>, chain: &[String], method: &str) -> Option<Outcome> {
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
fn resolve_rust_keyword_path(ctx: &CallCtx<'_>, chain: &[String]) -> Option<Outcome> {
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
fn qualname_outcome(t: &Tables, full: &str) -> Outcome {
    match t.by_qualname.get(full) {
        Some(cands) => match cands.len() {
            1 => Outcome::Extracted(cands[0]),
            0 => Outcome::Unresolved,
            _ => Outcome::Ambiguous(full.to_string()),
        },
        None => Outcome::Unresolved,
    }
}

fn crate_keyword_path(ctx: &CallCtx<'_>, chain: &[String]) -> Outcome {
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
const UBIQUITOUS_METHODS: &[&str] = &[
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
fn rust_unknown_receiver_outcome(t: &Tables, fi: usize, method: &str) -> Outcome {
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
fn module_scoped_call(t: &Tables, module: &str, rest: &[String]) -> Outcome {
    let out = chain_from_module(t, module.to_string(), rest);
    if !matches!(out, Outcome::Unresolved) {
        return out;
    }
    let full = format!("{module}.{}", rest.join("."));
    qualname_outcome(t, &full)
}

/// Replace a leading `crate` segment with the crate root dir segment.
fn root_chain(root: &str, chain: &[String]) -> Vec<String> {
    let mut out = vec![root.to_string()];
    out.extend_from_slice(&chain[1..]);
    out
}

/// Does the first chain segment bind to a module (alias or bare indexed name)?
fn chain_is_module_start(t: &Tables, fi: usize, first: &str, scope: &str) -> bool {
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
fn bare_candidates(cands: Option<&Vec<usize>>, display: &str) -> Option<Outcome> {
    let v = cands?;
    match v.len() {
        1 => Some(Outcome::Extracted(v[0])),
        0 => None,
        _ => Some(Outcome::Ambiguous(display.to_string())),
    }
}

/// Nearest import binding for `name` at `scope`: an exact local scope (or a
/// nested function under it) beats a file-level binding.
fn nearest_alias_scoped<'a>(
    fctx: &'a FileCtx,
    scope: &str,
    name: &str,
) -> Option<(&'a str, &'a ImportTarget)> {
    let entries = fctx.aliases.get(name)?;
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

fn file_alias<'a>(fctx: &'a FileCtx, name: &str) -> Option<&'a ImportTarget> {
    fctx.aliases
        .get(name)?
        .iter()
        .rev()
        .find(|(scope, _)| scope.is_empty())
        .map(|(_, target)| target)
}

/// A parameter or local variable in the current or an enclosing function
/// shadows module and file-level bindings. Python class scope is not lexical.
fn is_local_binding(fctx: &FileCtx, site: &CallSite, name: &str) -> bool {
    // Anonymous callback parameters shadow only inside their line range.
    if fctx.arrow_params.iter().any(|p| {
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
        if fctx.locals.contains(&key) {
            return true;
        }
        // An assignment shadows only from its own line onward; a later
        // assignment must not erase an earlier valid bare call.
        if let Some(events) = fctx.assignments.get(&key) {
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

fn local_alias_outcome(t: &Tables, target: &ImportTarget, name: &str) -> Outcome {
    alias_target_outcome(t, target, name).unwrap_or(Outcome::Unresolved)
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
        }) => follow_binding(t, m2, n2, depth + 1, seen, display),
        _ => None,
    }
}

/// MRO walk, level by level: one match at the nearest level wins; several at
/// the same level are AMBIGUOUS; exhausted -> UNRESOLVED. Bounded at 6 levels
/// (deliberate work cap; deeper hierarchies stay UNRESOLVED and surface in
/// `aw unresolved`).
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

/// Latest annotation or assignment at or before `site_line` in one scope.
/// `include_params` treats a parameter annotation as a binding at line 0.
fn latest_binding<'a>(
    fctx: &'a FileCtx,
    scope: &str,
    var: &str,
    site_line: usize,
    include_params: bool,
) -> Option<&'a Vec<String>> {
    let mut best: Option<(usize, &'a Vec<String>)> = None;
    if include_params {
        if let Some(ann) = fctx.params.get(&(scope.to_string(), var.to_string())) {
            best = Some((0, ann));
        }
    }
    if let Some(events) = fctx.assignments.get(&(scope.to_string(), var.to_string())) {
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
fn resolve_var_class(
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

fn resolve_class_of_chain(t: &Tables, fi: usize, chain: &[String], scope: &str) -> Option<String> {
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

fn crate_class(t: &Tables, root: Option<&str>, chain: &[String]) -> Option<String> {
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

fn alias_class(t: &Tables, fctx: &FileCtx, chain: &[String], scope: &str) -> Option<String> {
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
fn follow_reexport(t: &Tables, module: &str, name: &str) -> Option<String> {
    let mut seen: HashSet<(String, String)> = HashSet::new();
    match follow_binding(t, module, name, 0, &mut seen, name) {
        Some(Outcome::Extracted(idx)) if t.symbols[idx].kind == SymbolKind::Class => {
            Some(t.symbols[idx].qualname.to_string())
        }
        _ => None,
    }
}

fn unique_class_exists(t: &Tables, qualname: &str) -> bool {
    unique_class_index(t, qualname).is_some()
}

/// The single class/trait/interface index for a qualname, when exactly one
/// exists. The `is_some()` of this function is also the existence test.
fn unique_class_index(t: &Tables, qualname: &str) -> Option<usize> {
    t.by_qualname.get(qualname).and_then(|v| {
        let mut classes = v
            .iter()
            .copied()
            .filter(|&i| is_class_kind(t.symbols[i].kind));
        let first = classes.next()?;
        classes.next().is_none().then_some(first)
    })
}

fn class_exists(t: &Tables, qualname: &str) -> bool {
    t.by_qualname
        .get(qualname)
        .is_some_and(|v| v.iter().any(|&i| is_class_kind(t.symbols[i].kind)))
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
fn module_chain_outcome(t: &Tables, fi: usize, chain: &[String], scope: &str) -> Outcome {
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
        assert!(fx.aliases.iter().any(|(_, a, _)| a == "T"), "alias T");
        assert!(fx.aliases.iter().any(|(_, a, _)| a == "u"), "alias u");
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
                .any(|(_, a, _)| a == "knowledge_artifacts_dir"),
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
