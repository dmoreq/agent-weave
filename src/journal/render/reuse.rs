//! Reuse-specific usage fields: parsing the journal's typed count object and
//! folding it into one tool aggregate.

use serde_json::Value;

use super::ToolAgg;

#[derive(Default, Clone, Copy)]
pub(crate) struct ReuseUsage {
    pub(crate) provisional: usize,
    pub(crate) low: usize,
    pub(crate) medium: usize,
    pub(crate) high: usize,
    pub(crate) suppressed: usize,
    pub(crate) guarded_out: usize,
    pub(crate) below_threshold: usize,
    pub(crate) omitted: usize,
    pub(crate) capped: usize,
}

pub(crate) fn parse(value: &Value) -> Option<ReuseUsage> {
    if !value.is_object() {
        return None;
    }
    let number = |key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0) as usize;
    Some(ReuseUsage {
        provisional: number("provisional"),
        low: number("low"),
        medium: number("medium"),
        high: number("high"),
        suppressed: number("suppressed"),
        guarded_out: number("guarded_out"),
        below_threshold: number("below_threshold"),
        omitted: number("omitted"),
        capped: number("capped"),
    })
}

pub(super) fn fold(usage: Option<ReuseUsage>, truncated: bool, has_mode: bool, agg: &mut ToolAgg) {
    if let Some(reuse) = usage {
        agg.reuse_usage.provisional += reuse.provisional;
        agg.reuse_usage.low += reuse.low;
        agg.reuse_usage.medium += reuse.medium;
        agg.reuse_usage.high += reuse.high;
        agg.reuse_usage.suppressed += reuse.suppressed;
        agg.reuse_usage.guarded_out += reuse.guarded_out;
        agg.reuse_usage.below_threshold += reuse.below_threshold;
        agg.reuse_usage.omitted += reuse.omitted;
        agg.reuse_usage.capped += reuse.capped;
    }
    if truncated {
        agg.reuse_truncated += 1;
    }
    if has_mode {
        agg.reuse_modes += 1;
    }
}
