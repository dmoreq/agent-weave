//! Detectors D1-D4: callee-footprint containment, interface token overlap,
//! raw-client abstraction bypass, and guarded pass-through wrappers.

use super::corpus::SymbolData;
use super::index::{CorpusIndex, PostingKind};
use super::*;
use std::collections::HashMap;

/// One immutable view of the corpus shared by every detector: the symbol map,
/// the optional inverted index, and the deep-callee experiment switch.
pub(super) struct Corpus<'a> {
    pub(super) symbols: &'a HashMap<i64, SymbolData>,
    pub(super) index: Option<&'a CorpusIndex>,
    pub(super) deep_callee: bool,
}

/// Raw external clients targeted by D3 abstraction bypass.
pub(crate) const RAW_CLIENTS: &[&str] = &[
    "Connection::open",
    "Connection.open",
    "sqlite3_open",
    "requests.get",
    "requests.post",
    "axios",
    "axios.get",
    "axios.post",
    "fetch",
    "reqwest::get",
    "reqwest.get",
    "ureq::get",
    "ureq.get",
];

pub(crate) fn is_raw_client(callee: &str) -> bool {
    RAW_CLIENTS.iter().any(|&r| raw_client_eq(r, callee))
}

fn raw_client_eq(a: &str, b: &str) -> bool {
    a == b || a.replace("::", ".") == b.replace("::", ".")
}

/// One reuse detector: resolve a target against the corpus to a candidate.
pub(super) type Detector = fn(&SymbolData, &Corpus<'_>) -> Option<ReuseCandidate>;

/// Detector candidates for one signal. With an index this is the posting-list
/// union filtered to `min_shared`; without one it is the flat corpus scan.
/// Either way the list is a superset of the detector's exact matches.
fn candidate_symbols<'a>(
    corpus: &'a Corpus<'_>,
    target: &SymbolData,
    kind: PostingKind,
    min_shared: usize,
) -> Vec<&'a SymbolData> {
    match corpus.index {
        Some(index) => index
            .candidates(target, kind, min_shared)
            .into_iter()
            .filter_map(|id| corpus.symbols.get(&id))
            .collect(),
        None => corpus.symbols.values().collect(),
    }
}

/// The detector cascade in priority order; a target stops at the first
/// detector that produces a candidate.
/// The closed D1-D4 detector set. `as_str` keeps the journal and dashboard
/// spelling ("D1"...) byte-identical; exhaustive matches replace the old
/// string comparisons and their silent `_` fallbacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectorId {
    /// Callee Footprint Overlap.
    D1,
    /// Interface Token Overlap.
    D2,
    /// Raw Client Bypass.
    D3,
    /// Redundant Pass-Through Wrapper.
    D4,
}

impl DetectorId {
    /// The stable wire spelling used in journal hints and dashboard JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            DetectorId::D1 => "D1",
            DetectorId::D2 => "D2",
            DetectorId::D3 => "D3",
            DetectorId::D4 => "D4",
        }
    }
}

pub(super) const DETECTORS: &[Detector] = &[find_best_d1, find_best_d2, check_d3, check_d4];

/// Deterministic ordering: candidate is more canonical if it has more callers,
/// or alphabetically earlier file/line/qualname as tie-breaker.
fn is_more_canonical(candidate: &SymbolData, target: &SymbolData) -> bool {
    if candidate.inbound_callers != target.inbound_callers {
        candidate.inbound_callers > target.inbound_callers
    } else {
        (
            candidate.file_path.as_str(),
            candidate.line_start,
            candidate.qualname.as_str(),
        ) < (
            target.file_path.as_str(),
            target.line_start,
            target.qualname.as_str(),
        )
    }
}

/// `find_best_d1` -> `find_best_d`. Numbered siblings share a stem: they are a
/// deliberate family, so a matching name must not read as duplication.
fn strip_trailing_digits(name: &str) -> &str {
    name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '_')
}

/// The identity block every detector builds from a matched (target,
/// candidate) pair; detectors only differ in the overlap signal above it.
/// `shared` is sorted into the candidate's display items.
fn make_candidate(
    detector: DetectorId,
    target: &SymbolData,
    candidate: &SymbolData,
    overlap_pct: u64,
    shared: &[String],
) -> ReuseCandidate {
    let mut shared_items: Vec<String> = shared.to_vec();
    shared_items.sort();
    ReuseCandidate {
        verdict: None,
        detector,
        symbol: target.qualname.clone(),
        file: target.file_path.clone(),
        line: target.line_start,
        canonical: candidate.qualname.clone(),
        canonical_file: candidate.file_path.clone(),
        canonical_line: candidate.line_start,
        canonical_callers: candidate.inbound_callers,
        overlap_pct,
        shared_items,
    }
}

