//! Ranked-memory model: candidate scoring features, min-max fusion, and
//! row rendering for mem query output.

/// Fusion weights for the three ranked-candidate features (lexical token
/// overlap, ref-token overlap, bounded recency decay). Tuning constants of
/// `fuse` alone, private to this module.
const W_LEX: f64 = 0.5;
const W_REF: f64 = 0.3;
const W_REC: f64 = 0.2;
/// Weight of the call-graph expansion signal. Adding a fourth feature leaves
/// the lexical/ref/recency ordering untouched when no graph candidate exists:
/// an all-zero feature min-max-normalizes to the design's all-equal 1.0, which
/// is a constant score offset for every row.
const W_GRAPH: f64 = 0.25;

/// One candidate row with its raw signals. `lex_raw` is the FTS5 bm25 rank
/// (smaller is better); `ref_raw` is the token-overlap fraction in [0,1];
/// `graph_raw` is the call-graph decay `0.5^depth` (0 when not reachable);
/// `age_days` feeds the bounded recency decay.
pub struct Candidate {
    pub row: crate::db::MemRow,
    pub lex_raw: f64,
    pub ref_raw: f64,
    pub graph_raw: f64,
    pub age_days: f64,
    /// Fused score, filled by [`fuse`].
    pub score: f64,
    pub is_global: bool,
    /// True when the FTS index produced this candidate (the only direct
    /// evidence that the query lexically matched the note).
    pub from_fts: bool,
}

/// Min-max normalize with the design's all-equal convention: an all-equal
/// signal maps every row to 1.0 (zeromem convention; order-invariant).
fn minmax(values: &[f64], invert: bool) -> Vec<f64> {
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    if !min.is_finite() || !max.is_finite() || (max - min).abs() < f64::EPSILON {
        return vec![1.0; values.len()];
    }
    values
        .iter()
        .map(|v| {
            let n = (v - min) / (max - min);
            if invert {
                1.0 - n
            } else {
                n
            }
        })
        .collect()
}

/// Fuse the candidates: invert bm25 (smaller is better), min-max both
/// bounded signals, apply the recency decay, weight, then sort by
/// (score desc, created_at desc, key asc, rev desc). Deterministic: same
/// rows + same query + same binary -> same order.
pub fn fuse(mut candidates: Vec<Candidate>) -> Vec<Candidate> {
    let lex = minmax(
        &candidates.iter().map(|c| c.lex_raw).collect::<Vec<_>>(),
        true,
    );
    let reference = minmax(
        &candidates.iter().map(|c| c.ref_raw).collect::<Vec<_>>(),
        false,
    );
    let graph = minmax(
        &candidates.iter().map(|c| c.graph_raw).collect::<Vec<_>>(),
        false,
    );
    for (i, c) in candidates.iter_mut().enumerate() {
        let rec = 1.0 / (1.0 + c.age_days);
        c.score = W_LEX * lex[i] + W_REF * reference[i] + W_REC * rec + W_GRAPH * graph[i];
    }
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.row.created_at.cmp(&a.row.created_at))
            .then(a.row.key.cmp(&b.row.key))
            .then(b.row.rev.cmp(&a.row.rev))
    });
    candidates
}

// ---------------------------------------------------------------------------
// Rendering (shared bytes for CLI and MCP).
// ---------------------------------------------------------------------------

/// One note row header line: `key rev kind scope created_at`.
fn render_row_header(row: &crate::db::MemRow) -> String {
    format!(
        "{} r{} {} {} {}",
        row.key, row.rev, row.kind, row.scope, row.created_at
    )
}

/// Render a row with a trailing header tag (the `--include-superseded`
/// listing marks replaced revisions with ` [superseded]`).
pub fn render_row_tagged(row: &crate::db::MemRow, tag: &str) -> String {
    let mut out = render_row_header(row);
    out.push_str(tag);
    for line in row.body.lines() {
        out.push_str(&format!("\n  {line}"));
    }
    out
}
