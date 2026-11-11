//! `eg audit`: read-only L1 health and effectiveness report.
//! The command folds the retained usage journal, index snapshots, and
//! counts-only memory refs into a stable check registry. It never prunes,
//! rotates, or writes; see docs/health.md for the normative standards.

use std::path::Path;
use std::time::Instant;

use crate::error::{EgError, Result};

mod checks;
mod collect;
mod index;
mod memory;
mod render;
#[cfg(test)]
mod tests;

pub use collect::AuditData;

/// One `eg audit` invocation after CLI validation.
pub struct AuditRequest {
    pub json: bool,
    pub global: bool,
    pub days: u32,
    pub baseline_days: u32,
    pub all_checks: bool,
    pub reveal_paths: bool,
    pub fail_on: Option<String>,
}

pub struct AuditOutcome {
    pub stdout: String,
    pub exit_code: i32,
}

pub fn execute(req: &AuditRequest, root_arg: Option<&Path>) -> Result<AuditOutcome> {
    validate(req)?;
    let started = Instant::now();
    let now = crate::db::unix_now();
    let mode = if req.global {
        collect::Mode::Global
    } else {
        let root = crate::walk::discover_root(root_arg).map_err(|_| {
            EgError::NoResults(
                "audit: no repository here; run eg audit --global for machine-wide telemetry"
                    .to_string(),
            )
        })?;
        collect::Mode::Local(root)
    };
    let data = collect::collect(&mode, now, req.days, req.baseline_days)?;
    let checks = checks::evaluate(&data);
    let status = checks::overall_status(&checks);
    let data_status = checks::data_status(&data, &checks);
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let stdout = if req.json {
        render::json(&data, &checks, status, data_status, elapsed_ms)
    } else {
        render::human(
            &data,
            &checks,
            status,
            data_status,
            elapsed_ms,
            req.all_checks,
            req.reveal_paths,
        )
    };
    let no_data = data.coverage.window_entries + data.coverage.baseline_entries == 0;
    let fail = match req.fail_on.as_deref() {
        Some("warn") => status != checks::Status::Pass,
        Some("critical") => status == checks::Status::Critical,
        _ => false,
    };
    Ok(AuditOutcome {
        stdout,
        exit_code: if no_data || fail { 1 } else { 0 },
    })
}

fn validate(req: &AuditRequest) -> Result<()> {
    if !(1..=90).contains(&req.days) {
        return Err(EgError::Usage("audit: --days must be 1..=90".into()));
    }
    if !(3..=90).contains(&req.baseline_days) {
        return Err(EgError::Usage(
            "audit: --baseline-days must be 3..=90".into(),
        ));
    }
    if req.fail_on.is_some() && !matches!(req.fail_on.as_deref(), Some("warn" | "critical")) {
        return Err(EgError::Usage(
            "audit: --fail-on must be warn or critical".into(),
        ));
    }
    Ok(())
}

/// Stable pseudonymous repository label. This is not a confidentiality
/// control: the hash is unkeyed and the path space is small.
pub(crate) fn repo_alias(path: &Path) -> String {
    let hash = crate::hash::fnv1a64(path.to_string_lossy().as_bytes());
    format!("repo-{hash:016x}")
}
