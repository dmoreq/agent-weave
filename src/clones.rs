//! Clone-candidate shingling (Amendment 12d): normalized-line FNV-1a windows
//! computed during the build parse phase — workers already hold the file
//! bytes, so the pass adds no extra reads. Rows are (line_start, line_end,
//! shingle_hash) per fresh file; grouping/merging into clone pairs happens at
//! query time (`kg clones`, src/query.rs).

/// One shingle row: (line_start, line_end, FNV-1a window hash), 1-based
/// inclusive file line numbers.
pub type Shingle = (i64, i64, i64);

/// Per-file shingles: (inserted file id, window rows in file order).
pub type FileShingles = (i64, Vec<Shingle>);

/// One fresh-file parse unit: DB row, extraction rows, and the file's
/// shingles (worker-local; merged in spawn order by the build).
pub type ParseUnit = (
    crate::db::FilesRow,
    crate::extract::FileExtraction,
    Vec<Shingle>,
);

/// Window size in kept (non-blank, trimmed) lines. The query-side
/// `--min-lines` filter merges adjacent matching windows, so spans longer
/// than the window are detected without storing multiple window sizes.
pub const WINDOW: usize = 5;

/// Sliding-window shingles over normalized lines: trim trailing whitespace,
/// drop blank lines, hash each WINDOW-line run with the crate's FNV-1a.
/// Returns (line_start, line_end, hash) with 1-based INCLUSIVE file line
/// numbers of the first and last kept line in the window.
pub fn shingles(bytes: &[u8]) -> Vec<(i64, i64, i64)> {
    let text = String::from_utf8_lossy(bytes);
    let mut kept: Vec<(i64, &[u8])> = Vec::new();
    for (i, line) in text.as_bytes().split(|b| *b == b'\n').enumerate() {
        let trimmed: &[u8] = {
            let mut end = line.len();
            while end > 0
                && (line[end - 1] == b' ' || line[end - 1] == b'\t' || line[end - 1] == b'\r')
            {
                end -= 1;
            }
            &line[..end]
        };
        if trimmed.is_empty() {
            continue;
        }
        kept.push((i as i64 + 1, trimmed));
    }
    if kept.len() < WINDOW {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(kept.len() - WINDOW + 1);
    let mut buf: Vec<u8> = Vec::with_capacity(WINDOW * 32);
    for w in kept.windows(WINDOW) {
        buf.clear();
        for (j, (_, line)) in w.iter().enumerate() {
            if j > 0 {
                buf.push(b'\n');
            }
            buf.extend_from_slice(line);
        }
        let hash = crate::hash::fnv1a64(&buf);
        out.push((w[0].0, w[WINDOW - 1].0, hash as i64));
    }
    out
}
