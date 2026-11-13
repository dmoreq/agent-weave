//! Reuse-specific journal fields: typed count partition and candidate
//! identities. Kept separate from `journal.rs` to stay under the 500-line cap.

use serde_json::Value;

/// Count partition written to the usage line for a reuse scan.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReuseJournalCounts {
    pub provisional: usize,
    pub low: usize,
    pub medium: usize,
    pub high: usize,
    pub suppressed: usize,
    pub guarded_out: usize,
    pub below_threshold: usize,
    pub omitted: usize,
    pub capped: usize,
    pub truncated: bool,
}

/// Record which candidates this invocation surfaced, so the dashboard can
/// count per-candidate usage from the retained journal.
pub fn set_candidate_ids(list: &str) {
    super::PENDING.with(|pending| {
        if let Some(entry) = pending.borrow_mut().as_mut() {
            entry
                .args
                .insert("candidates".to_string(), Value::String(list.to_string()));
        }
    });
}

#[derive(Debug, Clone, Copy)]
pub struct ReuseFreshness {
    pub state: &'static str,
    pub changed_files: usize,
    pub checked_files: usize,
    pub unknown_count: usize,
    pub unknown_reason: Option<&'static str>,
}

pub fn set_freshness(freshness: ReuseFreshness) {
    super::PENDING.with(|pending| {
        if let Some(entry) = pending.borrow_mut().as_mut() {
            entry.freshness = Some(serde_json::json!({
                "state": freshness.state,
                "changed_files": freshness.changed_files,
                "checked_files": freshness.checked_files,
                "unknown_count": freshness.unknown_count,
                "unknown_reason": freshness.unknown_reason,
            }));
        }
    });
}

/// Record which reuse surface produced the line.
pub fn set_mode(mode: crate::query::reuse::ReuseMode) {
    super::PENDING.with(|pending| {
        if let Some(entry) = pending.borrow_mut().as_mut() {
            entry.mode = mode.as_str();
        }
    });
}

/// Record the typed reuse counts. The caller sets the mode separately so scan
/// and proposal lines cannot silently overwrite each other.
pub fn set_judged(value: bool) {
    super::PENDING.with(|pending| {
        if let Some(entry) = pending.borrow_mut().as_mut() {
            entry.judged = value;
        }
    });
}

pub fn set_reuse_counts(counts: ReuseJournalCounts) {
    super::PENDING.with(|pending| {
        if let Some(entry) = pending.borrow_mut().as_mut() {
            entry.truncated = counts.truncated;
            entry.reuse_counts = Some(counts);
        }
    });
}
