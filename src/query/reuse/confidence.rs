//! Confidence signals that are not part of a detector's overlap frame.

use super::detectors::Corpus;

/// D1's non-distinctive share: shared project callees called by more than two
/// sources. A pair whose shared footprint is mostly generic is Low even when
/// containment is high.
pub(super) fn distinctive_weakness(corpus: &Corpus<'_>, shared_ids: &[i64]) -> f64 {
    if shared_ids.is_empty() {
        return 1.0;
    }
    let distinctive = shared_ids
        .iter()
        .filter(|id| {
            corpus
                .df_all_excl
                .is_none_or(|df| df.get(id).copied().unwrap_or(0) <= 2)
        })
        .count();
    (shared_ids.len().saturating_sub(distinctive)) as f64 / shared_ids.len() as f64
}
