//! Detectors D1-D4: callee-footprint containment, interface token overlap,
//! raw-client abstraction bypass, and guarded pass-through wrappers.

use super::corpus::SymbolData;
use super::*;
use std::collections::HashMap;

/// Raw external clients targeted by D3 abstraction bypass.
const RAW_CLIENTS: &[&str] = &[
    "Connection::open",
    "sqlite3_open",
    "requests.get",
    "requests.post",
    "axios",
    "axios.get",
    "axios.post",
    "fetch",
    "reqwest::get",
    "ureq::get",
];

/// One reuse detector: resolve a target against the corpus to a candidate.
pub(super) type Detector = fn(&SymbolData, &HashMap<i64, SymbolData>) -> Option<ReuseCandidate>;

/// The detector cascade in priority order; a target stops at the first
/// detector that produces a candidate.
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

/// The identity block every detector builds from a matched (target,
/// candidate) pair; detectors only differ in the overlap signal above it.
/// `shared` is sorted into the candidate's display items.
fn make_candidate(
    detector: &'static str,
    target: &SymbolData,
    candidate: &SymbolData,
    overlap_pct: u64,
    shared: &[String],
) -> ReuseCandidate {
    let mut shared_items: Vec<String> = shared.to_vec();
    shared_items.sort();
    ReuseCandidate {
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
    detector: &'static str,
    target: &SymbolData,
    symbols: &HashMap<i64, SymbolData>,
    signal: impl Fn(&SymbolData) -> Option<(u64, Vec<String>)>,
) -> Option<ReuseCandidate> {
    let mut best: Option<(u64, &SymbolData, Vec<String>)> = None;
    for candidate in symbols.values() {
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
fn find_best_d1(target: &SymbolData, symbols: &HashMap<i64, SymbolData>) -> Option<ReuseCandidate> {
    if target.all_callees.len() < 3 || target.resolved_callees.len() < 2 {
        return None;
    }
    best_by_overlap("D1", target, symbols, |candidate| {
        let c_callees = &candidate.all_callees;
        if c_callees.len() < 3 || candidate.resolved_callees.len() < 2 {
            return None;
        }
        if target.callee_ids.contains(&candidate.id)
            || candidate.callee_ids.contains(&target.id)
            || target.all_callees.contains(&candidate.name)
            || candidate.all_callees.contains(&target.name)
        {
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
    })
}

/// Detector D2: Interface & Signature Token Overlap.
fn find_best_d2(target: &SymbolData, symbols: &HashMap<i64, SymbolData>) -> Option<ReuseCandidate> {
    if target.tokens.len() < 2 {
        return None;
    }
    best_by_overlap("D2", target, symbols, |candidate| {
        if candidate.tokens.len() < 2 {
            return None;
        }
        let shared: Vec<String> = target
            .tokens
            .intersection(&candidate.tokens)
            .cloned()
            .collect();
        if shared.len() < 2 {
            return None;
        }
        let union_len = target.tokens.union(&candidate.tokens).count();
        let jaccard = shared.len() as f64 / union_len as f64;
        let type_jaccard = if !target.type_tokens.is_empty() && !candidate.type_tokens.is_empty() {
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
    })
}

/// Detector D3: Raw Client / Abstraction Bypass.
fn check_d3(target: &SymbolData, symbols: &HashMap<i64, SymbolData>) -> Option<ReuseCandidate> {
    let raw_client = target
        .all_callees
        .iter()
        .find(|c| RAW_CLIENTS.iter().any(|r| r.contains(c.as_str())))?;

    let mut best_wrapper: Option<&SymbolData> = None;
    for s in symbols.values() {
        let same_file_overlap = s.file_id == target.file_id
            && !(s.line_end < target.line_start || s.line_start > target.line_end);
        if s.id != target.id
            && !same_file_overlap
            && is_more_canonical(s, target)
            && s.all_callees
                .iter()
                .any(|c| RAW_CLIENTS.iter().any(|r| r.contains(c.as_str())))
            // A caller-count tie breaks by the canonical key so the winner
            // does not depend on HashMap iteration order.
            && best_wrapper.is_none_or(|bw| {
                s.inbound_callers > bw.inbound_callers
                    || (s.inbound_callers == bw.inbound_callers && is_more_canonical(s, bw))
            })
        {
            best_wrapper = Some(s);
        }
    }
    let canonical = best_wrapper?;
    if canonical.inbound_callers < 2 {
        return None;
    }

    Some(make_candidate(
        "D3",
        target,
        canonical,
        90,
        &[format!("raw_client={raw_client}")],
    ))
}

/// Detector D4: Guarded Pass-Through Wrapper.
fn check_d4(target: &SymbolData, symbols: &HashMap<i64, SymbolData>) -> Option<ReuseCandidate> {
    let lines = target.line_end - target.line_start;
    if lines > 4
        || target.callee_ids.len() != 1
        || target.all_callees.len() != 1
        || target.inbound_callers > 1
    {
        return None;
    }

    let callee_id = *target.callee_ids.iter().next()?;
    let callee = symbols.get(&callee_id)?;
    if callee.id == target.id || callee.file_id == target.file_id {
        return None;
    }

    Some(make_candidate(
        "D4",
        target,
        callee,
        100,
        std::slice::from_ref(&callee.name),
    ))
}
