//! Operational, telemetry, and latency checks.

use super::*;

pub(super) fn memory_integrity(data: &AuditData) -> CheckResult {
    let status = match data.memory.failure {
        Some(_) => Status::Critical,
        None if data.memory.stores_truncated => Status::Insufficient,
        None => Status::Pass,
    };
    check(
        "memory.integrity",
        "correctness",
        status,
        Some(data.memory.stores_checked as f64),
        "stores",
        lower(0.0, Some(0.0), None),
        "memory",
        match data.memory.failure {
            Some(MemoryFailure::Corrupt) => "a memory store is corrupt or unreadable".into(),
            Some(MemoryFailure::Schema) => "a memory store has an incompatible schema".into(),
            Some(MemoryFailure::Symlink) => "a memory store is a symlink".into(),
            Some(MemoryFailure::Tracked) => "a repository tracks its .engram directory".into(),
            Some(MemoryFailure::TooLarge) => "a memory store exceeds the audit size cap".into(),
            Some(MemoryFailure::Unreadable) => "a memory store could not be opened".into(),
            None => format!("{} memory stores readable", data.memory.stores_checked),
        },
    )
}

pub(super) fn ops_errors(data: &AuditData) -> CheckResult {
    let m = &data.window_metrics;
    let outage = m.entries >= 10 && m.successes == 0;
    let status = if m.ops_critical > 0 || outage {
        Status::Critical
    } else if m.lock_errors > 3 {
        Status::Warn
    } else {
        Status::Pass
    };
    check(
        "ops.errors",
        "correctness",
        status,
        Some(m.ops_critical as f64),
        "calls",
        lower(0.0, Some(0.0), None),
        "usage",
        if outage {
            format!("{} calls, none succeeded", m.entries)
        } else {
            format!(
                "{} calls exited 3/5/6/7; {} exited 4 (lock)",
                m.ops_critical, m.lock_errors
            )
        },
    )
}

pub(super) fn malformed(data: &AuditData) -> CheckResult {
    let c = &data.coverage;
    if c.lines < 20 {
        return check(
            "telemetry.malformed",
            "telemetry",
            Status::Insufficient,
            None,
            "ratio",
            lower(0.005, Some(0.02), None),
            "inspect",
            format!("only {} journal lines; need 20", c.lines),
        );
    }
    let rate = (c.malformed_lines + c.invalid_lines) as f64 / c.lines as f64;
    check(
        "telemetry.malformed",
        "telemetry",
        lower_status(rate, &lower(0.005, Some(0.02), None), None),
        Some(rate),
        "ratio",
        lower(0.005, Some(0.02), None),
        "inspect",
        format!(
            "{} of {} journal lines are malformed or invalid",
            c.malformed_lines + c.invalid_lines,
            c.lines
        ),
    )
}

pub(super) fn continuity(data: &AuditData) -> CheckResult {
    let c = &data.coverage;
    if c.window_entries + c.baseline_entries == 0 {
        return check(
            "telemetry.continuity",
            "telemetry",
            Status::Insufficient,
            None,
            "hours",
            lower(96.0, Some(336.0), None),
            "inspect",
            "no journal entries in the selected windows".into(),
        );
    }
    let age_h = c
        .newest_ts
        .map(|t| (data.now - t).max(0) as f64 / 3600.0)
        .unwrap_or(0.0);
    let status = if c.window_entries == 0 {
        if data.baseline.active_days >= 3 {
            if age_h > 336.0 {
                Status::Critical
            } else {
                Status::Warn
            }
        } else {
            Status::Insufficient
        }
    } else if age_h > 96.0 {
        Status::Warn
    } else {
        Status::Pass
    };
    check(
        "telemetry.continuity",
        "telemetry",
        status,
        Some(age_h),
        "hours",
        lower(96.0, Some(336.0), None),
        "inspect",
        format!("newest journal entry is {:.0}h old", age_h),
    )
}

pub(super) fn performance(data: &AuditData) -> CheckResult {
    let mut evaluated = 0;
    let mut worst = Status::Pass;
    let mut status_tool = String::new();
    let mut status_p95 = 0.0;
    let mut max_tool = String::new();
    let mut max_p95 = 0.0;
    for (tool, agg) in &data.window_metrics.tools {
        let Some((warn, critical)) = budget(tool) else {
            continue;
        };
        if agg.calls < 20 {
            continue;
        }
        evaluated += 1;
        let p95 = agg.p95();
        let baseline = data
            .baseline_metrics
            .tools
            .get(tool)
            .filter(|b| b.calls >= 20)
            .map(|b| b.p95())
            .filter(|b| *b > 0.0);
        let mut status = if p95 > critical {
            Status::Critical
        } else if p95 > warn {
            Status::Warn
        } else {
            Status::Pass
        };
        if let Some(b) = baseline {
            if data.baseline.sufficient && p95 > 2.0 * b {
                status = Status::Warn;
            }
        }
        if max_tool.is_empty() || p95 > max_p95 {
            max_p95 = p95;
            max_tool = tool.clone();
        }
        if severity(status) > severity(worst) {
            worst = status;
            status_tool = tool.clone();
            status_p95 = p95;
        }
    }
    if evaluated == 0 {
        return check(
            "performance.p95",
            "performance",
            Status::Insufficient,
            None,
            "ms",
            lower(50.0, Some(150.0), Some(2.0)),
            "inspect",
            "no tool has the 20-call minimum for p95".into(),
        );
    }
    let (worst_tool, worst_p95) = if worst == Status::Pass {
        (max_tool, max_p95)
    } else {
        (status_tool, status_p95)
    };
    let mut c = check(
        "performance.p95",
        "performance",
        worst,
        Some(worst_p95),
        "ms",
        lower(50.0, Some(150.0), Some(2.0)),
        "usage",
        format!("highest p95 is {:.1} ms for {}", worst_p95, worst_tool),
    );
    c.baseline = data
        .baseline_metrics
        .tools
        .get(&worst_tool)
        .filter(|b| b.calls >= 20)
        .map(|b| BaselineView {
            value: b.p95(),
            ratio: if b.p95() > 0.0 {
                Some(worst_p95 / b.p95())
            } else {
                None
            },
            sufficient: data.baseline.sufficient,
        });
    c
}

pub(super) fn budget(tool: &str) -> Option<(f64, f64)> {
    if is_navigation(tool) {
        return Some((50.0, 150.0));
    }
    match tool {
        "search" | "symbols" => Some((100.0, 300.0)),
        "mem_query" | "mem_get" => Some((50.0, 150.0)),
        "mem_write" | "mem_touch" | "mem_delete" | "mem_gc" | "mem_import" => Some((100.0, 300.0)),
        "build" | "reuse" => Some((5000.0, 15000.0)),
        _ => None,
    }
}
