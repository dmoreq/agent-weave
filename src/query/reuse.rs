//! Anti-reinvention detection engine (D1-D4).
//! Referencing RepoReuse (arXiv:2609.35357) and codebase-minimalism.
//! Sub-20ms calibrated static duplicate detectors.

mod corpus;
mod detectors;
mod index;
mod tokens;

use std::collections::HashSet;

use rusqlite::Connection;

use crate::error::Result;
use crate::query::{clamp_limit, QueryCtx};

use corpus::{load_callees, load_deep_weights, load_symbols, select_targets};
use detectors::{Corpus, DETECTORS};
use index::{CorpusIndex, INVERTED_INDEX_MIN_SYMBOLS};

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
    /// Semantic verdict from `eg-laya`, present only with `--judge`.
    pub verdict: Option<crate::query::judge::Verdict>,
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
        if let Some(verdict) = &self.verdict {
            line.push_str(&format!(
                "\n  judge ({}): {} p={:.2}",
                verdict.mode, verdict.decision, verdict.probability
            ));
        }
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
    let (candidates, _) = analyze_corpus(conn, None, Some(new_names), 3, false)?;
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
    deep_callee: bool,
    judge: bool,
) -> Result<crate::query::LineQuery> {
    let QueryCtx { conn, .. } = *ctx;
    let limit = clamp_limit(limit)?;
    let path_filter = if paths.is_empty() { None } else { Some(paths) };
    let (mut candidates, total) = analyze_corpus(conn, path_filter, None, limit, deep_callee)?;
    let mut stderr = String::new();

    // Arbitrate before the empty-result early return so `--judge` reports why it could not run
    // even when the static detectors found nothing.
    if judge {
        stderr = arbitrate(ctx, &mut candidates);
    }

    if candidates.is_empty() {
        crate::journal::record_result(crate::journal::OutcomeCounts::of(0, 0), false, 0);
        return Ok(crate::query::LineQuery {
            lines: vec!["0 reuse candidates found".to_string()],
            stderr,
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
        stderr,
        shown,
    })
}

/// Fill `verdict` for gray-zone candidates and return the one stderr note (if any).
fn arbitrate(ctx: &QueryCtx<'_>, candidates: &mut [ReuseCandidate]) -> String {
    use crate::query::judge;
    let setup = match judge::detect_setup() {
        Ok(setup) => setup,
        Err(note) => return format!("eg: judge: {note}\n"),
    };
    let mut judged = 0usize;
    let mut gray = 0usize;
    for candidate in candidates.iter_mut() {
        if !judge::is_gray(candidate.overlap_pct) {
            continue;
        }
        gray += 1;
        if judged >= judge::MAX_JUDGED {
            continue;
        }
        if let Some(verdict) = judge::arbitrate(&setup, ctx.conn, ctx.root, candidate) {
            candidate.verdict = Some(verdict);
            judged += 1;
        }
    }
    if gray > 0 && judged == 0 {
        return "eg: judge: eg-laya returned no verdict (timeout or model error)\n".to_string();
    }
    if gray > judged {
        return format!(
            "eg: judge: {} gray-zone candidate(s) left unjudged (cap {})\n",
            gray - judged,
            judge::MAX_JUDGED
        );
    }
    String::new()
}

pub(crate) fn analyze_corpus(
    conn: &Connection,
    path_filter: Option<&[String]>,
    target_names: Option<&HashSet<String>>,
    limit: usize,
    deep_callee: bool,
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
    if deep_callee {
        load_deep_weights(&mut symbols);
    }

    let targets = select_targets(&symbols, path_filter, target_names);
    let index = (symbols.len() > INVERTED_INDEX_MIN_SYMBOLS).then(|| CorpusIndex::build(&symbols));
    let corpus = Corpus {
        symbols: &symbols,
        index: index.as_ref(),
        deep_callee,
    };
    let mut candidates = Vec::new();
    let mut seen_pairs = HashSet::new();

    for target in &targets {
        for detect in DETECTORS {
            if let Some(c) = detect(target, &corpus) {
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
