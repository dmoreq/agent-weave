//! Confinement hardening: owner-only POSIX modes and the git-provenance gate
//! for repo-local `.engram/` state. No write primitives live here — directory
//! creation stays in `db.rs`, and this module only adjusts modes and inspects
//! git-tracked status.

use super::*;

/// Restrict a confinement directory to its owner (`0700`) on Unix: the global
/// store holds private notes and telemetry, so a permissive umask must not
/// expose it to other local accounts.
#[cfg(unix)]
pub(crate) fn harden_dir_mode(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn harden_dir_mode(_: &Path) -> Result<()> {
    Ok(())
}

/// Restrict one confinement file (`0600`) on Unix: `usage.log` and `memory.db`
/// carry repository paths and agent-authored notes.
#[cfg(unix)]
pub(crate) fn harden_file_mode(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn harden_file_mode(_: &Path) -> Result<()> {
    Ok(())
}

/// Reject a symlinked path before any write or open: a symlink can redirect
/// every later operation outside the intended confinement root.
pub(crate) fn reject_symlink(path: &Path, label: &str) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(EgError::Internal(format!(
                "refusing symlinked {label}: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

/// Reject a symlinked or non-regular journal file. `File::open` follows a
/// symlink; local usage/dashboard/monitor reads must not render another
/// repository's journal through one.
pub(crate) fn reject_unsafe_journal(root: &Path) -> Result<()> {
    let dir = eg_dir(root);
    for name in ["usage.log", "usage.log.1"] {
        let path = dir.join(name);
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if meta.file_type().is_symlink() || !meta.is_file() {
                return Err(EgError::Internal(format!(
                    "refusing unsafe journal file: {}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

/// Create or validate the machine-wide confinement directory and keep it
/// owner-only. The repo-local `.engram/` is left at the project's own mode:
/// several developers can share one working tree.
pub(crate) fn ensure_global_dir(path: &Path) -> Result<()> {
    ensure_dir_validated(path, "global .engram directory", true)?;
    harden_dir_mode(path)
}

/// Provenance gate for repo-local state. A repository that commits `.engram/`
/// hands every querying agent attacker-authored memories and index rows, so a
/// tracked confinement directory is rejected (exit 2) instead of trusted.
///
/// No process-lifetime cache: `git add`, checkout, or a branch switch can
/// change provenance while a long-lived MCP/dashboard process is running.
/// The probe is one `git ls-files` spawn per open; correctness wins over that
/// cost, and a failed probe still means "not tracked" (a broken git must not
/// brick local queries).
pub(crate) fn reject_tracked_eg_dir(root: &Path) -> Result<()> {
    let dir = eg_dir(root);
    if !dir.exists() {
        return Ok(());
    }
    match git_tracked_engram(root) {
        Some(detail) => Err(EgError::Security(detail)),
        None => Ok(()),
    }
}

/// `git ls-files` over the confinement directory. The `:(icase)` pathspec is
/// required on case-insensitive filesystems: a tracked `.ENGRAM/index.db`
/// resolves to the same directory as `.engram/`, but a case-sensitive
/// pathspec returns no rows and would let the planted index through.
/// No git, no repository, or a failing git means "not tracked".
fn git_tracked_engram(root: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z", "--", ":(icase).engram"])
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    // `:(icase)` also matches an unrelated `.ENGRAM/` on a case-sensitive
    // filesystem. Confirm a case variant resolves to the same directory as
    // `.engram/` before rejecting it.
    let canonical_eg = fs::canonicalize(eg_dir(root)).ok();
    for entry in out
        .stdout
        .split(|byte| *byte == 0)
        .filter(|p| !p.is_empty())
    {
        let first = entry.split(|byte| *byte == b'/').next().unwrap_or(entry);
        if first == b".engram" {
            return Some(tracked_detail());
        }
        if first.eq_ignore_ascii_case(b".engram") {
            let variant = root.join(String::from_utf8_lossy(first).as_ref());
            if canonical_eg.is_some() && fs::canonicalize(&variant).ok() == canonical_eg {
                return Some(tracked_detail());
            }
        }
    }
    None
}

fn tracked_detail() -> String {
    ".engram/ contains git-tracked files — untrack and remove to prevent malicious memory injection"
        .to_string()
}
