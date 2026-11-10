//! Repository walk: `ignore` WalkBuilder from the git root discovered upward,
//! `.engram/` skipped unconditionally, per-file size cap 1 MB (skipped + counted),
//! extension allowlist per language generation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::error::{EgError, Result};

pub const MAX_FILE_SIZE: u64 = 1024 * 1024;

/// Walk allowlist (P2: Python + TypeScript/TSX + JavaScript/JSX + Rust),
/// derived from the master extension table `parse::SOURCE_EXTENSIONS` — one
/// row there adds the extension to the walk, `Lang::from_rel`, module stems,
/// and the query guards.
pub fn source_exts() -> Vec<&'static str> {
    crate::parse::source_ext_names().collect()
}

#[derive(Clone)]
pub struct WalkedFile {
    /// Absolute path on disk (for reading and stat'ing).
    pub abs: PathBuf,
    /// Repo-root-relative path with forward slashes (stored in the index).
    pub rel: String,
    pub size: u64,
    /// Nanosecond-precision stat mtime captured during the walk, so the build
    /// does not stat the same path again for classification and the files row.
    pub mtime: i64,
}

pub struct WalkStats {
    pub files: Vec<WalkedFile>,
    /// Files skipped for exceeding the size cap (counted, per spec).
    pub skipped_large: usize,
}

pub fn discover_root(explicit: Option<&Path>) -> Result<PathBuf> {
    let root = match explicit {
        Some(p) => {
            if !p.is_dir() {
                return Err(EgError::Usage(format!(
                    "--root {} is not a directory",
                    p.display()
                )));
            }
            p.to_path_buf()
        }
        None => {
            let cwd = std::env::current_dir()
                .map_err(|e| EgError::Internal(format!("cannot read cwd: {e}")))?;
            find_git_root(&cwd).ok_or_else(|| {
                EgError::Usage("no git root ancestor found — pass --root <path>".to_string())
            })?
        }
    };
    reject_filesystem_root(&root)?;
    Ok(root)
}

/// The filesystem root is never a project. Every surface would create its
/// `.engram/` there: an MCP client that reports `rootUri` of `/`, or a file
/// argument outside any repository, used to reach this and fail later with an
/// opaque I/O error. Refuse it at resolution time with the usage contract.
fn reject_filesystem_root(root: &Path) -> Result<()> {
    if root.parent().is_none() {
        return Err(EgError::Usage(format!(
            "{} is the filesystem root — pass --root <repository>",
            root.display()
        )));
    }
    Ok(())
}

