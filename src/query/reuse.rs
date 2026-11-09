//! Anti-reinvention detection engine (D1-D4).
//! Referencing RepoReuse (arXiv:2609.35357) and codebase-minimalism.
//! Sub-20ms calibrated static duplicate detectors.

mod corpus;
mod detectors;
mod tokens;

use std::collections::HashSet;

use rusqlite::Connection;

use crate::error::Result;
use crate::query::{clamp_limit, QueryCtx};

use corpus::{load_callees, load_symbols, select_targets};
use detectors::DETECTORS;

pub(crate) use detectors::is_raw_client;
pub use detectors::DetectorId;
use tokens::build_stoplist;

#[derive(Debug, Clone)]
pub struct ReuseCandidate {
    pub detector: DetectorId,
    pub symbol: String,
    pub file: String,
    pub line: i64,
    pub canonical: String,
    pub canonical_file: String,
    pub canonical_line: i64,
    pub canonical_callers: i64,
    pub overlap_pct: u64,
    pub shared_items: Vec<String>,
}

impl ReuseCandidate {
    pub fn hint_line(&self) -> String {
        // The caller tail every detector renders the same way.
        let tail = format!(
            "([callers={}] in {}:{})",
            self.canonical_callers, self.canonical_file, self.canonical_line
        );
        match self.detector {
            // D1/D2 share the overlap template and differ only in the noun.
            DetectorId::D1 | DetectorId::D2 => format!(
                "symbol '{}' shares {}% {} with existing '{}' {}",
                self.symbol,
                self.overlap_pct,
                if self.detector == DetectorId::D1 {
                    "callee footprint"
                } else {
                    "interface token overlap"
                },
                self.canonical,
                tail
            ),
            DetectorId::D3 => format!(
                "symbol '{}' bypasses canonical client '{}' to call raw external client {}",
                self.symbol, self.canonical, tail
            ),
            DetectorId::D4 => format!(
                "symbol '{}' is a pass-through wrapper around existing '{}' {}",
                self.symbol, self.canonical, tail
            ),
        }
    }

    pub fn format_text(&self) -> String {
        let mut line = self.hint_line();
        if !self.shared_items.is_empty() {
            let label = match self.detector {
                DetectorId::D1 => "shared callees",
                DetectorId::D2 => "shared tokens",
                DetectorId::D3 => "client bypass",
                DetectorId::D4 => "details",
            };
            line.push_str(&format!(
                "\n  {}: [{}]",
                label,
                self.shared_items.join(", ")
            ));
        }
        line
    }
}

/// Detect up to 3 non-failing advisory lines for modified symbols during incremental build.
pub fn detect_hints(conn: &Connection, new_names: &HashSet<String>) -> Result<Vec<String>> {
    if new_names.is_empty() {
        return Ok(Vec::new());
    }
    let (candidates, _) = analyze_corpus(conn, None, Some(new_names), 3)?;
    let mut hints = Vec::new();
    for c in candidates.iter().take(3) {
        hints.push(format!("eg: reuse-hint: {}", c.hint_line()));
    }
    Ok(hints)
}

/// Run `eg reuse` across the whole repository or specified paths.
pub fn run_reuse(
    ctx: &QueryCtx<'_>,
    paths: &[String],
    limit: usize,
) -> Result<crate::query::LineQuery> {
    let QueryCtx { conn, .. } = *ctx;
    let limit = clamp_limit(limit)?;
    let path_filter = if paths.is_empty() { None } else { Some(paths) };
    let (candidates, total) = analyze_corpus(conn, path_filter, None, limit)?;

    if candidates.is_empty() {
        crate::journal::record_result(crate::journal::OutcomeCounts::of(0, 0), false, 0);
        return Ok(crate::query::LineQuery {
            lines: vec!["0 reuse candidates found".to_string()],
            stderr: String::new(),
            shown: 0,
        });
    }

    let mut out = vec![format!(
        "eg: reuse — {} duplication candidate(s) found",
        candidates.len()
    )];
    for c in &candidates {
        for l in c.format_text().lines() {
            out.push(l.to_string());
        }
    }
    let shown = candidates.len();
    crate::journal::record_result(crate::journal::OutcomeCounts::of(shown, total), false, 0);
    Ok(crate::query::LineQuery {
        lines: out,
        stderr: String::new(),
        shown,
    })
}

pub(crate) fn analyze_corpus(
    conn: &Connection,
    path_filter: Option<&[String]>,
    target_names: Option<&HashSet<String>>,
    limit: usize,
) -> Result<(Vec<ReuseCandidate>, usize)> {
    let mut symbols = load_symbols(conn)?;
    if symbols.len() < 2 {
        return Ok((Vec::new(), 0));
    }

    load_callees(conn, &mut symbols)?;
    let stopset = build_stoplist(&symbols);
    for sym in symbols.values_mut() {
        sym.all_callees
            .retain(|c| !stopset.contains(c.as_str()) || is_raw_client(c.as_str()));
        sym.resolved_callees
            .retain(|c| !stopset.contains(c.as_str()));
    }

    let targets = select_targets(&symbols, path_filter, target_names);
    let mut candidates = Vec::new();
    let mut seen_pairs = HashSet::new();

    for target in &targets {
        for detect in DETECTORS {
            if let Some(c) = detect(target, &symbols) {
                accept_candidate(c, &mut seen_pairs, &mut candidates);
                break;
            }
        }
    }

    candidates.sort_by(|a, b| {
        b.overlap_pct
            .cmp(&a.overlap_pct)
            .then_with(|| b.canonical_callers.cmp(&a.canonical_callers))
            .then_with(|| a.symbol.cmp(&b.symbol))
    });
    let total = candidates.len();
    candidates.truncate(limit);
    Ok((candidates, total))
}

/// Dedup the (symbol, canonical) pair and push. A repeated pair is dropped,
/// but the target still stops at its first detector hit.
fn accept_candidate(
    candidate: ReuseCandidate,
    seen_pairs: &mut HashSet<(String, String)>,
    out: &mut Vec<ReuseCandidate>,
) {
    if seen_pairs.insert((candidate.symbol.clone(), candidate.canonical.clone())) {
        out.push(candidate);
    }
}
