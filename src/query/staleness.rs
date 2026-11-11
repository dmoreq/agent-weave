//! Result-set staleness: the shared changed-since-build header wording and
//! the (mtime, size) stat short-circuit with FNV-1a re-hash on mismatch.

use super::*;

/// Count result files changed since the build and render the standard header.
/// Every query surface uses the same wording and the same stat short-circuit.
pub(super) fn staleness_header(
    ctx: &QueryCtx<'_>,
    files: &BTreeSet<String>,
) -> (Option<String>, bool) {
    let mut changed = 0usize;
    for f in files {
        if file_changed(ctx, f) {
            changed += 1;
        }
    }
    let stale = changed > 0;
    let header = stale.then(|| {
        format!(
            "eg: {changed} of {} result files changed since build — run eg build",
            files.len()
        )
    });
    (header, stale)
}

/// Staleness: (mtime, size) stat short-circuit; FNV-1a re-hash on mismatch.
pub(crate) fn file_changed(ctx: &QueryCtx<'_>, rel: &str) -> bool {
    let QueryCtx { conn, root } = *ctx;
    let row = conn.query_row(
        "SELECT mtime, size, content_hash FROM files WHERE path = ?1",
        [rel],
        crate::db::ints3,
    );
    let Ok((stored_mtime, stored_size, stored_hash)) = row else {
        return false;
    };
    // Index rows are attacker-authored in a cloned repository. Never join an
    // absolute, parent-traversing, or backslash path onto the root, and never
    // follow a symlink out of the repository.
    if !safe_index_path(rel) {
        return true;
    }
    let abs = root.join(rel);
    let Ok(meta) = std::fs::symlink_metadata(&abs) else {
        return true;
    };
    if !meta.is_file() || meta.file_type().is_symlink() {
        return true;
    }
    if meta.len() != stored_size as u64 {
        return true;
    }
    let Some(mtime) = db::stat_mtime(&abs) else {
        return true;
    };
    if mtime != stored_mtime {
        // Same guard as the walk (regular file + hard 1 MiB cap): a FIFO,
        // socket, or over-cap replacement must not block or blow up the
        // re-hash. Any such oddity is conservatively treated as changed.
        let Ok(bytes) = crate::walk::read_bounded(&abs) else {
            return true;
        };
        let hash = fnv1a64(&bytes) as i64;
        return hash != stored_hash;
    }
    false
}

/// A repo-relative indexed path may contain only normal components. This
/// mirrors the write-path containment rule and rejects Windows separators on
/// every platform so an index built elsewhere cannot escape the root.
fn safe_index_path(rel: &str) -> bool {
    if rel.is_empty() || rel.contains('\\') {
        return false;
    }
    let path = std::path::Path::new(rel);
    if path.is_absolute() {
        return false;
    }
    path.components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::safe_index_path;

    #[test]
    fn index_paths_must_stay_relative_and_contained() {
        assert!(safe_index_path("src/main.rs"));
        assert!(safe_index_path("a/b/c.ts"));
        assert!(!safe_index_path(""));
        assert!(!safe_index_path("/etc/passwd"));
        assert!(!safe_index_path("../outside"));
        assert!(!safe_index_path("a/../../outside"));
        assert!(!safe_index_path("a\\b"));
    }
}
