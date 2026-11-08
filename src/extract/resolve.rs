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
    bindings: ResolvedBindings,
    unresolved_inherits: Vec<(String, usize)>,
    calls: Vec<CallSite>,
    import_edges: Vec<ImportEdgeCandidate>,
    /// (src idx global, base chain)
    inherits: Vec<(usize, Vec<String>)>,
    /// (src idx global, base chain, impl line) — edge-only, no class_bases.
    impl_inherits: Vec<(usize, Vec<String>, usize)>,
}

/// Resolution-time view of one file's [`FileBindings`] (extract.rs): the
/// per-symbol binding records folded into the lookup keys resolution reads
/// (`aliases`, `locals`, `assignments`, `params`, `self_fields`) plus the
/// records it consumes as-is (`arrow_params`, `stars`, `from_import_parents`).
/// Grouped with `FileBindings` so a new binding concept registers in one
/// block per side instead of per-field across `FileCtx` and `merge_fresh`.
struct ResolvedBindings {
    aliases: HashMap<String, Vec<(String, ImportTarget)>>,
    /// (scope_func, var) parameters/locals that shadow outer names.
    locals: HashSet<(String, String)>,
    /// Anonymous callback parameters, scoped by source line range.
    arrow_params: Vec<ArrowParam>,
    stars: Vec<(String, usize)>,
    from_import_parents: HashSet<(usize, String)>,
    /// (scope_func, var) -> ordered (site_line, constructor target chain).
    assignments: BindingEvents,
    /// (scope_func, var) -> annotation chain.
    params: HashMap<(String, String), Vec<String>>,
    /// (class qualname, field) -> constructor target chain.
    self_fields: HashMap<(String, String), Vec<String>>,
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
    index_parent_module(t, idx, module);
    index_method_class(t, idx, &name, &qualname, kind);
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
    emit_inherit_phase(EmitCtx {
        t: &mut t,
        unresolved_calls: &mut unresolved_calls,
    });
    let mut stats = ExtractStats::default();
    let mut unresolved_per_file = vec![0usize; t.file_ctx.len()];
    emit_import_edges(EmitCtx {
        t: &mut t,
        unresolved_calls: &mut unresolved_calls,
    });
    dedup_import_edges(&mut t, &mut unresolved_calls);
    reconcile_python_imports(&mut t);
    resolve_call_sites(
        EmitCtx {
            t: &mut t,
            unresolved_calls: &mut unresolved_calls,
        },
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
            bindings: fold_binding_maps(&mut fx.bindings),
            unresolved_inherits: fx.unresolved_inherits,
            calls: fx.calls,
            import_edges: fx.import_edges,
            inherits,
            impl_inherits,
        });
    }
}

/// Fold the per-symbol binding records into the lookup keys resolution uses.
/// One block move out of [`FileBindings`]; the folded maps replace the raw
/// vectors and the pass-through records keep their order.
fn fold_binding_maps(fx: &mut FileBindings) -> ResolvedBindings {
    let bindings = std::mem::take(fx);
    let mut aliases: HashMap<String, Vec<(String, ImportTarget)>> = HashMap::new();
    for (scope, alias, target) in bindings.aliases {
        aliases.entry(alias).or_default().push((scope, target));
    }
    let locals: HashSet<(String, String)> = bindings.locals.into_iter().collect();
    let mut assignments: BindingEvents = HashMap::new();
    for a in bindings.assignments {
        assignments
            .entry((a.scope_func, a.var))
            .or_default()
            .push((a.site_line, a.target_dotted));
    }
    for events in assignments.values_mut() {
        events.sort_by_key(|(line, _)| *line);
    }
    let mut params = HashMap::new();
    for p in bindings.params {
        params.insert((p.scope_func, p.var), p.ann_dotted);
    }
    let mut self_fields = HashMap::new();
    for sf in bindings.self_fields {
        self_fields.insert((sf.class_qn, sf.field), sf.target_dotted);
    }
    ResolvedBindings {
        aliases,
        locals,
        arrow_params: bindings.arrow_params,
        stars: bindings.stars,
        from_import_parents: bindings.from_import_parents,
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
        // Carried rows are never re-extracted, so they have no fresh file:
        // usize::MAX marks them. The only read is the `file_idx != fi` skip
        // in `rust_unknown_receiver_outcome`; it is never used as a `file_ctx`
        // index, and the assert keeps it distinct from any real file index.
        debug_assert!(
            t.file_ctx.len() < usize::MAX,
            "carried file_idx sentinel collided with a real file index"
        );
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
fn emit_inherit_phase(mut ctx: EmitCtx<'_>) {
    // Resolve fresh base chains now that alias tables exist; emit inherits
    // edges for fresh classes (carried inherits edges were carried as rows).
    // Take each file's lists so the emitter can own `&mut Tables` without a
    // whole-corpus snapshot; per-file order (inherits, then impl_inherits)
    // is preserved and nothing re-reads these fields afterwards.
    for fi in 0..ctx.t.file_ctx.len() {
        let (inherits, impls) = {
            let f = &mut ctx.t.file_ctx[fi];
            (
                std::mem::take(&mut f.inherits),
                std::mem::take(&mut f.impl_inherits),
            )
        };
        for (src_idx, chain) in &inherits {
            let site = ctx.t.symbols[*src_idx].line_start;
            // Fresh class declarations populate the MRO table.
            if let Some(base_qn) = emit_inherit_edge(&mut ctx, *src_idx, chain, site) {
                let src_qn = ctx.t.symbols[*src_idx].qualname.clone();
                ctx.t.class_bases.entry(src_qn).or_default().push(base_qn);
            }
        }
        for (src_idx, chain, line) in &impls {
            // Interface implements lists resolve edges but stay out of the MRO
            // table.
            emit_inherit_edge(&mut ctx, *src_idx, chain, *line);
        }
    }
}

/// Resolve import candidates into edges (or unresolved sites) and update the
mod attr;
mod bindings;
mod calls;
mod emit;
mod imports;
#[cfg(test)]
mod tests;

use attr::*;
use bindings::*;
use calls::*;
use emit::*;
use imports::*;
