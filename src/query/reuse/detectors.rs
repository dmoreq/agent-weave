//! Detectors D1-D5, D1L, DT: duplicate, bypass, wrapper, and coverage signals.

use super::candidate::{make_candidate, MatchMeta};
use super::confidence::distinctive_ratio;
use super::corpus::SymbolData;
use super::guards::*;
use super::index::{CorpusIndex, PostingKind};
use super::*;
use std::cell::Cell;
use std::collections::HashMap;

/// Immutable corpus view: symbols, optional index, and deep-callee switch.
pub(super) struct Corpus<'a> {
    pub(super) symbols: &'a HashMap<i64, SymbolData>,
    pub(super) index: Option<&'a CorpusIndex>,
    /// Distinct callers per resolved destination. `None` in detector unit
    /// tests, where every resolved shared callee counts as distinctive.
    pub(super) df_all_excl: Option<&'a HashMap<i64, i64>>,
    pub(super) deep_callee: bool,
    /// Explicit guard rejections seen while matching pairs. The scan reports
    /// this as `guarded_out`; it is not part of the candidate pool.
    pub(super) guarded_out: Cell<usize>,
}

impl Corpus<'_> {
    fn guard(&self) {
        self.guarded_out.set(self.guarded_out.get() + 1);
    }
}

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

/// Superset of candidates sharing the signal; the detector rechecks exactly.
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

/// Closed detector set; `as_str` is the stable journal/dashboard spelling.
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
    /// Duplicate class shape (Phase 4 coverage).
    D5,
    /// Unresolved bare-name call with an indexed canonical (Phase 4 coverage).
    D1L,
    /// Test-helper overlap (Phase 4 coverage).
    DT,
}

impl DetectorId {
    /// The stable wire spelling used in journal hints and dashboard JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            DetectorId::D1 => "D1",
            DetectorId::D2 => "D2",
            DetectorId::D3 => "D3",
            DetectorId::D4 => "D4",
            DetectorId::D5 => "D5",
            DetectorId::D1L => "D1L",
            DetectorId::DT => "DT",
        }
    }
}

pub(super) const DETECTORS: &[Detector] = &[find_best_d1, find_best_d2, check_d3, check_d4];

