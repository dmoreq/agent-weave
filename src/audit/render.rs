//! Human and JSON rendering for `eg audit`. Every string is a fixed template
//! over enums, numbers, timestamps, and pseudonymous aliases.

use serde_json::{json, Value};

use super::checks::{CheckResult, Status};
use super::collect::AuditData;
use super::index::{IndexState, RepoFacts};
use crate::time::civil_from_days;

pub fn human(
    data: &AuditData,
    checks: &[CheckResult],
    status: Status,
    data_status: &str,
    elapsed_ms: u64,
    all_checks: bool,
    reveal: bool,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "eg audit: health={} data={} scope={} generated={}\n",
        status.as_str(),
        data_status,
        data.mode,
        fmt_utc(data.now)
    ));
    out.push_str(&format!(
        "window={}..{} ({}d) baseline={}..{} ({}d)\n",
        fmt_utc(data.window.start_ts),
        fmt_utc(data.window.end_ts - 1),
        data.window.days,
        fmt_utc(data.baseline.start_ts),
        fmt_utc(data.baseline.end_ts - 1),
        data.baseline.days
    ));
    out.push_str(&format!(
        "baseline: sufficient={} successful={}/{} active_days={}/{} clipped={}\n",
        yes_no(data.baseline.sufficient),
        data.baseline.successful_calls,
        20,
        data.baseline.active_days,
        3,
        yes_no(data.baseline.clipped)
    ));
    let c = &data.coverage;
    out.push_str(&format!(
        "coverage: window={} baseline={} malformed={} invalid={} truncated={} repos={}/{} stores={} elapsed={}ms\n",
        c.window_entries,
        c.baseline_entries,
        c.malformed_lines,
        c.invalid_lines,
        yes_no(c.truncated),
        c.repos_checked,
        c.repos_active,
        c.stores_checked,
        elapsed_ms
    ));
    out.push_str(&format!("checks={}\n", counts_line(checks)));
    for check in checks {
        if !all_checks && check.status == Status::Pass {
            continue;
        }
        let alias = check.repo_alias.as_deref().map(|a| {
            if reveal {
                data.repos
                    .iter()
                    .find(|r| r.alias == a)
                    .map(|r| r.path.display().to_string())
                    .unwrap_or_else(|| a.to_string())
            } else {
                a.to_string()
            }
        });
        let suffix = alias.map(|a| format!(" repo={a}")).unwrap_or_default();
        out.push_str(&format!(
            "[{}] {} {}{} - {}; {}\n",
            check.status.as_str(),
            check.id,
            check.message,
            suffix,
            action_command(check.action, data.mode),
            action_hint(check.action)
        ));
    }
    out
}

pub fn json(
    data: &AuditData,
    checks: &[CheckResult],
    status: Status,
    data_status: &str,
    elapsed_ms: u64,
) -> String {
    let counts = json!({
        "checks": checks.len(),
        "pass": checks.iter().filter(|c| c.status == Status::Pass).count(),
        "warn": checks.iter().filter(|c| c.status == Status::Warn).count(),
        "critical": checks.iter().filter(|c| c.status == Status::Critical).count(),
        "insufficient": checks.iter().filter(|c| c.status == Status::Insufficient).count(),
        "skipped": checks.iter().filter(|c| c.status == Status::Skipped).count(),
    });
    let checks_json: Vec<Value> = checks.iter().map(check_json).collect();
    let repos: Vec<Value> = data.repos.iter().map(repo_json).collect();
    let doc = json!({
        "schema_version": 1,
        "standards_version": 1,
        "audit_version": env!("CARGO_PKG_VERSION"),
        "generated_at": data.now,
        "elapsed_ms": elapsed_ms,
        "mode": data.mode,
        "status": status.as_str(),
        "data_status": data_status,
        "window": {"days": data.window.days, "start_ts": data.window.start_ts, "end_ts": data.window.end_ts},
        "baseline": {
            "days": data.baseline.days,
            "start_ts": data.baseline.start_ts,
            "end_ts": data.baseline.end_ts,
            "sufficient": data.baseline.sufficient,
            "successful_calls": data.baseline.successful_calls,
            "active_days": data.baseline.active_days,
            "required_successful_calls": 20,
            "required_active_days": 3,
            "clipped": data.baseline.clipped
        },
        "coverage": coverage_json(data),
        "counts": counts,
        "checks": checks_json,
        "repos": repos
    });
    let mut text = serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into());
    text.push('\n');
    text
}

