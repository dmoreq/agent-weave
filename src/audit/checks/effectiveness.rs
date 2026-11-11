//! Memory, adoption, quality, and savings checks.

use super::*;

pub(super) fn memory_refs(data: &AuditData) -> CheckResult {
    let m = &data.memory;
    let checkable = m.broken + m.modified + m.valid;
    if checkable == 0 {
        return check(
            "memory.refs",
            "memory",
            Status::Skipped,
            None,
            "ratio",
            lower(0.05, Some(0.20), Some(1.5)),
            "memory",
            "no checkable memory refs in the selected stores".into(),
        );
    }
    if checkable < 5 {
        return check(
            "memory.refs",
            "memory",
            Status::Insufficient,
            Some(checkable as f64),
            "refs",
            lower(0.05, Some(0.20), Some(1.5)),
            "memory",
            format!("only {checkable} checkable refs; need 5"),
        );
    }
    let broken = m.broken as f64 / checkable as f64;
    let modified = m.modified as f64 / checkable as f64;
    let status = if m.broken >= 5 && broken > 0.20 {
        Status::Critical
    } else if (m.broken >= 2 && broken > 0.05) || modified > 0.30 {
        Status::Warn
    } else {
        Status::Pass
    };
    check(
        "memory.refs",
        "memory",
        status,
        Some(broken),
        "ratio",
        lower(0.05, Some(0.20), Some(1.5)),
        "memory",
        format!(
            "{} broken and {} modified of {} checkable refs",
            m.broken, m.modified, checkable
        ),
    )
}

pub(super) fn adoption_navigation(data: &AuditData) -> CheckResult {
    let m = &data.window_metrics;
    if m.successes < 20 {
        return check(
            "adoption.navigation",
            "effectiveness",
            Status::Insufficient,
            None,
            "ratio",
            higher(0.10, Some(0.5)),
            "usage",
            format!("only {} successful calls; need 20", m.successes),
        );
    }
    let value = m.nav_successes as f64 / m.successes as f64;
    let base = data.baseline_metrics.successes > 0 && data.baseline.sufficient;
    let b = base.then(|| {
        data.baseline_metrics.nav_successes as f64 / data.baseline_metrics.successes as f64
    });
    let baseline = baseline_of(value, b.map(|v| (v, true)));
    let mut c = check(
        "adoption.navigation",
        "effectiveness",
        higher_status(value, &higher(0.10, Some(0.5)), baseline.as_ref()),
        Some(value),
        "ratio",
        higher(0.10, Some(0.5)),
        "usage",
        format!("navigation share is {}", pct(value)),
    );
    c.baseline = baseline;
    c
}