/// Shared D1/D2 scan frame: walk the corpus, drop the target's own id/file
/// and non-canonical candidates, let `signal` produce the `(pct, shared)`
/// overlap, and keep the best by overlap percentage. An equal percentage
/// breaks the tie by the canonical key (file_path, line_start, qualname
/// ascending) so the winner does not depend on HashMap iteration order.
fn best_by_overlap(
    detector: DetectorId,
    target: &SymbolData,
    corpus: &Corpus<'_>,
    kind: PostingKind,
    min_shared: usize,
    signal: impl Fn(&SymbolData) -> Option<(u64, Vec<String>)>,
) -> Option<ReuseCandidate> {
    let mut best: Option<(u64, &SymbolData, Vec<String>)> = None;
    for candidate in candidate_symbols(corpus, target, kind, min_shared) {
        if candidate.id == target.id {
            continue;
        }
        // If in the same file, drop enclosing/overlapping spans (e.g. methods in class or nested fns)
        if candidate.file_id == target.file_id
            && !(candidate.line_end < target.line_start || candidate.line_start > target.line_end)
        {
            continue;
        }
        if !is_more_canonical(candidate, target) {
            continue;
        }
        let Some((pct, shared)) = signal(candidate) else {
            continue;
        };
        let better = match &best {
            None => true,
            Some((best_pct, best_sym, _)) => {
                pct > *best_pct || (pct == *best_pct && is_more_canonical(candidate, best_sym))
            }
        };
        if better {
            best = Some((pct, candidate, shared));
        }
    }
    best.map(|(pct, candidate, shared)| make_candidate(detector, target, candidate, pct, &shared))
}

/// Detector D1: Callee Footprint Containment.
/// Flag when (g, f) not in calls, |C(f)| >= 3, |resolved(g) ∩ resolved(f)| >= 2,
/// and max(|C(g) ∩ C(f)| / |C(f)|, |C(g) ∩ C(f)| / |C(g)|) >= 0.80.
/// With `--deep-callee`, the footprint extends to second-hop callees (weight
/// 0.5) and the containment ratio is computed over the weighted closure.
fn find_best_d1(target: &SymbolData, corpus: &Corpus<'_>) -> Option<ReuseCandidate> {
    if corpus.deep_callee {
        if target.deep_weights.len() < 3 {
            return None;
        }
    } else if target.all_callees.len() < 3 || target.resolved_callees.len() < 2 {
        return None;
    }
    best_by_overlap(
        DetectorId::D1,
        target,
        corpus,
        if corpus.deep_callee {
            PostingKind::DeepCallee
        } else {
            PostingKind::Callee
        },
        2,
        |candidate| {
            let c_callees = &candidate.all_callees;
            if target.callee_ids.contains(&candidate.id)
                || candidate.callee_ids.contains(&target.id)
                || target.all_callees.contains(&candidate.name)
                || candidate.all_callees.contains(&target.name)
            {
                return None;
            }
            if corpus.deep_callee {
                if candidate.deep_weights.len() < 3 {
                    return None;
                }
                let ratio = weighted_containment(target, candidate);
                if ratio < 0.85 {
                    return None;
                }
                let shared: Vec<String> = target
                    .all_callees
                    .intersection(c_callees)
                    .cloned()
                    .collect();
                return Some(((ratio * 100.0).round() as u64, shared));
            }
            if c_callees.len() < 3 || candidate.resolved_callees.len() < 2 {
                return None;
            }
            if target
                .resolved_callees
                .intersection(&candidate.resolved_callees)
                .count()
                < 2
            {
                return None;
            }
            let shared: Vec<String> = target
                .all_callees
                .intersection(c_callees)
                .cloned()
                .collect();
            // Guard size balance: min callees must be at least 50% of max callees
            let min_len = target.all_callees.len().min(c_callees.len());
            let max_len = target.all_callees.len().max(c_callees.len());
            if min_len * 2 < max_len {
                return None;
            }
            let ratio_candidate = shared.len() as f64 / c_callees.len() as f64;
            let ratio_target = shared.len() as f64 / target.all_callees.len() as f64;
            let ratio = ratio_candidate.max(ratio_target);
            if ratio < 0.80 {
                return None;
            }
            Some(((ratio * 100.0).round() as u64, shared))
        },
    )
}

/// Weighted containment of the smaller deep footprint in the larger one:
/// shared weight divided by the smaller footprint's total weight. 1.0 means
/// the smaller call graph is fully covered by the larger.
fn weighted_containment(target: &SymbolData, candidate: &SymbolData) -> f64 {
    let target_total: f64 = target.deep_weights.values().sum();
    let candidate_total: f64 = candidate.deep_weights.values().sum();
    if target_total <= 0.0 || candidate_total <= 0.0 {
        return 0.0;
    }
    let target_shared: f64 = target
        .deep_weights
        .iter()
        .filter(|(name, _)| candidate.deep_weights.contains_key(*name))
        .map(|(_, weight)| weight)
        .sum();
    let candidate_shared: f64 = candidate
        .deep_weights
        .iter()
        .filter(|(name, _)| target.deep_weights.contains_key(*name))
        .map(|(_, weight)| weight)
        .sum();
    (target_shared / target_total).max(candidate_shared / candidate_total)
}

