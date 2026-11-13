//! Phase 2 pre-finalize proposal. A proposed signature and optional callee
//! list are matched against indexed production symbols without creating a
//! file. Candidates are derived only from resolved corpus data.

use std::collections::HashSet;

use super::corpus::{load_callees, load_symbols};
use super::tokens::{build_stoplist, tokenize_name_sig};
use super::*;

pub(super) fn analyze(conn: &Connection, opts: &ReuseOpts) -> Result<Vec<ReuseCandidate>> {
    let signature = opts
        .proposal_signature
        .as_deref()
        .ok_or_else(|| EgError::Usage("propose must be non-empty".to_string()))?;
    let name = proposal_name(signature)
        .ok_or_else(|| EgError::Usage("propose needs an identifier".to_string()))?;
    let (name_tokens, _type_tokens, all_tokens) = tokenize_name_sig(&name, signature);
    let proposed = callee_set(&opts.proposed_callees);

    let mut symbols = load_symbols(conn)?;
    if symbols.len() < 2 {
        return Ok(Vec::new());
    }
    load_callees(conn, &mut symbols)?;
    let stopset = build_stoplist(&symbols);
    for sym in symbols.values_mut() {
        sym.all_callees
            .retain(|c| !stopset.contains(c.as_str()) || is_raw_client(c.as_str()));
        sym.resolved_callees
            .retain(|c| !stopset.contains(c.as_str()));
    }

    let mut pool = Vec::new();
    for candidate in symbols.values() {
        if candidate.qualname == name {
            continue;
        }
        let shared_callees: Vec<String> = proposed
            .intersection(&candidate.all_callees)
            .cloned()
            .collect();
        let candidate_tokens = if candidate.name_tokens.is_empty() {
            &candidate.tokens
        } else {
            &candidate.name_tokens
        };
        let proposed_tokens = if name_tokens.is_empty() {
            &all_tokens
        } else {
            &name_tokens
        };
        let shared_tokens: Vec<String> = proposed_tokens
            .intersection(candidate_tokens)
            .cloned()
            .collect();
        let union = proposed_tokens.union(candidate_tokens).count();
        let token_overlap = if union == 0 {
            0.0
        } else {
            shared_tokens.len() as f64 / union as f64
        };

        let mut reasons = Vec::new();
        let (detector, pct, confidence, items) = if !proposed.is_empty() {
            let coverage = shared_callees.len() as f64 / proposed.len() as f64;
            let containment = if candidate.all_callees.is_empty() {
                0.0
            } else {
                shared_callees.len() as f64 / candidate.all_callees.len() as f64
            };
            if shared_callees.is_empty() || coverage.max(containment) < 0.50 {
                continue;
            }
            if coverage < 1.0 {
                reasons.push(ReasonCode::ProposalPartial);
            }
            let confidence = if coverage >= 0.95 && token_overlap >= 0.50 {
                ReuseConfidence::High
            } else if coverage >= 0.75 {
                ReuseConfidence::Medium
            } else {
                ReuseConfidence::Low
            };
            (
                DetectorId::D1,
                (coverage.max(containment) * 100.0).round() as u64,
                confidence,
                shared_callees,
            )
        } else {
            if shared_tokens.len() < 2 || token_overlap < 0.60 {
                continue;
            }
            reasons.push(ReasonCode::ProposalTokenOnly);
            let confidence = if token_overlap >= 0.80 {
                ReuseConfidence::Medium
            } else {
                ReuseConfidence::Low
            };
            (
                DetectorId::D2,
                (token_overlap * 100.0).round() as u64,
                confidence,
                shared_tokens,
            )
        };

        let mut items = items;
        items.sort();
        pool.push(ReuseCandidate {
            detector,
            symbol: name.clone(),
            file: "<proposal>".to_string(),
            line: 0,
            canonical: candidate.qualname.clone(),
            canonical_file: candidate.file_path.clone(),
            canonical_line: candidate.line_start,
            canonical_callers: candidate.inbound_callers,
            overlap_pct: Some(pct),
            shared_items: items,
            raw_client: None,
            confidence,
            reasons,
            freshness: Freshness::Fresh,
        });
    }

    pool.sort_by(|a, b| {
        b.confidence
            .rank()
            .cmp(&a.confidence.rank())
            .then_with(|| b.overlap_pct.cmp(&a.overlap_pct))
            .then_with(|| b.canonical_callers.cmp(&a.canonical_callers))
            .then_with(|| a.canonical.cmp(&b.canonical))
    });
    Ok(pool)
}

fn proposal_name(signature: &str) -> Option<String> {
    let trimmed = signature.trim();
    let reduced = ["def ", "fn ", "function ", "class ", "method "]
        .iter()
        .find_map(|prefix| trimmed.strip_prefix(prefix))
        .unwrap_or(trimmed);
    let name = reduced
        .split(['(', '<', ' ', ':'])
        .next()
        .unwrap_or("")
        .trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn callee_set(values: &[String]) -> HashSet<String> {
    let mut out = HashSet::new();
    for value in values {
        for part in value.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            out.insert(part.to_string());
            if let Some(last) = part.rsplit(['.', ':']).next() {
                out.insert(last.to_string());
            }
        }
    }
    out
}