pub(crate) fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut cur = start.to_path_buf();
    loop {
        if cur.join(".git").exists() {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

/// Walk `root` collecting files whose extension is in `exts`.
///
/// When `git ls-files` succeeds, the repository-relative path list is the
/// source of truth: tracked plus untracked-unignored files, which is exactly
/// the set `git` would show. Iterating that list avoids descending ignored
/// trees (`target/`, `node_modules/`, generated output), so a huge ignored
/// corpus costs no directory traversal and an unreadable ignored directory
/// cannot fail the build. `WalkBuilder` remains the fallback when git is
/// absent or the probe fails.
pub fn walk(root: &Path, exts: &[&str]) -> Result<WalkStats> {
    if let Some(rels) = git_visible_paths(root) {
        return Ok(walk_git_visible(root, &rels, exts));
    }
    walk_fallback(root, exts)
}

/// `git ls-files -co --exclude-standard -z`: tracked plus untracked-unignored
/// paths, repository-relative with forward slashes. `None` when git is
/// unavailable or the command fails.
fn git_visible_paths(root: &Path) -> Option<Vec<String>> {
    let out = std::process::Command::new("git")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(["ls-files", "-co", "--exclude-standard", "-z"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    Some(
        String::from_utf8_lossy(&out.stdout)
            .split('\0')
            .filter(|l| !l.is_empty())
            .map(|l| l.replace('\\', "/"))
            .collect(),
    )
}

/// Shared walk tail: one deterministic path order, one stats shape.
fn finish_walk(mut files: Vec<WalkedFile>, skipped_large: usize) -> WalkStats {
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    WalkStats {
        files,
        skipped_large,
    }
}

/// One candidate path's guard-chain verdict in [`walk_git_visible`]: index the
/// file with this metadata, skip it, or skip it as over-cap (counted for the
/// build report).
enum WalkVerdict {
    Index(std::fs::Metadata),
    Skip,
    OverCap,
}

/// The guard chain for one git-visible path: `.engram/` membership,
/// extension allowlist, ancestor-symlink containment, symlink leaf, non-file,
/// and size cap. The metadata is stat'ed exactly once, here.
fn walk_verdict(
    rel: &str,
    root: &Path,
    exts: &[&str],
    root_canon: &Path,
    parent_contained: &mut HashMap<PathBuf, bool>,
) -> WalkVerdict {
    // Whole-segment equality keeps application paths such as `src/engram/`
    // indexed while both confinement directories stay out of the graph.
    if rel.split('/').any(|seg| seg == ".engram") {
        return WalkVerdict::Skip;
    }
    let ext = Path::new(rel)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if !exts.contains(&ext) {
        return WalkVerdict::Skip;
    }
    let path = root.join(rel);
    // An ancestor component can be a symlink even when the leaf is a
    // regular file. Check the canonical parent stays inside the canonical
    // root before any metadata or read, so a tracked path cannot expose
    // content outside the repository.
    let parent = path.parent().unwrap_or(root);
    let contained = *parent_contained
        .entry(parent.to_path_buf())
        .or_insert_with(|| {
            std::fs::canonicalize(parent)
                .map(|p| p.starts_with(root_canon))
                .unwrap_or(false)
        });
    if !contained {
        return WalkVerdict::Skip;
    }
    // symlink_metadata does not follow links, so a symlink to a source
    // file is skipped rather than indexed under a second path.
    let Ok(metadata) = std::fs::symlink_metadata(&path) else {
        return WalkVerdict::Skip;
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return WalkVerdict::Skip;
    }
    if metadata.len() > MAX_FILE_SIZE {
        return WalkVerdict::OverCap;
    }
    WalkVerdict::Index(metadata)
}

/// Stat each git-visible path once. Missing paths (staged deletions), symlinks,
/// directories (submodules), `.engram/`, unsupported extensions, and files over
/// `MAX_FILE_SIZE` are skipped; the size cap is counted for the build report.
fn walk_git_visible(root: &Path, rels: &[String], exts: &[&str]) -> WalkStats {
    let mut files = Vec::new();
    let mut skipped_large = 0usize;
    let root_canon = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut parent_contained: HashMap<PathBuf, bool> = HashMap::new();
    for rel in rels {
        let metadata = match walk_verdict(rel, root, exts, &root_canon, &mut parent_contained) {
            WalkVerdict::Index(metadata) => metadata,
            WalkVerdict::Skip => continue,
            WalkVerdict::OverCap => {
                skipped_large += 1;
                continue;
            }
        };
        // The single nanosecond arithmetic lives in db::metadata_mtime; the
        // walk passes its already-stat'ed metadata so no path is stat'ed twice.
        let mtime = crate::db::metadata_mtime(&metadata).unwrap_or(0);
        files.push(WalkedFile {
            abs: root.join(rel),
            rel: rel.clone(),
            size: metadata.len(),
            mtime,
        });
    }
    finish_walk(files, skipped_large)
}

/// Shared "open a plain file" frame: reject symlinks and special files via
/// `symlink_metadata` before opening; a path whose metadata is unavailable
/// falls through to the open error. `Ok(None)` is a rejected non-regular
/// path; callers keep their own cap and error contract on top.
pub(crate) fn open_regular_file(path: &Path) -> std::io::Result<Option<std::fs::File>> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if !meta.is_file() {
            return Ok(None);
        }
    }
    std::fs::File::open(path).map(Some)
}

/// Read a file with a hard allocation bound. The walk captured the size, but
/// the file can grow or be replaced before this read. `take(MAX_FILE_SIZE + 1)`
/// prevents an unbounded allocation, and an over-cap result is an error rather
/// than a truncated parse.
pub fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let not_regular = || EgError::Internal(format!("not a regular file: {}", path.display()));
    let file = match open_regular_file(path)? {
        Some(file) => file,
        None => return Err(not_regular()),
    };
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(not_regular());
    }
    if meta.len() > MAX_FILE_SIZE {
        return Err(EgError::Internal(format!(
            "file exceeds the 1 MiB cap after walk: {}",
            path.display()
        )));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    let mut reader = file.take(MAX_FILE_SIZE + 1);
    reader.read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err(EgError::Internal(format!(
            "file grew past the 1 MiB cap during read: {}",
            path.display()
        )));
    }
    Ok(bytes)
}

