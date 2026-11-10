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

/// Create or validate the machine-wide confinement directory and keep it
/// owner-only. The repo-local `.engram/` is left at the project's own mode:
/// several developers can share one working tree.
pub(crate) fn ensure_global_dir(path: &Path) -> Result<()> {
    ensure_dir_validated(path, "global .engram directory", true)?;
    harden_dir_mode(path)
}

/// Lazily built map: confinement directory -> rejection detail (`None` means
/// the directory holds no git-tracked files).
static TRACKED_DIR_VERDICTS: std::sync::OnceLock<
    std::sync::Mutex<HashMap<PathBuf, Option<String>>>,
> = std::sync::OnceLock::new();

/// Provenance gate for repo-local state. A repository that commits `.engram/`
/// hands every querying agent attacker-authored memories and index rows, so a
/// tracked confinement directory is rejected (exit 2) instead of trusted.
/// Verdicts are cached per root: the MCP server re-enters this gate on every
/// tool call, and a repository's tracking state cannot change mid-process.
pub(crate) fn reject_tracked_eg_dir(root: &Path) -> Result<()> {
    let dir = eg_dir(root);
    // The `git ls-files` probe costs a process spawn: only pay it when the
    // confinement directory exists and could actually carry planted state.
    if !dir.exists() {
        return Ok(());
    }
    let cache = TRACKED_DIR_VERDICTS.get_or_init(Default::default);
    if let Ok(guard) = cache.lock() {
        if let Some(verdict) = guard.get(&dir) {
            return match verdict {
                Some(detail) => Err(EgError::Security(detail.clone())),
                None => Ok(()),
            };
        }
    }
    let verdict = git_tracked_engram(root);
    if let Ok(mut guard) = cache.lock() {
        guard.insert(dir, verdict.clone());
    }
    match verdict {
        Some(detail) => Err(EgError::Security(detail)),
        None => Ok(()),
    }
}

/// `git ls-files` over the confinement directory. No git, no repository, or a
/// failing git means "not tracked": without git there is no committed state to
/// distrust, and a broken git must never brick local queries.
fn git_tracked_engram(root: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z", "--", ".engram"])
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    Some(
        ".engram/ contains git-tracked files — untrack and remove to prevent malicious memory injection"
            .to_string(),
    )
}
