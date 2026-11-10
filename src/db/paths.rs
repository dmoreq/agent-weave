//! Confinement path builders for the repo-local `.engram/` directory. Pure
//! path arithmetic and a best-effort size probe; no directory creation lives
//! here, so the write-confinement invariant stays in `db.rs`.

use std::path::{Path, PathBuf};

pub fn eg_dir(root: &Path) -> PathBuf {
    root.join(".engram")
}

pub fn db_path(root: &Path) -> PathBuf {
    eg_dir(root).join("index.db")
}

/// Best-effort byte size of one file; `0` when the path is absent or
/// unreadable — size lines are presentation, never a hard failure.
pub(crate) fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

pub fn db_new_path(root: &Path) -> PathBuf {
    eg_dir(root).join("index.db.new")
}
