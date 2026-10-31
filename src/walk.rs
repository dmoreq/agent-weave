//! Repository walk: `ignore` WalkBuilder from the git root discovered upward,
//! `.kg/` skipped unconditionally, per-file size cap 1 MB (skipped + counted),
//! extension allowlist per language generation.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::error::{KgError, Result};

pub const MAX_FILE_SIZE: u64 = 1024 * 1024;

/// P2 extension allowlist: Python + TypeScript/TSX + JavaScript/JSX.
pub const SOURCE_EXTS: &[&str] = &["py", "ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"];

pub struct WalkedFile {
    /// Absolute path on disk (for reading and stat'ing).
    pub abs: PathBuf,
    /// Repo-root-relative path with forward slashes (stored in the index).
    pub rel: String,
    pub size: u64,
}

pub struct WalkStats {
    pub files: Vec<WalkedFile>,
    /// Files skipped for exceeding the size cap (counted, per spec).
    pub skipped_large: usize,
}

pub fn discover_root(explicit: Option<&Path>) -> Result<PathBuf> {
    match explicit {
        Some(p) => {
            if !p.is_dir() {
                return Err(KgError::Usage(format!("--root {} is not a directory", p.display())));
            }
            Ok(p.to_path_buf())
        }
        None => {
            let cwd = std::env::current_dir()
                .map_err(|e| KgError::Internal(format!("cannot read cwd: {e}")))?;
            find_git_root(&cwd).ok_or_else(|| {
                KgError::Usage("no git root ancestor found — pass --root <path>".to_string())
            })
        }
    }
}

fn find_git_root(start: &Path) -> Option<PathBuf> {
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
pub fn walk(root: &Path, exts: &[&str]) -> Result<WalkStats> {
    let mut builder = WalkBuilder::new(root);
    // Respect .gitignore/.ignore (node_modules and build dirs vanish here).
    // Hidden files are kept so dot-directories are governed solely by the
    // explicit .kg/ filter below, not by ignore's hidden heuristic.
    builder.hidden(false);
    builder.filter_entry(|entry| entry.file_name() != std::ffi::OsStr::new(".kg"));

    // The ignore crate applies gitignore rules textually and skips
    // TRACKED-but-gitignored files, which git itself still versions. Query the
    // git index once and keep exactly the git-visible set (tracked +
    // untracked-unignored). No git / probe failure -> keep the textual rules.
    let git_visible: Option<HashSet<String>> = (|| {
        let out = std::process::Command::new("git")
            .args(["ls-files", "-co", "--exclude-standard"])
            .current_dir(root)
            .output()
            .ok()
            .filter(|o| o.status.success())?;
        let set: HashSet<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.replace('\\', "/"))
            .collect();
        Some(set)
    })();

    let mut files = Vec::new();
    let mut skipped_large = 0usize;
    for entry in builder.build() {
        let entry = entry.map_err(|e| KgError::Internal(format!("walk error: {e}")))?;
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
        if let Some(set) = &git_visible {
            if !set.contains(&rel) {
                continue;
            }
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if size > MAX_FILE_SIZE {
            skipped_large += 1;
            continue;
        }
        files.push(WalkedFile { abs: path.to_path_buf(), rel, size });
    }
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(WalkStats { files, skipped_large })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn walk_skips_kg_dir_and_large_files_and_filters_ext() {
        let dir = std::env::temp_dir().join(format!("kg-walk-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write(&dir.join("a.py"), b"x = 1\n");
        write(&dir.join("sub/b.py"), b"y = 2\n");
        write(&dir.join("sub/c.txt"), b"not indexed\n");
        write(&dir.join(".kg/index.db"), b"stale");
        write(&dir.join("sub/huge.py"), &[b'a'; MAX_FILE_SIZE as usize + 1]);

        let stats = walk(&dir, &["py"]).unwrap();
        let rels: Vec<&str> = stats.files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(rels, vec!["a.py", "sub/b.py"]);
        assert_eq!(stats.skipped_large, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_root_explicit_bad_dir_is_usage() {
        let err = discover_root(Some(Path::new("/definitely/not/a/dir"))).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }
}
