//! Finalize a reuse report: confidence filter, sort/limit, 30 KiB cap,
//! rendering, and journal counters. Analysis owns the pool and freshness;
//! this module owns everything the user sees.

use super::*;

pub(super) fn finalize(
    _ctx: &QueryCtx<'_>,
    mut report: ReuseReport,
    opts: &ReuseOpts,
) -> Result<crate::query::LineQuery> {
    let freshness_headers = report.freshness.headers();
    let below_threshold = report
        .pool
        .iter()
        .filter(|c| c.confidence.rank() < opts.min_confidence.rank())
        .count();
    report
        .pool
        .retain(|c| c.confidence.rank() >= opts.min_confidence.rank());
    let eligible = report.pool.len();
    let limit_omitted = eligible.saturating_sub(opts.limit);
    report.pool.truncate(opts.limit);

    if report.pool.is_empty() {
        crate::journal::record_result(
            crate::journal::OutcomeCounts::of(0, report.counts.total),
            false,
            0,
        );
        let mut lines = vec!["eg: reuse — 0 duplication candidate(s) found".to_string()];
        lines.extend(freshness_headers);
        if below_threshold > 0 || report.counts.guarded_out > 0 || report.counts.suppressed > 0 {
            let mut summary = format!(
                "eg: reuse: 0 provisional, 0 low, 0 medium, 0 high, {} guarded out, {below_threshold} below threshold",
                report.counts.guarded_out
            );
            if report.counts.suppressed > 0 {
                summary.push_str(&format!("; {} suppressed", report.counts.suppressed));
            }
            lines.push(summary);
        }
        if !opts.paths.is_empty() {
            lines.push(format!("eg: reuse: path_matches={}", report.path_matches));
        }
        let mut query = crate::query::LineQuery::new(lines, report.stderr, 0);
        query.mode = report.mode;
        return Ok(query);
    }

    let mut blocks: Vec<(&ReuseCandidate, String)> = report
        .pool
        .iter()
        .map(|c| (c, c.format_text(opts.explain)))
        .collect();
    const OUTPUT_CAP: usize = 30 * 1024;
    let mut emitted: Vec<(&ReuseCandidate, String)> = Vec::new();
    let mut bytes = 0usize;
    let mut capped = 0usize;
    for (candidate, block) in blocks.drain(..) {
        let cost = block.len() + 1;
        if bytes + cost > OUTPUT_CAP {
            capped += 1;
            continue;
        }
        bytes += cost;
        emitted.push((candidate, block));
    }
    let shown = emitted.len();
    let count_level = |level: ReuseConfidence| {
        emitted
            .iter()
            .filter(|(c, _)| c.confidence == level)
            .count()
    };
    let provisional = count_level(ReuseConfidence::Provisional);
    let low = count_level(ReuseConfidence::Low);
    let medium = count_level(ReuseConfidence::Medium);
    let high = count_level(ReuseConfidence::High);
    let omitted = limit_omitted + capped;
    report.counts = ReuseCounts {
        shown_provisional: provisional,
        shown_low: low,
        shown_medium: medium,
        shown_high: high,
        suppressed: report.counts.suppressed,
        guarded_out: report.counts.guarded_out,
        below_threshold,
        omitted,
        capped,
        shown,
        total: report.counts.total,
        truncated: omitted > 0,
    };
    crate::journal::set_mode(report.mode);
    crate::journal::set_reuse_counts(crate::journal::ReuseJournalCounts {
        provisional: report.counts.shown_provisional,
        low: report.counts.shown_low,
        medium: report.counts.shown_medium,
        high: report.counts.shown_high,
        suppressed: report.counts.suppressed,
        guarded_out: report.counts.guarded_out,
        below_threshold: report.counts.below_threshold,
        omitted: report.counts.omitted,
        capped: report.counts.capped,
        truncated: report.counts.truncated,
    });
    let mut summary = format!(
        "eg: reuse: {} provisional, {} low, {} medium, {} high, {} guarded out, {} below threshold",
        report.counts.shown_provisional,
        report.counts.shown_low,
        report.counts.shown_medium,
        report.counts.shown_high,
        report.counts.guarded_out,
        report.counts.below_threshold
    );
    if report.counts.omitted > 0 {
        summary.push_str(&format!("; {} omitted", report.counts.omitted));
    }
    if report.counts.capped > 0 {
        summary.push_str(&format!("; {} capped", report.counts.capped));
    }
    if report.counts.suppressed > 0 {
        summary.push_str(&format!("; {} suppressed", report.counts.suppressed));
    }
    if report.counts.omitted > 0 {
        summary.push_str(&format!(
            "; showing {} of {}",
            report.counts.shown, report.counts.total
        ));
    }
    if !opts.paths.is_empty() {
        summary.push_str(&format!("; path_matches={}", report.path_matches));
    }
    let mut out = vec![
        format!(
            "eg: reuse — {} duplication candidate(s) found",
            report.counts.shown
        ),
        summary,
    ];
    out.extend(freshness_headers);
    for (_, block) in &emitted {
        for line in block.lines() {
            out.push(line.to_string());
        }
    }
    if report.counts.capped > 0 {
        out.push("eg: output capped at 30 KiB; narrow the path or raise the limit".to_string());
    }
    let surfaced: Vec<String> = emitted
        .iter()
        .take(25)
        .map(|(c, _)| format!("{}->{}", sanitize(&c.symbol), sanitize(&c.canonical)))
        .collect();
    crate::journal::set_candidate_ids(&surfaced.join("\n"));
    crate::journal::record_result(
        crate::journal::OutcomeCounts::of(report.counts.shown, report.counts.total),
        false,
        0,
    );
    let mut query = crate::query::LineQuery::new(out, report.stderr, report.counts.shown);
    query.mode = report.mode;
    query.truncated = report.counts.truncated;
    Ok(query)
}
