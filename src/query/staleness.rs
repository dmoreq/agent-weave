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
    let abs = root.join(rel);
    let Some(size) = db::stat_size(&abs) else {
        return true;
    };
    if size != stored_size {
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
