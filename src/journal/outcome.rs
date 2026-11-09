//! Outcome counters the journal records for one invocation: how many rows the
//! tool produced, how many were displayed, and how many existed in total.

use super::*;

/// The three same-typed outcome counters `record_result` journals. Named
/// fields keep `result_count`/`shown`/`total` from being swapped silently at
/// the call sites.
pub struct OutcomeCounts {
    pub result_count: usize,
    pub shown: usize,
    pub total: usize,
}

impl OutcomeCounts {
    /// One count for all three counters: the non-paginated commands that
    /// journal no truncation and no filtering.
    pub fn uniform(n: usize) -> Self {
        Self {
            result_count: n,
            shown: n,
            total: n,
        }
    }

    /// The paginated pointer queries: the journal count is the displayed set,
    /// so `result_count` equals `shown` and only `total` can differ.
    pub fn of(shown: usize, total: usize) -> Self {
        Self {
            result_count: shown,
            shown,
            total,
        }
    }
}

/// Handlers inject the query outcome before the process exits. No-op when the
/// journal was not armed (disabled, or a non-journaled subcommand).
pub fn record_result(counts: OutcomeCounts, stale: bool, ambiguous: usize) {
    PENDING.with(|p| {
        if let Some(pending) = p.borrow_mut().as_mut() {
            pending.result_count = counts.result_count;
            pending.shown = counts.shown;
            pending.total = counts.total;
            pending.stale = stale;
            pending.ambiguous = ambiguous;
        }
    });
}
