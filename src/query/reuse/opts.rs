//! Validated scan options shared by CLI, MCP, dashboard, and build hints.

use super::*;

pub(crate) struct ReuseOpts {
    pub(super) paths: Vec<String>,
    pub(super) limit: usize,
    pub(super) deep_callee: bool,
    pub(super) min_confidence: ReuseConfidence,
    pub(super) explain: bool,
    pub(super) freshness_mode: FreshnessMode,
}

impl ReuseOpts {
    pub(super) fn new(
        paths: Vec<String>,
        limit: usize,
        deep_callee: bool,
        min_confidence: ReuseConfidence,
        explain: bool,
    ) -> Result<Self> {
        if !(1..=100).contains(&limit) {
            return Err(EgError::Usage("limit must be 1..=100".to_string()));
        }
        if matches!(
            min_confidence,
            ReuseConfidence::Medium | ReuseConfidence::High
        ) {
            return Err(EgError::Usage(
                "min-confidence must be low|provisional".to_string(),
            ));
        }
        Ok(Self {
            paths,
            limit,
            deep_callee,
            min_confidence,
            explain,
            freshness_mode: FreshnessMode::Full,
        })
    }
}
