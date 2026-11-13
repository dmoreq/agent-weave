//! D1 callee-footprint overlap. Counts project symbol IDs, never bare names,
//! so same-named helpers in different modules stay distinct.

use super::*;
use crate::query::reuse::confidence::distinctive_weakness;

/// A two-hop closure this broad describes a hub, not a wrapper. Skipping it
/// keeps `--deep-callee` linear on generated hub corpora instead of comparing
/// every worker against every other worker's thousands of leaf callees.
const MAX_DEEP_FOOTPRINT: usize = 512;

/// `C_in(s)`: resolved project callees in the source's language family whose
/// names survived the static/dynamic stoplist.
fn d1_project_ids(corpus: &Corpus<'_>, symbol: &SymbolData) -> std::collections::HashSet<i64> {
    symbol
        .callee_ids
        .iter()
        .copied()
        .filter(|id| {
            corpus.symbols.get(id).is_some_and(|callee| {
                !cross_language(symbol, callee) && symbol.resolved_callees.contains(&callee.name)
            })
        })
        .collect()
}

/// Weighted deep scan. Computes the expensive shared-evidence set only for a
/// candidate that can still beat the current best, and stops after the
/// per-invocation pair budget.
fn find_best_d1_deep(target: &SymbolData, corpus: &Corpus<'_>) -> Option<ReuseCandidate> {
    type DeepBest<'a> = (
        u64,
        &'a SymbolData,
        Vec<String>,
        ReuseConfidence,
        Vec<ReasonCode>,
    );
    let mut best: Option<DeepBest<'_>> = None;
    for candidate in candidate_symbols(corpus, target, PostingKind::DeepCallee, 2) {
        if corpus.deep_budget.get() == 0 {
            corpus.guard();
            break;
        }
        corpus.deep_budget.set(corpus.deep_budget.get() - 1);
        let same_file_overlap = candidate.file_id == target.file_id
            && !(candidate.line_end < target.line_start || candidate.line_start > target.line_end);
        if candidate.id == target.id || same_file_overlap {
            corpus.guard();
            continue;
        }
        if cross_language(target, candidate) || !is_more_canonical(candidate, target) {
            corpus.guard();
            continue;
        }
        if target.callee_ids.contains(&candidate.id) || candidate.callee_ids.contains(&target.id) {
            corpus.guard();
            continue;
        }
        if candidate.deep_weights.len() < 3 || candidate.deep_weights.len() > MAX_DEEP_FOOTPRINT {
            continue;
        }
        let ratio = weighted_containment(target, candidate);
        if ratio < 0.85 {
            continue;
        }
        let bal = weighted_balance(target, candidate);
        if bal < 0.30 {
            corpus.guard();
            continue;
        }
        let pct = (ratio * 100.0).round() as u64;
        let can_win = best.as_ref().is_none_or(|(best_pct, best_sym, ..)| {
            pct > *best_pct || (pct == *best_pct && is_more_canonical(candidate, best_sym))
        });
        if !can_win {
            continue;
        }
        let shared_ids: Vec<i64> = target
            .deep_weights
            .keys()
            .filter(|id| candidate.deep_weights.contains_key(*id))
            .copied()
            .collect();
        let mut shared: Vec<String> = shared_ids
            .iter()
            .filter_map(|id| corpus.symbols.get(id))
            .map(|sym| sym.name.clone())
            .collect();
        shared.sort();
        shared.dedup();
        let mut reasons = Vec::new();
        let unresolved_composition = target.unresolved_callees.contains(&candidate.name)
            || candidate.unresolved_callees.contains(&target.name);
        if unresolved_composition {
            reasons.push(ReasonCode::CallerCallee);
        }
        if target
            .name_tokens
            .intersection(&candidate.name_tokens)
            .next()
            .is_none()
        {
            reasons.push(ReasonCode::ZeroName);
        }
        if bal < 0.50 {
            reasons.push(ReasonCode::SizeImbalance);
        }
        let weak = distinctive_weakness(corpus, &shared_ids);
        let confidence = if !reasons.is_empty() || weak > 0.75 {
            ReuseConfidence::Low
        } else if ratio >= 0.95 && bal >= 0.75 && weak <= 0.25 {
            ReuseConfidence::High
        } else if ratio >= 0.90 {
            ReuseConfidence::Medium
        } else {
            ReuseConfidence::Provisional
        };
        best = Some((pct, candidate, shared, confidence, reasons));
    }
    best.map(|(pct, candidate, shared, confidence, reasons)| {
        make_candidate(
            DetectorId::D1,
            target,
            candidate,
            Some(pct),
            &shared,
            MatchMeta {
                raw_client: None,
                confidence,
                reasons,
            },
        )
    })
}

