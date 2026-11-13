//! Validated scan options shared by CLI, MCP, dashboard, and build hints.

use super::*;

pub(crate) struct ReuseOpts {
    pub(super) mode: ReuseMode,
    pub(super) paths: Vec<String>,
    pub(super) proposal_signature: Option<String>,
    pub(super) proposed_callees: Vec<String>,
    pub(super) limit: usize,
    pub(super) deep_callee: bool,
    pub(super) min_confidence: ReuseConfidence,
    pub(super) explain: bool,
    pub(super) show_suppressed: bool,
    pub(super) coverage: Vec<super::coverage::CoverageKind>,
    pub(super) judge: bool,
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
        Ok(Self {
            mode: ReuseMode::Scan,
            paths,
            proposal_signature: None,
            proposed_callees: Vec::new(),
            limit,
            deep_callee,
            min_confidence,
            explain,
            show_suppressed: false,
            coverage: Vec::new(),
            judge: false,
            freshness_mode: FreshnessMode::Full,
        })
    }

    pub(super) fn new_proposal(
        signature: String,
        callees: Vec<String>,
        limit: usize,
        deep_callee: bool,
        min_confidence: ReuseConfidence,
        explain: bool,
    ) -> Result<Self> {
        if !(1..=100).contains(&limit) {
            return Err(EgError::Usage("limit must be 1..=100".to_string()));
        }
        if signature.trim().is_empty() {
            return Err(EgError::Usage("propose must be non-empty".to_string()));
        }
        Ok(Self {
            mode: ReuseMode::Propose,
            paths: Vec::new(),
            proposal_signature: Some(signature),
            proposed_callees: callees,
            limit,
            deep_callee,
            min_confidence,
            explain,
            show_suppressed: false,
            coverage: Vec::new(),
            judge: false,
            freshness_mode: FreshnessMode::Full,
        })
    }

    pub(super) fn with_show_suppressed(mut self, show: bool) -> Self {
        self.show_suppressed = show;
        self
    }

    pub(super) fn with_judge(mut self, enabled: bool) -> Self {
        self.judge = enabled;
        self
    }

    pub(super) fn with_coverage(mut self, values: &[String]) -> Result<Self> {
        let mut kinds = Vec::new();
        for value in values {
            for part in value.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                let kind = super::coverage::CoverageKind::parse(part).ok_or_else(|| {
                    EgError::Usage(format!("coverage must be class,unresolved,tests: {part}"))
                })?;
                if !kinds.contains(&kind) {
                    kinds.push(kind);
                }
            }
        }
        self.coverage = kinds;
        if !self.coverage.is_empty() {
            self.mode = ReuseMode::Coverage;
        }
        Ok(self)
    }
}
