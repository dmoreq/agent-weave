//! CLI-to-request mapping for `eg audit` (kept out of the main matcher so
//! the surface file stays under the module-size budget).

use super::CommandRequest;

pub(super) fn from_cmd(cmd: crate::cli::Cmd) -> CommandRequest {
    match cmd {
        crate::cli::Cmd::Audit {
            json,
            global,
            days,
            baseline_days,
            all_checks,
            reveal_paths,
            fail_on,
        } => CommandRequest::Audit {
            json,
            global,
            days,
            baseline_days,
            all_checks,
            reveal_paths,
            fail_on,
        },
        _ => unreachable!("from_cli/audit called with a non-audit command"),
    }
}

pub(super) fn journal_args(
    days: u32,
    baseline_days: u32,
    global: bool,
) -> Vec<(&'static str, String)> {
    vec![
        ("days", days.to_string()),
        ("baseline_days", baseline_days.to_string()),
        ("global", global.to_string()),
    ]
}
