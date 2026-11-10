//! Staleness rule for usage-journal entries.
//!
//! One predicate decides whether an entry names a repository that no longer
//! exists and is disposable. The CLI folds, the dashboard read path, and the
//! pruner all call it, so the three surfaces cannot disagree about which
//! repositories are still live.
//!
//! Only two kinds of root are judged disposable:
//! - a linked git worktree (`.git` is a file), which `git worktree remove`
//!   deletes while its journal lines survive;
//! - a repository under the OS temp directory, where benchmarks and scratch
//!   runs create throwaway roots.
//!
//! An ordinary repository that was deleted keeps its historical totals: `eg
//! usage --global` is a lifetime record, not a live inventory.
//!
//! Relative `repo` values are never judged. `--root .tmp/x` means different
//! things from different working directories, so a liveness answer would
//! depend on which process asks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::render::UsageFields;

/// How long one path verdict stays valid. The dashboard polls every 15 s and
/// the monitor refreshes every 5 s, so a longer window than that keeps the
/// stat count flat without hiding a re-created root for long.
const CACHE_TTL: Duration = Duration::from_secs(30);
/// Upper bound on remembered paths; a pathological log must not grow the map
/// without limit.
const CACHE_CAP: usize = 512;

struct LivenessCache {
    checked_at: Instant,
    verdicts: HashMap<PathBuf, bool>,
}

impl LivenessCache {
    fn new() -> Self {
        Self {
            checked_at: Instant::now(),
            verdicts: HashMap::new(),
        }
    }

    fn exists(&mut self, path: &Path) -> bool {
        if self.checked_at.elapsed() >= CACHE_TTL || self.verdicts.len() >= CACHE_CAP {
            self.verdicts.clear();
            self.checked_at = Instant::now();
        }
        if let Some(&hit) = self.verdicts.get(path) {
            return hit;
        }
        let verdict = path.exists();
        self.verdicts.insert(path.to_path_buf(), verdict);
        verdict
    }
}

fn cache() -> &'static Mutex<LivenessCache> {
    static CACHE: OnceLock<Mutex<LivenessCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(LivenessCache::new()))
}

fn path_exists(path: &Path) -> bool {
    match cache().lock() {
        Ok(mut guard) => guard.exists(path),
        Err(poisoned) => poisoned.into_inner().exists(path),
    }
}

/// The OS temp roots in both literal and canonical spelling: macOS reports
/// `/var/...` while canonicalization yields `/private/var/...`, and a journal
/// line may carry either.
///
/// `std::env::temp_dir()` alone is not enough. A dashboard started by a
/// service manager can inherit a nested or unset `TMPDIR` (a scratch
/// subdirectory, or `/tmp`), while a benchmark writes its root under the
/// user's real temp tree. The stable roots below keep the rule independent of
/// the inspecting process's environment.
fn temp_prefixes() -> &'static [PathBuf] {
    static PREFIXES: OnceLock<Vec<PathBuf>> = OnceLock::new();
    PREFIXES.get_or_init(|| {
        let temp = std::env::temp_dir();
        let mut out = vec![temp.clone()];
        if let Ok(canonical) = temp.canonicalize() {
            if !out.contains(&canonical) {
                out.push(canonical);
            }
        }
        for root in [
            "/tmp",
            "/private/tmp",
            "/var/tmp",
            "/private/var/tmp",
            "/var/folders",
            "/private/var/folders",
        ] {
            let prefix = PathBuf::from(root);
            if !out.contains(&prefix) {
                out.push(prefix);
            }
        }
        out
    })
}

fn under_temp_dir(path: &Path) -> bool {
    temp_prefixes()
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

fn is_disposable(repo: Option<&str>, worktree: bool) -> bool {
    let Some(repo) = repo else {
        return false;
    };
    let path = Path::new(repo);
    if repo.is_empty() || !path.is_absolute() {
        return false;
    }
    if !worktree && !under_temp_dir(path) {
        return false;
    }
    !path_exists(path)
}

/// True when one raw journal entry should be dropped: its repository path is
/// gone and the root is a worktree or a temp-directory scratch repo.
pub(crate) fn entry_is_stale(entry: &Value) -> bool {
    is_disposable(
        entry.get("repo").and_then(Value::as_str),
        entry
            .get("worktree")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    )
}

impl UsageFields {
    /// The same rule for the typed folds, which never hold a raw `Value`.
    pub(crate) fn is_stale(&self) -> bool {
        is_disposable(self.repo.as_deref(), self.worktree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{remove_tree, temp_root};
    use serde_json::json;

    fn absent_under_temp(name: &str) -> PathBuf {
        temp_root(name).join("gone")
    }

    #[test]
    fn absent_temp_root_is_stale_without_worktree_flag() {
        let path = absent_under_temp("liveness-temp");
        let entry = json!({ "repo": path.to_string_lossy(), "worktree": false });
        assert!(entry_is_stale(&entry));
    }

    #[test]
    fn absent_stable_temp_root_is_stale() {
        // Independent of $TMPDIR: a benchmark root under the user temp tree
        // must stay disposable even when the inspector has a nested TMPDIR.
        let entry = json!({ "repo": "/var/folders/zz/eg-bench-1", "worktree": false });
        assert!(entry_is_stale(&entry));
    }

    #[test]
    fn existing_temp_root_is_live() {
        let root = temp_root("liveness-live");
        let entry = json!({ "repo": root.to_string_lossy(), "worktree": false });
        assert!(!entry_is_stale(&entry));
        remove_tree(&root);
    }

    #[test]
    fn absent_worktree_is_stale_outside_temp() {
        let entry = json!({ "repo": "/nonexistent-eg-liveness/wt", "worktree": true });
        assert!(entry_is_stale(&entry));
    }

    #[test]
    fn absent_ordinary_repo_keeps_its_history() {
        let entry = json!({ "repo": "/nonexistent-eg-liveness/archived", "worktree": false });
        assert!(!entry_is_stale(&entry));
    }

    #[test]
    fn relative_repo_is_never_judged() {
        let entry = json!({ "repo": ".tmp/verify/repo2", "worktree": true });
        assert!(!entry_is_stale(&entry));
    }

    #[test]
    fn entry_without_repo_is_live() {
        assert!(!entry_is_stale(&json!({ "tool": "build" })));
        assert!(!entry_is_stale(&json!({ "repo": "", "worktree": true })));
    }
}