/// Shared D1/D2 scan frame: drop self/overlap/non-canonical candidates and
/// keep the best signal with a deterministic canonical tie-break.
fn best_by_overlap(
    detector: DetectorId,
    target: &SymbolData,
    corpus: &Corpus<'_>,
    kind: PostingKind,
    min_shared: usize,
    signal: impl Fn(&SymbolData) -> Option<(u64, Vec<String>, ReuseConfidence, Vec<ReasonCode>)>,
) -> Option<ReuseCandidate> {
    type BestMatch<'a> = (
        u64,
        &'a SymbolData,
        Vec<String>,
        ReuseConfidence,
        Vec<ReasonCode>,
    );
    let mut best: Option<BestMatch<'_>> = None;
    for candidate in candidate_symbols(corpus, target, kind, min_shared) {
        if candidate.id == target.id {
            corpus.guard();
            continue;
        }
        // If in the same file, drop enclosing/overlapping spans (e.g. methods in class or nested fns)
        if candidate.file_id == target.file_id
            && !(candidate.line_end < target.line_start || candidate.line_start > target.line_end)
        {
            corpus.guard();
            continue;
        }
        if cross_language(target, candidate) {
            corpus.guard();
            continue;
        }
        if !is_more_canonical(candidate, target) {
            corpus.guard();
            continue;
        }
        let Some((pct, shared, confidence, reasons)) = signal(candidate) else {
            continue;
        };
        let better = match &best {
            None => true,
            Some((best_pct, best_sym, _, _, _)) => {
                pct > *best_pct || (pct == *best_pct && is_more_canonical(candidate, best_sym))
            }
        };
        if better {
            best = Some((pct, candidate, shared, confidence, reasons));
        }
    }
    best.map(|(pct, candidate, shared, confidence, reasons)| {
        make_candidate(
            detector,
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

/// D1: callee-footprint containment over resolved project callees.
fn d1_footprint(symbol: &SymbolData) -> &std::collections::HashSet<String> {
    if symbol.resolved_callees.is_empty() {
        &symbol.all_callees
    } else {
        &symbol.resolved_callees
    }
}

fn find_best_d1(target: &SymbolData, corpus: &Corpus<'_>) -> Option<ReuseCandidate> {
    if corpus.deep_callee {
        if target.deep_weights.len() < 3 {
            return None;
        }
    } else if d1_footprint(target).len() < 3 || target.resolved_callees.len() < 2 {
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
            let c_callees = d1_footprint(candidate);
            // GUARD-COMP: a resolved call edge in either direction is a
            // caller/callee composition, not duplication.
            if target.callee_ids.contains(&candidate.id)
                || candidate.callee_ids.contains(&target.id)
            {
                corpus.guard();
                return None;
            }
            let unresolved_composition = target.all_callees.contains(&candidate.name)
                || candidate.all_callees.contains(&target.name);
            let zero_name = target
                .name_tokens
                .intersection(&candidate.name_tokens)
                .next()
                .is_none();
            if corpus.deep_callee {
                if candidate.deep_weights.len() < 3 {
                    return None;
                }
                let ratio = weighted_containment(target, candidate);
                if ratio < 0.85 {
                    return None;
                }
                let shared: Vec<String> = d1_footprint(target)
                    .intersection(c_callees)
                    .cloned()
                    .collect();
                let pct = (ratio * 100.0).round() as u64;
                let mut reasons = Vec::new();
                if unresolved_composition {
                    reasons.push(ReasonCode::CallerCallee);
                }
                if zero_name {
                    reasons.push(ReasonCode::ZeroName);
                }
                let weak = distinctive_ratio(target, candidate, corpus, &shared);
                let confidence = if !reasons.is_empty() || weak > 0.75 {
                    ReuseConfidence::Low
                } else if ratio >= 0.95 && weak <= 0.25 {
                    ReuseConfidence::High
                } else if ratio >= 0.90 {
                    ReuseConfidence::Medium
                } else {
                    ReuseConfidence::Provisional
                };
                return Some((pct, shared, confidence, reasons));
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
            let shared: Vec<String> = d1_footprint(target)
                .intersection(c_callees)
                .cloned()
                .collect();
            // GUARD-BAL: reject < 0.30, low in [0.30, 0.50), provisional >= 0.50.
            let target_fp = d1_footprint(target);
            let min_len = target_fp.len().min(c_callees.len());
            let max_len = target_fp.len().max(c_callees.len());
            let bal = min_len as f64 / max_len as f64;
            if bal < 0.30 {
                corpus.guard();
                return None;
            }
            let ratio_candidate = shared.len() as f64 / c_callees.len() as f64;
            let ratio_target = shared.len() as f64 / target_fp.len() as f64;
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
            let weak = distinctive_ratio(target, candidate, corpus, &shared);
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

/// Detector D2: Interface & Signature Token Overlap.
fn find_best_d2(target: &SymbolData, corpus: &Corpus<'_>) -> Option<ReuseCandidate> {
    if target.name_tokens.len() < 2 {
        return None;
    }
    best_by_overlap(
        DetectorId::D2,
        target,
        corpus,
        PostingKind::TokenPair,
        1,
        |candidate| {
            if candidate.tokens.len() < 2
                || target.name_tokens.is_empty()
                || candidate.name_tokens.is_empty()
            {
                return None;
            }
            // GUARD-COMP: resolved caller/callee edges reject. A bare-name-only
            // overlap is still a composition hint, so keep it at Low.
            if target.callee_ids.contains(&candidate.id)
                || candidate.callee_ids.contains(&target.id)
            {
                corpus.guard();
                return None;
            }
            let unresolved_composition = target.all_callees.contains(&candidate.name)
                || candidate.all_callees.contains(&target.name);
            let shared: Vec<String> = target
                .name_tokens
                .intersection(&candidate.name_tokens)
                .cloned()
                .collect();
            let union_len = target.name_tokens.union(&candidate.name_tokens).count();
            if shared.len() < 2 {
                return None;
            }
            let strict_subset = target.name_tokens.len() != candidate.name_tokens.len()
                && (target.name_tokens.is_subset(&candidate.name_tokens)
                    || candidate.name_tokens.is_subset(&target.name_tokens));
            let shared_callees = target
                .resolved_callees
                .intersection(&candidate.resolved_callees)
                .count();
            if strict_subset && shared_callees == 0 {
                corpus.guard();
                return None;
            }
            // Numbered siblings in one file (`find_best_d1` / `find_best_d2`,
            // `check_d3` / `check_d4`) are a family by design, not twins.
            if candidate.file_id == target.file_id
                && strip_trailing_digits(&target.name) == strip_trailing_digits(&candidate.name)
            {
                corpus.guard();
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
            let mut reasons = Vec::new();
            if unresolved_composition {
                reasons.push(ReasonCode::CallerCallee);
            }
            if strict_subset {
                reasons.push(ReasonCode::SameNameSubset);
            }
            let confidence = if !reasons.is_empty() || shared.len() < 3 {
                ReuseConfidence::Low
            } else if shared.len() >= 4 && jaccard >= 0.80 && type_jaccard >= 0.50 {
                ReuseConfidence::High
            } else if jaccard >= 0.75 {
                ReuseConfidence::Medium
            } else {
                ReuseConfidence::Provisional
            };
            Some((
                (jaccard * 100.0).round() as u64,
                shared,
                confidence,
                reasons,
            ))
        },
    )
}

/// Detector D3: Raw Client / Abstraction Bypass.
fn check_d3(target: &SymbolData, corpus: &Corpus<'_>) -> Option<ReuseCandidate> {
    let mut best_wrapper: Option<(&SymbolData, &str)> = None;
    for s in candidate_symbols(corpus, target, PostingKind::RawClient, 1) {
        let same_file_overlap = s.file_id == target.file_id
            && !(s.line_end < target.line_start || s.line_start > target.line_end);
        if s.id == target.id || same_file_overlap {
            corpus.guard();
            continue;
        }
        if cross_language(target, s) {
            corpus.guard();
            continue;
        }
        if !is_more_canonical(s, target) {
            corpus.guard();
            continue;
        }
        let mut shared_raw: Vec<&String> = target
            .all_callees
            .iter()
            .filter(|tc| is_raw_client(tc) && s.all_callees.iter().any(|sc| raw_client_eq(tc, sc)))
            .collect();
        shared_raw.sort();
        let Some(shared_raw) = shared_raw.first() else {
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
        None,
        &[],
        MatchMeta {
            raw_client: Some(raw_client.to_string()),
            // D3 canonical semantics are not precise enough for High until a
            // labeled gate proves otherwise (self-scan produced 0/4 precision).
            confidence: ReuseConfidence::Medium,
            reasons: Vec::new(),
        },
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
        corpus.guard();
        return None;
    }
    if cross_language(target, callee) {
        corpus.guard();
        return None;
    }

    Some(make_candidate(
        DetectorId::D4,
        target,
        callee,
        None,
        &[],
        MatchMeta {
            raw_client: None,
            confidence: ReuseConfidence::Medium,
            reasons: Vec::new(),
        },
    ))
}

#[cfg(test)]
mod tests;
