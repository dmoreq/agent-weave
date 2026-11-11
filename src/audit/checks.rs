//! The 15-check audit registry. Thresholds and status precedence are
//! normative in docs/designs/2026-11-11_engram-health-standards.md.

use super::collect::{is_navigation, AuditData};
use super::index::IndexState;
use super::memory::MemoryFailure;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pass,
    Warn,
    Critical,
    Insufficient,
    Skipped,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Warn => "warn",
            Status::Critical => "critical",
            Status::Insufficient => "insufficient",
            Status::Skipped => "skipped",
        }
    }
}

#[derive(Debug)]
pub struct Threshold {
    pub warn: Option<f64>,
    pub critical: Option<f64>,
    pub direction: &'static str,
    pub multiplier: Option<f64>,
}

#[derive(Debug)]
pub struct BaselineView {
    pub value: f64,
    pub ratio: Option<f64>,
    pub sufficient: bool,
}

#[derive(Debug)]
pub struct CheckResult {
    pub id: &'static str,
    pub dimension: &'static str,
    pub status: Status,
    pub value: Option<f64>,
    pub unit: &'static str,
    pub floor: Option<f64>,
    pub baseline: Option<BaselineView>,
    pub threshold: Threshold,
    pub action: &'static str,
    pub repo_alias: Option<String>,
    pub message: String,
}

pub fn overall_status(checks: &[CheckResult]) -> Status {
    if checks.iter().any(|c| c.status == Status::Critical) {
        Status::Critical
    } else if checks.iter().any(|c| c.status == Status::Warn) {
        Status::Warn
    } else {
        Status::Pass
    }
}

pub fn data_status(data: &AuditData, checks: &[CheckResult]) -> &'static str {
    if data.coverage.window_entries + data.coverage.baseline_entries == 0 {
        return "insufficient";
    }
    if checks
        .iter()
        .any(|c| matches!(c.status, Status::Insufficient | Status::Skipped))
    {
        "partial"
    } else {
        "sufficient"
    }
}

pub fn evaluate(data: &AuditData) -> Vec<CheckResult> {
    vec![
        index_integrity(data),
        memory_integrity(data),
        ops_errors(data),
        parse_errors(data),
        staleness(data),
        malformed(data),
        continuity(data),
        performance(data),
        memory_refs(data),
        adoption_navigation(data),
        pillars(data),
        empty_results(data),
        memory_reads(data),
        reuse_activity(data),
        savings_per_call(data),
    ]
}

#[allow(clippy::too_many_arguments)]
fn check(
    id: &'static str,
    dimension: &'static str,
    status: Status,
    value: Option<f64>,
    unit: &'static str,
    threshold: Threshold,
    action: &'static str,
    message: String,
) -> CheckResult {
    CheckResult {
        id,
        dimension,
        status,
        value,
        unit,
        floor: threshold.warn,
        baseline: None,
        threshold,
        action,
        repo_alias: None,
        message,
    }
}

fn lower(warn: f64, critical: Option<f64>, mult: Option<f64>) -> Threshold {
    Threshold {
        warn: Some(warn),
        critical,
        direction: "lower_is_better",
        multiplier: mult,
    }
}

fn higher(floor: f64, mult: Option<f64>) -> Threshold {
    Threshold {
        warn: Some(floor),
        critical: None,
        direction: "higher_is_better",
        multiplier: mult,
    }
}

fn lower_status(value: f64, t: &Threshold, baseline: Option<&BaselineView>) -> Status {
    if t.critical.is_some_and(|c| value > c) {
        return Status::Critical;
    }
    if t.warn.is_some_and(|w| value > w) {
        return Status::Warn;
    }
    if let (Some(b), Some(m)) = (baseline, t.multiplier) {
        if b.sufficient && b.value > 0.0 && value > m * b.value {
            return Status::Warn;
        }
    }
    Status::Pass
}

fn higher_status(value: f64, t: &Threshold, baseline: Option<&BaselineView>) -> Status {
    if t.warn.is_some_and(|w| value < w) {
        return Status::Warn;
    }
    if let (Some(b), Some(m)) = (baseline, t.multiplier) {
        if b.sufficient && b.value > 0.0 && value < m * b.value {
            return Status::Warn;
        }
    }
    Status::Pass
}

fn baseline_of(w: f64, b: Option<(f64, bool)>) -> Option<BaselineView> {
    b.map(|(value, sufficient)| BaselineView {
        value,
        ratio: if value > 0.0 { Some(w / value) } else { None },
        sufficient,
    })
}

fn pct(v: f64) -> String {
    format!("{:.1}%", v * 100.0)
}

mod effectiveness;
mod index_checks;
mod runtime;

use effectiveness::*;
use index_checks::*;
use runtime::*;

fn severity(s: Status) -> u8 {
    match s {
        Status::Pass => 0,
        Status::Skipped => 1,
        Status::Insufficient => 2,
        Status::Warn => 3,
        Status::Critical => 4,
    }
}
