//! Index integrity, parse-rate, and staleness checks.

use super::*;

pub(super) fn index_integrity(data: &AuditData) -> CheckResult {
    if data.repos.is_empty() {
        return check(
            "index.integrity",
            "correctness",
            Status::Skipped,
            None,
            "count",
            lower(0.0, None, None),
            "doctor",
            "no repository index is in scope".into(),
        );
    }
    let mut worst = Status::Pass;
    let mut alias = None;
    let mut failures = 0.0;
    let mut hard = 0usize;
    let mut soft = 0usize;
    let mut pending = 0usize;
    for repo in &data.repos {
        let status = match &repo.state {
            IndexState::Ok(_) => Status::Pass,
            IndexState::Empty(_) | IndexState::Missing => {
                soft += 1;
                Status::Warn
            }
            IndexState::Symlink
            | IndexState::Tracked
            | IndexState::Corrupt
            | IndexState::Schema => {
                hard += 1;
                Status::Critical
            }
            IndexState::TooLarge | IndexState::BuildInProgress => {
                pending += 1;
                Status::Insufficient
            }
        };
        if status != Status::Pass {
            failures += 1.0;
            alias.get_or_insert_with(|| repo.alias.clone());
        }
        if severity(status) > severity(worst) {
            worst = status;
        }
    }
    let message = if hard > 0 {
        format!(
            "{} of {} checked indexes failed integrity, schema, or provenance",
            hard,
            data.repos.len()
        )
    } else if soft > 0 {
        format!(
            "{} of {} checked indexes are missing or empty",
            soft,
            data.repos.len()
        )
    } else if pending > 0 {
        format!(
            "{} of {} checked indexes could not be scanned",
            pending,
            data.repos.len()
        )
    } else {
        format!("all {} checked indexes are readable", data.repos.len())
    };
    let mut c = check(
        "index.integrity",
        "correctness",
        worst,
        Some(failures),
        "repos",
        lower(0.0, Some(0.0), None),
        "doctor",
        message,
    );
    c.repo_alias = alias;
    c
}

pub(super) fn parse_errors(data: &AuditData) -> CheckResult {
    let mut worst = Status::Pass;
    let mut value = 0.0;
    let mut alias = None;
    let mut eligible = 0;
    for repo in &data.repos {
        let (IndexState::Ok(f) | IndexState::Empty(f)) = &repo.state else {
            continue;
        };
        if f.files < 20 {
            continue;
        }
        eligible += 1;
        let rate = f.parse_errors as f64 / f.files as f64;
        if rate > value {
            value = rate;
            alias = Some(repo.alias.clone());
        }
        let status = if rate > 0.10 && f.parse_errors >= 5 {
            Status::Critical
        } else if rate > 0.05 && f.parse_errors >= 2 {
            Status::Warn
        } else {
            Status::Pass
        };
        if severity(status) > severity(worst) {
            worst = status;
        }
    }
    if eligible == 0 {
        return check(
            "index.parse_errors",
            "freshness",
            Status::Skipped,
            None,
            "ratio",
            lower(0.05, Some(0.10), None),
            "doctor",
            "no index has the 20-file minimum for parse-rate evaluation".into(),
        );
    }
    let mut c = check(
        "index.parse_errors",
        "freshness",
        worst,
        Some(value),
        "ratio",
        lower(0.05, Some(0.10), None),
        "doctor",
        format!("highest parse-error rate is {}", pct(value)),
    );
    c.repo_alias = alias;
    c
}

pub(super) fn staleness(data: &AuditData) -> CheckResult {
    let mut worst = Status::Pass;
    let mut value = 0.0;
    let mut alias = None;
    let mut eligible = 0;
    let mut moving = false;
    for repo in &data.repos {
        match &repo.state {
            IndexState::BuildInProgress => moving = true,
            IndexState::Ok(f) if f.files >= 20 => {
                eligible += 1;
                if f.stale_ratio > value {
                    value = f.stale_ratio;
                    alias = Some(repo.alias.clone());
                }
                let changed = (f.stale_ratio * f.files as f64).round() as i64;
                let status = if f.stale_ratio > 0.25 && changed >= 5 {
                    Status::Critical
                } else if f.stale_ratio > 0.10 && changed >= 2 {
                    Status::Warn
                } else {
                    Status::Pass
                };
                if severity(status) > severity(worst) {
                    worst = status;
                }
            }
            IndexState::TooLarge => moving = true,
            _ => {}
        }
    }
    if moving && worst == Status::Pass {
        return check(
            "index.staleness",
            "freshness",
            Status::Insufficient,
            Some(value),
            "ratio",
            lower(0.10, Some(0.25), None),
            "doctor",
            "an index is building or too large to scan safely".into(),
        );
    }
    if eligible == 0 {
        return check(
            "index.staleness",
            "freshness",
            Status::Skipped,
            None,
            "ratio",
            lower(0.10, Some(0.25), None),
            "doctor",
            "no index has the 20-file minimum for staleness evaluation".into(),
        );
    }
    let mut c = check(
        "index.staleness",
        "freshness",
        worst,
        Some(value),
        "ratio",
        lower(0.10, Some(0.25), None),
        "doctor",
        format!("highest stale ratio is {}", pct(value)),
    );
    c.repo_alias = alias;
    c
}
