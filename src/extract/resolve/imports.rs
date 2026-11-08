//! Import-edge emission: star-import binding, alias maps, dedup, and
//! Python import reconciliation against module indexes.

use super::*;

/// per-kind counters.
pub(super) struct ImportEmitter<'a> {
    pub(super) t: &'a mut Tables,
    pub(super) unresolved_calls: &'a mut Vec<UnresolvedSite>,
    pub(super) fi: usize,
    pub(super) module_sym_idx: usize,
    pub(super) stars: Vec<(String, usize)>,
}

pub(super) fn emit_import_edges(ctx: EmitCtx<'_>) {
    // Take each file's import work one file at a time so the emitter can own
    // `&mut Tables` without a whole-corpus snapshot. `import_edges` is never
    // read again; `stars` is needed later by call resolution, so it is put
    // back after the file's edges are emitted.
    for fi in 0..ctx.t.file_ctx.len() {
        let (module_sym_idx, stars, edges) = {
            let f = &mut ctx.t.file_ctx[fi];
            (
                f.module_sym_idx,
                std::mem::take(&mut f.bindings.stars),
                std::mem::take(&mut f.import_edges)
                    .into_iter()
                    .map(|c| (c.target, c.line))
                    .collect::<Vec<_>>(),
            )
        };
        let mut emitter = ImportEmitter {
            t: &mut *ctx.t,
            unresolved_calls: &mut *ctx.unresolved_calls,
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
        ctx.t.file_ctx[fi].bindings.stars = emitter.stars;
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
pub(super) fn dedup_import_edges(t: &mut Tables, unresolved_calls: &mut Vec<UnresolvedSite>) {
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
pub(super) fn reconcile_python_imports(t: &mut Tables) {
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
            .bindings
            .from_import_parents
            .contains(&(e.site_line, e.dst_name.to_string()));
        !parent_candidate || !module_children.contains(&key) || symbol_children.contains(&key)
    });
}

/// Python typed receiver: the class of `x` in `x.m()`, when it resolves.
/// Emitting a `type_ref` edge keeps the incremental dirty closure honest even
/// when the call itself resolves through an inherited method.
pub(super) fn python_type_ref(t: &Tables, fi: usize, site: &CallSite) -> Option<(String, usize)> {
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
