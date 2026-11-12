//! Final confidence signals that are not part of a detector's overlap frame.
//! Phase 1 emits `provisional` only when D1's shared footprint is distinctive.

use super::corpus::SymbolData;
use super::detectors::Corpus;

/// D1's non-distinctive share: resolved shared callees that are either not
/// project symbols or are called by more than two sources. A pair whose shared
/// footprint is mostly generic is Low even when containment is high.
pub(super) fn distinctive_ratio(
    target: &SymbolData,
    candidate: &SymbolData,
    corpus: &Corpus<'_>,
    shared: &[String],
) -> f64 {
    if shared.is_empty() {
        return 1.0;
    }
    let distinctive = target
        .callee_ids
        .intersection(&candidate.callee_ids)
        .filter_map(|id| corpus.symbols.get(id))
        .filter(|sym| target.resolved_callees.contains(&sym.name))
        .filter(|sym| {
            corpus
                .df_all_excl
                .is_none_or(|df| df.get(&sym.id).copied().unwrap_or(0) <= 2)
        })
        .count();
    (shared.len().saturating_sub(distinctive)) as f64 / shared.len() as f64
}