pub(crate) fn find_best_d1(target: &SymbolData, corpus: &Corpus<'_>) -> Option<ReuseCandidate> {
    if corpus.deep_callee {
        if target.deep_weights.len() < 3 || target.deep_weights.len() > MAX_DEEP_FOOTPRINT {
            corpus.guard();
            return None;
        }
        return find_best_d1_deep(target, corpus);
    }
    let target_ids = d1_project_ids(corpus, target);
    if target_ids.len() < 3 {
        return None;
    }
    best_by_overlap(
        DetectorId::D1,
        target,
        corpus,
        PostingKind::Callee,
        2,
        |candidate| {
            let c_ids = d1_project_ids(corpus, candidate);
            // GUARD-COMP: a resolved call edge in either direction is a
            // caller/callee composition, not duplication.
            if target.callee_ids.contains(&candidate.id)
                || candidate.callee_ids.contains(&target.id)
            {
                corpus.guard();
                return None;
            }
            // Only unresolved bare-name overlaps are composition hints; a
            // call resolved to a different project symbol is not this pair.
            let unresolved_composition = target.unresolved_callees.contains(&candidate.name)
                || candidate.unresolved_callees.contains(&target.name);
            let zero_name = target
                .name_tokens
                .intersection(&candidate.name_tokens)
                .next()
                .is_none();
            if c_ids.len() < 3 {
                return None;
            }
            let shared_ids: Vec<i64> = target_ids.intersection(&c_ids).copied().collect();
            if shared_ids.len() < 2 {
                return None;
            }
            let mut shared: Vec<String> = shared_ids
                .iter()
                .filter_map(|id| corpus.symbols.get(id))
                .map(|sym| sym.name.clone())
                .collect();
            shared.sort();
            shared.dedup();
            // GUARD-BAL: reject < 0.30, low in [0.30, 0.50), provisional >= 0.50.
            let min_len = target_ids.len().min(c_ids.len());
            let max_len = target_ids.len().max(c_ids.len());
            let bal = min_len as f64 / max_len as f64;
            if bal < 0.30 {
                corpus.guard();
                return None;
            }
            let ratio_candidate = shared_ids.len() as f64 / c_ids.len() as f64;
            let ratio_target = shared_ids.len() as f64 / target_ids.len() as f64;
            let ratio = ratio_candidate.max(ratio_target);
            if ratio < 0.80 {
                return None;
            }
            let pct = (ratio * 100.0).round() as u64;
            let mut reasons = Vec::new();
            if unresolved_composition {
                reasons.push(ReasonCode::CallerCallee);
            }
            if zero_name {
                reasons.push(ReasonCode::ZeroName);
            }
            if bal < 0.50 {
                reasons.push(ReasonCode::SizeImbalance);
            }
            let weak = distinctive_weakness(corpus, &shared_ids);
            let confidence = if !reasons.is_empty() || weak > 0.75 {
                ReuseConfidence::Low
            } else if ratio >= 0.95 && bal >= 0.75 && weak <= 0.25 {
                ReuseConfidence::High
            } else if ratio >= 0.90 {
                ReuseConfidence::Medium
            } else {
                ReuseConfidence::Provisional
            };
            Some((pct, shared, confidence, reasons))
        },
    )
}
