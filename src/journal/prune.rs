//! Usage-log pruning with symlink-safe, atomic rewrites.

use std::fs;
use std::path::Path;

/// Rewrite `log_path` without the lines `should_drop` selects. Invalid JSON is
/// preserved: the pruner owns liveness policy, not syntax repair.
fn prune_matching(
    log_path: &Path,
    should_drop: impl Fn(&serde_json::Value) -> bool,
) -> std::io::Result<usize> {
    if !safe_prunable_log(log_path) {
        return Ok(0);
    }
    let content = match fs::read_to_string(log_path) {
        Ok(c) => c,
        Err(_) => return Ok(0),
    };
    let mut kept = Vec::new();
    let mut pruned = 0;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if should_drop(&val) {
                pruned += 1;
                continue;
            }
        }
        kept.push(trimmed);
    }
    if pruned > 0 {
        let mut new_text = kept.join("\n");
        if !new_text.is_empty() {
            new_text.push('\n');
        }
        super::rewrite_pruned_atomic(log_path, &new_text)?;
    }
    Ok(pruned)
}

/// Prune entries with dead roots or no usable repository identity.
pub fn prune_stale_entries(log_path: &Path) -> std::io::Result<usize> {
    prune_matching(log_path, |val| {
        super::liveness::entry_is_stale(val) || super::liveness::entry_is_unattributed(val)
    })
}

/// Prune entries whose repository root is a scratch path under the OS temp
/// tree. The dashboard global view owns this policy: it lists durable
/// repositories, so probe fixtures must not accumulate in the journal.
pub fn prune_ephemeral_entries(log_path: &Path) -> std::io::Result<usize> {
    prune_matching(log_path, super::liveness::entry_is_ephemeral)
}

/// A prune target must be a regular file beneath a real directory.
fn safe_prunable_log(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let Ok(dir) = fs::symlink_metadata(parent) else {
        return false;
    };
    if !dir.is_dir() || dir.file_type().is_symlink() {
        return false;
    }
    match fs::symlink_metadata(path) {
        Ok(meta) => meta.is_file() && !meta.file_type().is_symlink(),
        Err(err) => err.kind() == std::io::ErrorKind::NotFound,
    }
}

/// Prune the current generation and its rotated (`.1`) predecessor.
fn prune_generations(
    base_path: &Path,
    prune: fn(&Path) -> std::io::Result<usize>,
) -> std::io::Result<usize> {
    let mut total = prune(base_path)?;
    if let Some(parent) = base_path.parent() {
        let rotated = parent.join("usage.log.1");
        if rotated.is_file() {
            total += prune(&rotated)?;
        }
    }
    Ok(total)
}

/// Prune both the current and rotated (`.1`) generations.
pub fn prune_stale_entries_all(base_path: &Path) -> std::io::Result<usize> {
    prune_generations(base_path, prune_stale_entries)
}

/// Prune scratch-root entries from both journal generations.
pub fn prune_ephemeral_entries_all(base_path: &Path) -> std::io::Result<usize> {
    prune_generations(base_path, prune_ephemeral_entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{make_dir_all, remove_tree, write_file};

    #[test]
    fn prune_stale_entries_removes_dead_repos_and_unjudgeable_lines() {
        let dir = crate::db::temp_root("prune-wt");
        let active_wt = dir.join("active-wt");
        let deleted_wt = dir.join("deleted-wt");
        make_dir_all(&active_wt).unwrap();

        let log_path = dir.join("usage.log");
        let line1 = format!(
            r#"{{"tool":"build","repo":"{}","worktree":true}}"#,
            active_wt.display()
        );
        let line2 = format!(
            r#"{{"tool":"build","repo":"{}","worktree":true}}"#,
            deleted_wt.display()
        );
        let line3 = r#"{"tool":"symbols","repo":"/persistent/repo","worktree":false}"#.to_string();
        // Identityless legacy lines (pre-repo-field binaries, test artifacts)
        // are pruned alongside the dead roots.
        let line4 =
            r#"{"tool":"symbols","repo":".tmp/verify/repo1","repo_name":"repo1"}"#.to_string();
        let line5 = r#"{"tool":"mem_query","exit_code":0}"#.to_string();

        write_file(
            &log_path,
            format!("{line1}\n{line2}\n{line3}\n{line4}\n{line5}\n").as_bytes(),
        )
        .unwrap();

        let pruned = prune_stale_entries(&log_path).unwrap();
        assert_eq!(
            pruned, 4,
            "deleted worktree, dead repo, relative and missing repo"
        );

        let remaining = fs::read_to_string(&log_path).unwrap();
        assert!(remaining.contains(&active_wt.to_string_lossy().to_string()));
        assert!(!remaining.contains(&deleted_wt.to_string_lossy().to_string()));
        assert!(!remaining.contains("/persistent/repo"));
        assert!(!remaining.contains(".tmp/verify/repo1"));

        remove_tree(&dir);
    }

    #[test]
    fn prune_ephemeral_entries_drops_scratch_roots_only() {
        let dir = crate::db::temp_root("prune-ephemeral");
        let log_path = dir.join("usage.log");
        let scratch = std::env::temp_dir().join("eg-probe-scratch");
        let stable = env!("CARGO_MANIFEST_DIR");
        let scratch_line = format!(
            r#"{{"tool":"build","repo":"{}"}}"#,
            scratch.to_string_lossy()
        );
        let stable_line = format!(r#"{{"tool":"build","repo":"{stable}"}}"#);
        // Relative and identityless lines stay: the stale rule owns them.
        let relative = r#"{"tool":"build","repo":".tmp/verify/repo1"}"#;
        let identityless = r#"{"tool":"build"}"#;
        write_file(
            &log_path,
            format!("{scratch_line}\n{stable_line}\n{relative}\n{identityless}\n").as_bytes(),
        )
        .unwrap();

        assert_eq!(prune_ephemeral_entries(&log_path).unwrap(), 1);

        let remaining = fs::read_to_string(&log_path).unwrap();
        assert!(!remaining.contains("eg-probe-scratch"));
        assert!(remaining.contains(stable));
        assert!(remaining.contains(".tmp/verify/repo1"));
        assert_eq!(remaining.lines().count(), 3);

        remove_tree(&dir);
    }
}