/// Detector D2: Interface & Signature Token Overlap.
fn find_best_d2(target: &SymbolData, corpus: &Corpus<'_>) -> Option<ReuseCandidate> {
    if target.tokens.len() < 2 {
        return None;
    }
    best_by_overlap(
        DetectorId::D2,
        target,
        corpus,
        PostingKind::TokenPair,
        1,
        |candidate| {
            if candidate.tokens.len() < 2 {
                return None;
            }
            // Similarity is measured on the symbol names when both sides have
            // name tokens. Signature tokens dominate the union otherwise, which
            // pushed every D2 candidate to ~100% and left the 45-75% judge band
            // permanently empty.
            let comparable = !target.name_tokens.is_empty() || !candidate.name_tokens.is_empty();
            let (shared, union_len): (Vec<String>, usize) = if comparable {
                (
                    target
                        .name_tokens
                        .intersection(&candidate.name_tokens)
                        .cloned()
                        .collect(),
                    target.name_tokens.union(&candidate.name_tokens).count(),
                )
            } else {
                (
                    target
                        .tokens
                        .intersection(&candidate.tokens)
                        .cloned()
                        .collect(),
                    target.tokens.union(&candidate.tokens).count(),
                )
            };
            if shared.len() < 2 {
                return None;
            }
            // Numbered siblings in one file (`find_best_d1` / `find_best_d2`,
            // `check_d3` / `check_d4`) are a family by design, not twins.
            if candidate.file_id == target.file_id
                && strip_trailing_digits(&target.name) == strip_trailing_digits(&candidate.name)
            {
                return None;
            }
            let jaccard = shared.len() as f64 / union_len as f64;
            let type_jaccard =
                if !target.type_tokens.is_empty() && !candidate.type_tokens.is_empty() {
                    let shared_types = target
                        .type_tokens
                        .intersection(&candidate.type_tokens)
                        .count();
                    let union_types = target.type_tokens.union(&candidate.type_tokens).count();
                    shared_types as f64 / union_types as f64
                } else {
                    0.0
                };
            if jaccard < 0.60 && !(jaccard >= 0.40 && type_jaccard >= 0.50) {
                return None;
            }
            Some(((jaccard * 100.0).round() as u64, shared))
        },
    )
}

/// Detector D3: Raw Client / Abstraction Bypass.
fn check_d3(target: &SymbolData, corpus: &Corpus<'_>) -> Option<ReuseCandidate> {
    let mut best_wrapper: Option<(&SymbolData, &str)> = None;
    for s in candidate_symbols(corpus, target, PostingKind::RawClient, 1) {
        let same_file_overlap = s.file_id == target.file_id
            && !(s.line_end < target.line_start || s.line_start > target.line_end);
        if s.id == target.id || same_file_overlap || !is_more_canonical(s, target) {
            continue;
        }
        let Some(shared_raw) = target
            .all_callees
            .iter()
            .find(|tc| is_raw_client(tc) && s.all_callees.iter().any(|sc| raw_client_eq(tc, sc)))
        else {
            continue;
        };
        let is_better = best_wrapper.is_none_or(|(bw, _)| {
            s.inbound_callers > bw.inbound_callers
                || (s.inbound_callers == bw.inbound_callers && is_more_canonical(s, bw))
        });
        if is_better {
            best_wrapper = Some((s, shared_raw.as_str()));
        }
    }
    let (canonical, raw_client) = best_wrapper?;
    if canonical.inbound_callers < 2 {
        return None;
    }

    Some(make_candidate(
        DetectorId::D3,
        target,
        canonical,
        90,
        &[format!("raw_client={raw_client}")],
    ))
}

/// Detector D4: Guarded Pass-Through Wrapper.
fn check_d4(target: &SymbolData, corpus: &Corpus<'_>) -> Option<ReuseCandidate> {
    let lines = target.line_end - target.line_start;
    if lines > 4
        || target.callee_ids.len() != 1
        || target.all_callees.len() != 1
        || target.inbound_callers > 1
    {
        return None;
    }

    let callee_id = *target.callee_ids.iter().next()?;
    let callee = corpus.symbols.get(&callee_id)?;
    if callee.id == target.id || callee.file_id == target.file_id {
        return None;
    }

    Some(make_candidate(
        DetectorId::D4,
        target,
        callee,
        100,
        std::slice::from_ref(&callee.name),
    ))
}

#[cfg(test)]
mod tests;
