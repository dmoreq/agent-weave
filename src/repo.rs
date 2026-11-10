//! Canonical repository identity.
//!
//! Every surface used to derive a repository label on its own: the journal took
//! `root.file_name()`, the dashboard fell back to `"workspace"`, the monitor to
//! `"workspace"`, the reuse panel to `"repo"`. That produced one row per spelling
//! and let a subdirectory stand in for a repository.
//!
//! One resolver now answers "which repository is this?", and consumers read the
//! answer instead of guessing. The identity is the canonical absolute path; the
//! name is only a label.

use std::path::{Path, PathBuf};

use crate::error::{EgError, Result};
use crate::walk::{discover_root, find_git_root};

/// Bucket for calls that resolved no repository at all.
pub const UNATTRIBUTED: &str = "(unattributed)";

/// One resolved repository: canonical root, aggregation key, display label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRef {
    root: PathBuf,
    name: String,
    worktree: bool,
}

impl RepoRef {
    /// Resolve the repository for one invocation.
    ///
    /// An explicit path inside a git work tree belongs to that repository, so a
    /// subdirectory resolves to its git root instead of inventing a repository
    /// named after the directory. A directory outside any work tree is its own
    /// repository, which keeps a plain temp root usable. The filesystem root is
    /// refused upstream by `discover_root`.
    pub fn resolve(explicit: Option<&Path>) -> Result<Self> {
        let start = discover_root(explicit)?;
        // Walk from an absolute path: a relative start makes the git-root walk
        // terminate on the empty path, which is not a root at all.
        let start = std::path::absolute(&start).unwrap_or(start);
        // A directory that already carries its own `.engram/` is a repository in
        // its own right, even when it sits inside a larger work tree: a nested
        // project or a test fixture must not have its writes redirected to the
        // enclosing repository.
        let root = if start.join(".engram").is_dir() {
            start
        } else {
            find_git_root(&start).unwrap_or(start)
        };
        if root.parent().is_none() {
            return Err(EgError::Usage(format!(
                "{} is the filesystem root — pass --root <repository>",
                root.display()
            )));
        }
        // A symlinked workspace is one repository under its real path; a path
        // that no longer exists keeps its literal form.
        Ok(Self::from_root(&root))
    }

    pub fn from_root(root: &Path) -> Self {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| UNATTRIBUTED.to_string());
        let worktree = root.join(".git").is_file();
        Self {
            root,
            name,
            worktree,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Display label. Consumers disambiguate collisions; the journal keeps the
    /// plain basename.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_worktree(&self) -> bool {
        self.worktree
    }

    /// `repo` for the journal line: the canonical path, or `None` when the
    /// invocation resolved no repository.
    pub fn journal_repo(&self) -> Option<String> {
        if self.root.as_os_str().is_empty() {
            None
        } else {
            Some(self.root.to_string_lossy().into_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{make_dir_all, remove_tree, temp_root, write_file};

    fn git_root(name: &str) -> PathBuf {
        let root = temp_root(name);
        make_dir_all(root.join(".git")).unwrap();
        root
    }

    #[test]
    fn subdirectory_resolves_to_its_git_root() {
        let root = git_root("repo-subdir");
        let nested = root.join("a").join("b");
        make_dir_all(&nested).unwrap();

        let from_sub = RepoRef::resolve(Some(&nested)).unwrap();
        let from_root = RepoRef::resolve(Some(&root)).unwrap();
        assert_eq!(from_sub.root(), from_root.root());
        assert_eq!(from_sub.name(), from_root.name());

        remove_tree(&root);
    }

    #[test]
    fn directory_outside_a_work_tree_is_its_own_repository() {
        let dir = temp_root("repo-plain");
        let resolved = RepoRef::resolve(Some(&dir)).unwrap();
        assert!(resolved.journal_repo().is_some());
        assert!(resolved.root().to_string_lossy().contains("repo-plain"));
        assert!(!resolved.is_worktree());

        remove_tree(&dir);
    }

    #[test]
    fn filesystem_root_is_refused() {
        let err = RepoRef::resolve(Some(Path::new("/"))).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn linked_worktree_is_flagged() {
        let root = temp_root("repo-wt");
        // A linked worktree carries `.git` as a file, not a directory.
        write_file(root.join(".git"), b"gitdir: /elsewhere\n").unwrap();

        let resolved = RepoRef::resolve(Some(&root)).unwrap();
        assert!(resolved.is_worktree());

        remove_tree(&root);
    }

    #[test]
    fn relative_root_uses_the_absolute_repository_path() {
        // `.` resolves against the process working tree, which is this repo.
        let from_dot = RepoRef::resolve(Some(Path::new("."))).unwrap();
        assert!(from_dot.journal_repo().is_some());
        assert!(from_dot.root().is_absolute());
    }

    #[test]
    fn nested_store_keeps_its_own_root() {
        // A directory with its own `.engram/` inside a work tree is its own
        // repository; resolving it must not redirect writes to the enclosing one.
        let outer = git_root("repo-nested-outer");
        let inner = outer.join("tools").join("nested");
        make_dir_all(inner.join(".engram")).unwrap();

        let resolved = RepoRef::resolve(Some(&inner)).unwrap();
        let enclosing = RepoRef::resolve(Some(&outer)).unwrap();
        assert_ne!(resolved.root(), enclosing.root());
        assert!(resolved.root().ends_with("nested"));

        remove_tree(&outer);
    }
}
