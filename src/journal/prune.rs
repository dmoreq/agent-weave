//! Usage-log pruning with symlink-safe, atomic rewrites.

use std::fs;
use std::path::Path;

/// Prune entries with dead roots or no usable repository identity.
pub fn prune_stale_entries(log_path: &Path) -> std::io::Result<usize> {
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
            if super::liveness::entry_is_stale(&val) || super::liveness::entry_is_unattributed(&val)
            {
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

/// Prune both the current and rotated (`.1`) generations.
pub fn prune_stale_entries_all(base_path: &Path) -> std::io::Result<usize> {
    let mut total = prune_stale_entries(base_path)?;
    if let Some(parent) = base_path.parent() {
        let rotated = parent.join("usage.log.1");
        if rotated.is_file() {
            total += prune_stale_entries(&rotated)?;
        }
    }
    Ok(total)
}
