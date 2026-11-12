//! Candidate assembly shared by D1-D4: the identity block and its metadata.

use super::corpus::SymbolData;
use super::*;

/// Confidence plus the detector-specific evidence outside the shared frame.
pub(super) struct MatchMeta {
    pub(super) raw_client: Option<String>,
    pub(super) confidence: ReuseConfidence,
    pub(super) reasons: Vec<ReasonCode>,
}

/// The identity block every detector builds from a matched (target,
/// candidate) pair; detectors only differ in the overlap signal above it.
/// `shared` is sorted into the candidate's display items.
pub(super) fn make_candidate(
    detector: DetectorId,
    target: &SymbolData,
    candidate: &SymbolData,
    overlap_pct: Option<u64>,
    shared: &[String],
    meta: MatchMeta,
) -> ReuseCandidate {
    let MatchMeta {
        raw_client,
        confidence,
        mut reasons,
    } = meta;
    let mut shared_items: Vec<String> = shared.to_vec();
    shared_items.sort();
    reasons.sort_by_key(|r| r.as_str());
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
        raw_client,
        confidence,
        reasons,
        freshness: Freshness::Fresh,
    }
}