pub(super) fn pillars(data: &AuditData) -> CheckResult {
    let m = &data.recent7;
    if m.entries < 20 {
        return check(
            "adoption.pillars",
            "effectiveness",
            Status::Insufficient,
            None,
            "count",
            higher(3.0, None),
            "usage",
            format!("only {} calls in 7 days; need 20", m.entries),
        );
    }
    let graph = m.nav_successes > 0;
    let prewrite = m.prewrite_successes > 0;
    let memory = m.mem_reads > 0;
    let present = [graph, prewrite, memory].iter().filter(|v| **v).count();
    let status = if present < 3 {
        Status::Warn
    } else {
        Status::Pass
    };
    let missing = [
        (!graph).then_some("navigation"),
        (!prewrite).then_some("prewrite"),
        (!memory).then_some("memory-read"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(",");
    check(
        "adoption.pillars",
        "effectiveness",
        status,
        Some(present as f64),
        "pillars",
        higher(3.0, None),
        "usage",
        if present == 3 {
            "all three pillars used in the last 7 days".into()
        } else {
            format!("missing pillar(s): {missing}")
        },
    )
}

pub(super) fn empty_results(data: &AuditData) -> CheckResult {
    let mut worst = Status::Pass;
    let mut value = 0.0;
    let mut family = "";
    for (name, successes, empty, floor) in [
        (
            "navigation",
            data.window_metrics.nav_successes,
            data.window_metrics.nav_empty,
            0.40,
        ),
        (
            "search",
            data.window_metrics.search_successes,
            data.window_metrics.search_empty,
            0.30,
        ),
        (
            "mem-query",
            data.window_metrics.mem_query_successes,
            data.window_metrics.mem_query_empty,
            0.40,
        ),
    ] {
        if successes < 10 {
            continue;
        }
        let rate = empty as f64 / successes as f64;
        if family.is_empty() || rate > value {
            value = rate;
            family = name;
        }
        let status = if rate > floor {
            Status::Warn
        } else {
            Status::Pass
        };
        if severity(status) > severity(worst) {
            worst = status;
        }
    }
    if family.is_empty() {
        return check(
            "quality.empty_results",
            "effectiveness",
            Status::Insufficient,
            None,
            "ratio",
            lower(0.40, None, Some(1.5)),
            "usage",
            "no family has 10 successful calls".into(),
        );
    }
    let mut c = check(
        "quality.empty_results",
        "effectiveness",
        worst,
        Some(value),
        "ratio",
        lower(0.40, None, Some(1.5)),
        "usage",
        format!("{} empty-result share is {}", family, pct(value)),
    );
    c.baseline = None;
    c
}

pub(super) fn memory_reads(data: &AuditData) -> CheckResult {
    let active = data.recent7.active_days.len();
    if active < 3 {
        return check(
            "memory.reads",
            "effectiveness",
            Status::Insufficient,
            None,
            "per_day",
            higher(0.5, Some(0.5)),
            "memory",
            format!("only {active} active days in 7 days; need 3"),
        );
    }
    let value = data.recent7.mem_reads as f64 / active as f64;
    let base = data.baseline.successful_calls.gt(&0).then(|| {
        let b_active = data.baseline.active_days.max(1) as f64;
        data.baseline_metrics.mem_reads as f64 / b_active
    });
    let baseline = baseline_of(value, base.map(|v| (v, data.baseline.sufficient)));
    let mut status = higher_status(value, &higher(0.5, Some(0.5)), baseline.as_ref());
    if data.recent7.mem_writes > 0 && data.recent7.mem_reads == 0 {
        status = Status::Warn;
    }
    let mut c = check(
        "memory.reads",
        "effectiveness",
        status,
        Some(value),
        "per_day",
        higher(0.5, Some(0.5)),
        "memory",
        format!("{:.2} memory reads per active day", value),
    );
    c.baseline = baseline;
    c
}

pub(super) fn reuse_activity(data: &AuditData) -> CheckResult {
    let active = data.recent7.active_days.len();
    if active < 3 {
        return check(
            "reuse.activity",
            "effectiveness",
            Status::Insufficient,
            None,
            "per_day",
            higher(0.2, Some(0.5)),
            "inspect",
            format!("only {active} active days in 7 days; need 3"),
        );
    }
    let reuse = data
        .recent7
        .tools
        .get("reuse")
        .map(|t| t.calls)
        .unwrap_or(0);
    let value = reuse as f64 / active as f64;
    let base = data
        .baseline_metrics
        .tools
        .get("reuse")
        .map(|t| t.calls as f64 / data.baseline.active_days.max(1) as f64);
    let baseline = baseline_of(value, base.map(|v| (v, data.baseline.sufficient)));
    let mut status = higher_status(value, &higher(0.2, Some(0.5)), baseline.as_ref());
    if data.recent7.nav_successes >= 50 && reuse == 0 {
        status = Status::Warn;
    }
    let mut c = check(
        "reuse.activity",
        "effectiveness",
        status,
        Some(value),
        "per_day",
        higher(0.2, Some(0.5)),
        "inspect",
        format!("{:.2} reuse calls per active day", value),
    );
    c.baseline = baseline;
    c
}

pub(super) fn savings_per_call(data: &AuditData) -> CheckResult {
    let m = &data.window_metrics;
    if m.priced_successes < 20 {
        return check(
            "savings.per_call",
            "effectiveness",
            Status::Insufficient,
            None,
            "tokens",
            higher(1000.0, Some(0.5)),
            "usage",
            format!(
                "only {} priced successful calls; need 20",
                m.priced_successes
            ),
        );
    }
    let value = m.savings as f64 / m.priced_successes as f64;
    let base = data.baseline_metrics.priced_successes.gt(&0).then(|| {
        data.baseline_metrics.savings as f64 / data.baseline_metrics.priced_successes as f64
    });
    let baseline = baseline_of(value, base.map(|v| (v, data.baseline.sufficient)));
    let mut c = check(
        "savings.per_call",
        "effectiveness",
        higher_status(value, &higher(1000.0, Some(0.5)), baseline.as_ref()),
        Some(value),
        "tokens",
        higher(1000.0, Some(0.5)),
        "usage",
        format!("estimated savings per priced call is {:.0} tokens", value),
    );
    c.baseline = baseline;
    c
}
