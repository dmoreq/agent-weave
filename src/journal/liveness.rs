//! Staleness rule for usage-journal entries.
//!
//! One predicate decides whether an entry names a repository that is gone:
//! `repo` is an absolute path (never `/`) whose root is no longer a real
//! directory on disk. Symlinks count as gone: the journal records canonical
//! paths, so a journaled root that has become a symlink now aliases a
//! different repository, and the repository that lived there is defunct.
//! The CLI folds, the dashboard read path, the live event feeds, and the
//! pruner all call it, so no surface can disagree about which
//! repositories are still live.
//!
//! A stale entry is dropped by every reader and physically removed from the
//! log by the pruner (`eg dashboard` start and `eg doctor`), so a deleted
//! repository disappears from the totals as well as the tables. Entries with
//! no `repo` identity at all (missing, empty, or relative — legacy binaries
//! and test artifacts) are unjudgeable by the same token: the pruner removes
//! them, the readers skip them, and the writer no longer produces them.
//!
//! Relative `repo` values are never judged as live. `--root .tmp/x` means
//! different things from different working directories, so a liveness answer
//! would depend on which process asks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;

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

    fn is_real_dir(&mut self, path: &Path) -> bool {
        if self.checked_at.elapsed() >= CACHE_TTL || self.verdicts.len() >= CACHE_CAP {
            self.verdicts.clear();
            self.checked_at = Instant::now();
        }
        if let Some(&hit) = self.verdicts.get(path) {
            return hit;
        }
        // lstat semantics: a real directory only. A file is not a repository,
        // and a symlink is not the repository the journal path named — the
        // target is a different repository identity.
        let verdict = std::fs::symlink_metadata(path)
            .map(|meta| meta.is_dir())
            .unwrap_or(false);
        self.verdicts.insert(path.to_path_buf(), verdict);
        verdict
    }
}

fn cache() -> &'static Mutex<LivenessCache> {
    static CACHE: OnceLock<Mutex<LivenessCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(LivenessCache::new()))
}

fn cached_is_dir(path: &Path) -> bool {
    match cache().lock() {
        Ok(mut guard) => guard.is_real_dir(path),
        Err(poisoned) => poisoned.into_inner().is_real_dir(path),
    }
}

/// A journal `repo` value that can be judged at all: non-empty, absolute, and
/// not the filesystem root (which `RepoRef::resolve` refuses upstream).
fn judgeable_root(repo: &str) -> bool {
    !repo.is_empty() && Path::new(repo).is_absolute() && Path::new(repo) != Path::new("/")
}

/// True when `repo` names an absolute repository directory that still exists
/// as a real directory. Every repo row must satisfy this; relative, empty,
/// root, deleted, non-directory, and symlinked paths are not a live inventory
/// entry — a symlink aliases a different repository identity. The filesystem
/// verdict is cached, so a polling dashboard pays one stat per distinct root
/// per cache window.
pub(crate) fn repo_is_live(repo: Option<&str>) -> bool {
    match repo {
        Some(repo) if judgeable_root(repo) => cached_is_dir(Path::new(repo)),
        _ => false,
    }
}

/// True when one journal entry names a root that can be judged and is gone.
/// Such an entry is dropped by every reader and purged from the log.
fn is_dead_root(repo: Option<&str>) -> bool {
    match repo {
        Some(repo) if judgeable_root(repo) => !cached_is_dir(Path::new(repo)),
        _ => false,
    }
}

/// True when one raw journal entry should be dropped: its absolute repository
/// root no longer exists on disk.
pub(crate) fn entry_is_stale(entry: &Value) -> bool {
    is_dead_root(entry.get("repo").and_then(Value::as_str))
}

