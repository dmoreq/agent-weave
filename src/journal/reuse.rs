//! Reuse-specific journal fields: typed count partition and candidate
//! identities. Kept separate from `journal.rs` to stay under the 500-line cap.

use serde_json::Value;

/// Count partition written to the usage line for a reuse scan.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReuseJournalCounts {
    pub provisional: usize,
    pub low: usize,
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

/// Record the typed reuse counts and mark the line as a scan.
pub fn set_reuse_counts(counts: ReuseJournalCounts) {
    super::PENDING.with(|pending| {
        if let Some(entry) = pending.borrow_mut().as_mut() {
            entry.mode = "scan";
            entry.truncated = counts.truncated;
            entry.reuse_counts = Some(counts);
        }
    });
}