/// Fallback discovery when git cannot answer: `ignore`'s textual rules.
fn walk_fallback(root: &Path, exts: &[&str]) -> Result<WalkStats> {
    let mut builder = WalkBuilder::new(root);
    // Respect .gitignore/.ignore (node_modules and build dirs vanish here).
    // Hidden files are kept so dot-directories are governed solely by the
    // explicit confinement-directory filter below, not by ignore's hidden
    // heuristic.
    builder.hidden(false);
    builder.filter_entry(|entry| {
        let name = entry.file_name();
        name != std::ffi::OsStr::new(".engram")
    });

    let mut files = Vec::new();
    let mut skipped_large = 0usize;
    for entry in builder.build() {
        let entry = entry.map_err(|e| EgError::Internal(format!("walk error: {e}")))?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !exts.contains(&ext) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let metadata = entry.metadata().ok();
        let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
        if size > MAX_FILE_SIZE {
            skipped_large += 1;
            continue;
        }
        let mtime = metadata
            .as_ref()
            .and_then(crate::db::metadata_mtime)
            .unwrap_or(0);
        files.push(WalkedFile {
            abs: path.to_path_buf(),
            rel,
            size,
            mtime,
        });
    }
    Ok(finish_walk(files, skipped_large))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn walk_skips_eg_dir_and_large_files_and_filters_ext() {
        let dir = crate::db::temp_root("walk-test");
        write(&dir.join("a.py"), b"x = 1\n");
        write(&dir.join("sub/b.py"), b"y = 2\n");
        write(&dir.join("sub/c.txt"), b"not indexed\n");
        write(&dir.join(".engram/index.db"), b"stale");
        write(
            &dir.join("sub/huge.py"),
            &[b'a'; MAX_FILE_SIZE as usize + 1],
        );

        let stats = walk(&dir, &["py"]).unwrap();
        let rels: Vec<&str> = stats.files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(rels, vec!["a.py", "sub/b.py"]);
        assert_eq!(stats.skipped_large, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_bounded_rejects_over_cap() {
        let dir = crate::db::temp_root("walk-bounded");
        let path = dir.join("huge.py");
        write(&path, &[b'a'; MAX_FILE_SIZE as usize + 1]);
        let err = read_bounded(&path).unwrap_err();
        assert_eq!(err.exit_code(), 6);
        assert!(read_bounded(&dir.join("missing.py")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn walk_git_visible_skips_ancestor_symlink() {
        use std::os::unix::fs::symlink;
        let dir = crate::db::temp_root("walk-symlink");
        let external = crate::db::temp_root("walk-external");
        write(&dir.join("pkg/a.py"), b"def inside(): pass\n");
        let status = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&dir)
            .status()
            .unwrap();
        assert!(status.success());
        let status = std::process::Command::new("git")
            .args(["add", "pkg/a.py"])
            .current_dir(&dir)
            .status()
            .unwrap();
        assert!(status.success());

        write(&external.join("a.py"), b"def external_marker(): pass\n");
        std::fs::remove_dir_all(dir.join("pkg")).unwrap();
        symlink(&external, dir.join("pkg")).unwrap();

        let stats = walk(&dir, &["py"]).unwrap();
        assert!(
            stats.files.iter().all(|f| f.rel != "pkg/a.py"),
            "ancestor symlink was indexed: {:?}",
            stats.files.iter().map(|f| &f.rel).collect::<Vec<_>>()
        );
        assert!(stats.files.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&external);
    }

    #[test]
    fn discover_root_explicit_bad_dir_is_usage() {
        let err = discover_root(Some(Path::new("/definitely/not/a/dir"))).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn discover_root_filesystem_root_is_usage() {
        let err = discover_root(Some(Path::new("/"))).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(
            err.to_string().contains("filesystem root"),
            "message must name the cause: {err}"
        );
    }
}
