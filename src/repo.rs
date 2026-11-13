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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

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

/// Main repository root for a linked git worktree, read from its `.git`
/// pointer file (`gitdir: <main>/.git/worktrees/<name>`). `None` for ordinary
/// repositories (`.git` is a directory), submodules (the pointer lands under
/// `.git/modules/`), and dangling or malformed pointers. The dashboard folds a
/// worktree's telemetry into its main repository with this answer, so the
/// answer is read from the pointer file without spawning git.
pub fn worktree_main_root(root: &Path) -> Option<PathBuf> {
    let pointer = root.join(".git");
    let meta = std::fs::symlink_metadata(&pointer).ok()?;
    if !meta.is_file() {
        // An ordinary repository carries `.git` as a directory; anything else
        // here is not a worktree pointer either.
        return None;
    }
    let content = std::fs::read_to_string(&pointer).ok()?;
    let gitdir = PathBuf::from(content.strip_prefix("gitdir:")?.trim());
    let gitdir = if gitdir.is_absolute() {
        gitdir
    } else {
        root.join(gitdir)
    };
    // Layout: <main>/.git/worktrees/<name>. A `modules` segment means a
    // submodule pointer, which belongs to no worktree.
    let name_dir = gitdir.parent()?;
    if name_dir.file_name()? != "worktrees" {
        return None;
    }
    let main_git = name_dir.parent()?;
    if main_git.file_name()? != ".git" {
        return None;
    }
    let main = main_git.parent()?.to_path_buf();
    // Relative pointers leave `..` segments in the derived root; the journal
    // records canonical paths, so the answer must be canonical too.
    let main = std::fs::canonicalize(&main).unwrap_or(main);
    if main == root || !main.join(".git").is_dir() || !gitdir.is_dir() {
        return None;
    }
    Some(main)
}

/// Upper bound on remembered worktree verdicts; mirrors the liveness cache.
const WORKTREE_CACHE_CAP: usize = 512;

/// Cached `worktree_main_root`. The dashboard evaluates one journal entry per
/// line on every poll and the pointer file never changes within a session, so
/// one verdict per distinct root per process is enough; the map clears when it
/// hits the cap instead of growing without limit.
pub fn cached_worktree_main_root(root: &Path) -> Option<PathBuf> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Option<PathBuf>>>> = OnceLock::new();
    let cell = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = root.to_path_buf();
    let mut guard = match cell.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if !guard.contains_key(&key) && guard.len() >= WORKTREE_CACHE_CAP {
        guard.clear();
    }
    if let Some(hit) = guard.get(&key) {
        return hit.clone();
    }
    let verdict = worktree_main_root(root);
    guard.insert(key, verdict.clone());
    verdict
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

    fn worktree_fixture(name: &str) -> (PathBuf, PathBuf) {
        // Synthetic linked-worktree layout without spawning git: a main repo
        // with a `.git` directory and a worktree whose pointer names the
        // main-side admin directory.
        let main = temp_root(name);
        make_dir_all(main.join(".git/worktrees/wt")).unwrap();
        let wt = temp_root(&format!("{name}-wt"));
        let pointer = format!("gitdir: {}/.git/worktrees/wt\n", main.display());
        write_file(wt.join(".git"), pointer.as_bytes()).unwrap();
        (main, wt)
    }

    #[test]
    fn linked_worktree_resolves_to_its_main_repository() {
        let (main, wt) = worktree_fixture("wt-resolve");
        // The answer is canonical: temp roots spell /var/folders while the
        // journal records /private/var/folders on macOS.
        let expected = std::fs::canonicalize(&main).unwrap_or_else(|_| main.clone());
        assert_eq!(worktree_main_root(&wt), Some(expected.clone()));
        assert_eq!(cached_worktree_main_root(&wt), Some(expected));
        remove_tree(&main);
        remove_tree(&wt);
    }

    #[test]
    fn relative_pointer_resolves_against_the_worktree() {
        let (main, wt) = worktree_fixture("wt-relative");
        // git may store the pointer relative to the worktree root.
        let rel = format!(
            "gitdir: ../{}/.git/worktrees/wt\n",
            main.file_name().unwrap().to_string_lossy()
        );
        write_file(wt.join(".git"), rel.as_bytes()).unwrap();
        let expected = std::fs::canonicalize(&main).unwrap_or_else(|_| main.clone());
        assert_eq!(worktree_main_root(&wt), Some(expected));
        remove_tree(&main);
        remove_tree(&wt);
    }

    #[test]
    fn submodule_pointer_is_not_a_worktree() {
        let (main, wt) = worktree_fixture("wt-submodule");
        // Submodules point into `.git/modules/<name>`, not `.git/worktrees/`.
        let modules = main.join(".git/modules/sub");
        make_dir_all(&modules).unwrap();
        let pointer = format!("gitdir: {}\n", modules.display());
        write_file(wt.join(".git"), pointer.as_bytes()).unwrap();
        assert_eq!(worktree_main_root(&wt), None);
        remove_tree(&main);
        remove_tree(&wt);
    }

    #[test]
    fn ordinary_repository_and_dangling_pointer_are_not_worktrees() {
        let (main, wt) = worktree_fixture("wt-ordinary");
        // The main side has `.git` as a directory: not a worktree.
        assert_eq!(worktree_main_root(&main), None);
        // A pointer to a removed admin directory is dead.
        remove_tree(main.join(".git/worktrees/wt"));
        assert_eq!(worktree_main_root(&wt), None);
        remove_tree(&main);
        remove_tree(&wt);
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