/// True when one raw journal entry carries no judgeable repository identity:
/// the `repo` field is missing, empty, or relative. Legacy binaries wrote such
/// lines; the current writer never does. The pruner removes them and every
/// reader skips them.
pub(crate) fn entry_is_unattributed(entry: &Value) -> bool {
    !judgeable_root(entry.get("repo").and_then(Value::as_str).unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{make_dir_all, make_symlink, remove_tree, temp_root, write_file};
    use serde_json::json;

    #[test]
    fn absent_temp_root_is_stale() {
        let path = temp_root("liveness-temp").join("gone");
        let entry = json!({ "repo": path.to_string_lossy(), "worktree": false });
        assert!(entry_is_stale(&entry));
    }

    #[test]
    fn absent_stable_temp_root_is_stale() {
        // Independent of $TMPDIR: a benchmark root under the user temp tree
        // must stay stale even when the inspector has a nested TMPDIR.
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
    fn absent_worktree_is_stale() {
        let entry = json!({ "repo": "/nonexistent-eg-liveness/wt", "worktree": true });
        assert!(entry_is_stale(&entry));
    }

    #[test]
    fn absent_ordinary_repo_is_stale() {
        // Purge policy: a deleted repository is dropped by every reader and
        // removed from the log by the pruner; totals shrink with it.
        let entry = json!({ "repo": "/nonexistent-eg-liveness/archived", "worktree": false });
        assert!(entry_is_stale(&entry));
    }

    #[test]
    fn file_at_repo_path_is_stale() {
        // A file left where a repository root used to be is not a repository.
        let root = temp_root("liveness-file");
        let file = root.join("not-a-repo");
        write_file(&file, b"x").unwrap();
        let entry = json!({ "repo": file.to_string_lossy() });
        assert!(entry_is_stale(&entry));
        assert!(!repo_is_live(Some(file.to_string_lossy().as_ref())));
        remove_tree(&root);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_repo_path_is_stale() {
        // A journal root that has become a symlink aliases a different
        // repository: the identity that lived there is defunct, even though
        // the link resolves to a live directory.
        let root = temp_root("liveness-symlink");
        let target = root.join("real-repo");
        make_dir_all(&target).unwrap();
        let link = root.join("alias");
        make_symlink(&target, &link).unwrap();

        let entry = json!({ "repo": link.to_string_lossy() });
        assert!(entry_is_stale(&entry), "symlink-to-dir must be stale");
        assert!(!repo_is_live(Some(link.to_string_lossy().as_ref())));
        // The target itself stays live.
        assert!(repo_is_live(Some(target.to_string_lossy().as_ref())));

        // A dangling symlink is stale too.
        let dangling = root.join("dangling");
        make_symlink(root.join("gone"), &dangling).unwrap();
        assert!(entry_is_stale(
            &json!({ "repo": dangling.to_string_lossy() })
        ));
        remove_tree(&root);
    }

    #[test]
    fn relative_repo_is_never_judged() {
        let entry = json!({ "repo": ".tmp/verify/repo2", "worktree": true });
        assert!(!entry_is_stale(&entry));
    }

    #[test]
    fn identityless_entries_are_unattributed() {
        assert!(entry_is_unattributed(&json!({ "tool": "build" })));
        assert!(entry_is_unattributed(
            &json!({ "tool": "build", "repo": null })
        ));
        assert!(entry_is_unattributed(
            &json!({ "tool": "build", "repo": "" })
        ));
        assert!(entry_is_unattributed(
            &json!({ "tool": "build", "repo": "." })
        ));
        assert!(entry_is_unattributed(
            &json!({ "tool": "build", "repo": ".tmp/verify/repo2", "repo_name": "repo2" })
        ));
        let root = temp_root("liveness-unattributed");
        assert!(!entry_is_unattributed(
            &json!({ "tool": "build", "repo": root.to_string_lossy() })
        ));
        // A judgeable-but-dead root is stale, not unattributed.
        assert!(!entry_is_unattributed(
            &json!({ "tool": "build", "repo": "/nonexistent-eg-liveness/dead" })
        ));
        remove_tree(&root);
    }

    #[test]
    fn entry_without_repo_is_live() {
        assert!(!entry_is_stale(&json!({ "tool": "build" })));
        assert!(!entry_is_stale(&json!({ "repo": "", "worktree": true })));
    }

    #[test]
    fn existing_absolute_root_is_a_live_repo() {
        let root = temp_root("liveness-live-repo");
        assert!(repo_is_live(Some(root.to_string_lossy().as_ref())));
        remove_tree(&root);
    }

    #[test]
    fn deleted_absolute_root_is_not_live() {
        assert!(!repo_is_live(Some("/nonexistent-eg-liveness/deleted")));
    }

    #[test]
    fn relative_and_rootless_values_are_not_live_repos() {
        assert!(!repo_is_live(None));
        assert!(!repo_is_live(Some("")));
        assert!(!repo_is_live(Some("/")));
        assert!(!repo_is_live(Some(".tmp/verify/repo1")));
    }
}