fn check_json(c: &CheckResult) -> Value {
    json!({
        "id": c.id,
        "dimension": c.dimension,
        "status": c.status.as_str(),
        "value": c.value,
        "unit": c.unit,
        "floor": c.floor,
        "baseline": c.baseline.as_ref().map(|b| json!({
            "value": b.value, "ratio": b.ratio, "sufficient": b.sufficient
        })),
        "threshold": {
            "warn": c.threshold.warn,
            "critical": c.threshold.critical,
            "direction": c.threshold.direction,
            "multiplier": c.threshold.multiplier
        },
        "action": c.action,
        "repo_alias": c.repo_alias,
        "message": c.message
    })
}

fn repo_json(r: &RepoFacts) -> Value {
    let mut value = json!({"alias": r.alias});
    match &r.state {
        IndexState::Ok(f) | IndexState::Empty(f) => {
            value["index"] = json!(if matches!(&r.state, IndexState::Ok(_)) {
                "ok"
            } else {
                "empty"
            });
            value["files"] = json!(f.files);
            value["symbols"] = json!(f.symbols);
            value["parse_errors"] = json!(f.parse_errors);
            value["stale_ratio"] = json!(f.stale_ratio);
            value["built_at"] = json!(f.built_at);
        }
        IndexState::Missing => value["index"] = json!("missing"),
        IndexState::Symlink => value["index"] = json!("symlink"),
        IndexState::Tracked => value["index"] = json!("tracked"),
        IndexState::Corrupt => value["index"] = json!("corrupt"),
        IndexState::Schema => value["index"] = json!("schema"),
        IndexState::TooLarge => value["index"] = json!("too_large"),
        IndexState::BuildInProgress => value["index"] = json!("build_in_progress"),
    }
    value
}

fn coverage_json(data: &AuditData) -> Value {
    let c = &data.coverage;
    json!({
        "window_entries": c.window_entries,
        "baseline_entries": c.baseline_entries,
        "oldest_ts": c.oldest_ts,
        "newest_ts": c.newest_ts,
        "malformed_lines": c.malformed_lines,
        "invalid_lines": c.invalid_lines,
        "legacy_lines": c.legacy_lines,
        "dropped_dead_repos": c.dropped_dead_repos,
        "future_ts_lines": c.future_ts_lines,
        "outside_scope_lines": c.outside_scope_lines,
        "truncated": c.truncated,
        "repos_active": c.repos_active,
        "repos_checked": c.repos_checked,
        "repos_truncated": c.repos_truncated,
        "stores_checked": c.stores_checked,
        "stores_truncated": c.stores_truncated,
        "generations": c.generations.iter().map(|g| json!({
            "name": g.name, "status": g.status, "bytes": g.bytes, "truncated": g.truncated
        })).collect::<Vec<_>>()
    })
}

fn counts_line(checks: &[CheckResult]) -> String {
    let n = |s: Status| checks.iter().filter(|c| c.status == s).count();
    format!(
        "{} pass={} warn={} critical={} insufficient={} skipped={}",
        checks.len(),
        n(Status::Pass),
        n(Status::Warn),
        n(Status::Critical),
        n(Status::Insufficient),
        n(Status::Skipped)
    )
}

fn action_command(action: &str, mode: &str) -> &'static str {
    match (action, mode) {
        ("doctor", "local") => "eg doctor",
        ("doctor", _) => "eg audit --global --reveal-paths",
        ("usage", "local") => "eg usage --quality",
        ("usage", _) => "eg usage --global --quality",
        ("memory", "local") => "eg mem query --all",
        ("memory", _) => "eg mem query --all --global",
        ("inspect", "local") => "eg audit --all-checks",
        ("inspect", _) => "eg audit --global --all-checks",
        _ => "",
    }
}

fn action_hint(action: &str) -> &'static str {
    match action {
        "doctor" => "inspect index health",
        "usage" => "inspect recent telemetry",
        "memory" => "inspect memory notes",
        "inspect" => "widen the window or show all checks",
        _ => "no action",
    }
}

fn yes_no(v: bool) -> &'static str {
    if v {
        "yes"
    } else {
        "no"
    }
}

/// UTC civil timestamp without a date crate. `ts` is a positive Unix second.
fn fmt_utc(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}
