//! Fresh extraction pipeline: batch absorption, healed re-extraction, and
//! the parallel chunked extract workers.

use super::*;

/// Extract one healed batch and absorb it: move the files from the carried
/// pool to fresh, subtract their old error count from the carried pool
/// (they re-extract here, so their errors arrive via `fe.error_files`
/// instead), and append the extraction output. Returns the batch's rels so
/// the caller can seed the next dirty closure.
fn absorb_batch(
    cls: &mut Classified,
    carried_errors: &mut usize,
    fe: &mut FreshExtract,
    c: &rusqlite::Connection,
    batch: Vec<crate::walk::WalkedFile>,
) -> Result<Vec<String>> {
    let healed: Vec<String> = batch.iter().map(|f| f.rel.clone()).collect();
    *carried_errors -= crate::db::old_error_count(c, &healed)?;
    let base = cls.fresh.len();
    let mut res = extract_units(&batch, base)?;
    cls.fresh.extend(batch);
    fe.rows.append(&mut res.rows);
    fe.extractions.append(&mut res.extractions);
    fe.error_files += res.error_files;
    fe.secs += res.secs;
    Ok(healed)
}

/// Re-extract the healed batch, then re-apply the inbound closure to each
/// newly fresh set until it is stable (ANEW1).
pub(super) fn extract_healed(
    prep: &mut BuildPrep,
    fe: &mut FreshExtract,
    second: Vec<crate::walk::WalkedFile>,
) -> Result<()> {
    let Some(c) = prep.old.as_ref() else {
        return Ok(());
    };
    let mut healed = absorb_batch(&mut prep.cls, &mut prep.carried_errors, fe, c, second)?;

    // ANEW1: gap healing moves cls.carried files to cls.fresh after the
    // initial dirty closure. Their inbound callers are still
    // cls.carried and point at the old symbol ids, so re-apply the
    // inbound closure to each newly cls.fresh set until it is stable.
    while !healed.is_empty() {
        let inbound = crate::db::dirty_closure(c, &healed)?;
        let third = take_fresh_batch(
            inbound,
            &mut prep.cls.carried,
            &prep.by_rel,
            &prep.stats.files,
        );
        if third.is_empty() {
            break;
        }
        healed = absorb_batch(&mut prep.cls, &mut prep.carried_errors, fe, c, third)?;
    }
    Ok(())
}
/// Parse + extract one batch of walked files on scoped workers (Amendment 11c
/// parallel parse; R6d Blocker 1: 64 MB stacks). Trees never cross threads (not
/// Send); only owned extraction rows do. Chunks are contiguous and joined in
/// spawn order, so the merged order equals the serial loop's. `base_idx` keeps
/// the extraction file index aligned with the accumulated `fresh` vector.
pub(super) fn extract_units(
    files: &[crate::walk::WalkedFile],
    base_idx: usize,
) -> Result<FreshExtract> {
    let mut rows: Vec<crate::db::FilesRow> = Vec::with_capacity(files.len());
    let mut extractions: Vec<crate::extract::FileExtraction> = Vec::with_capacity(files.len());
    let mut error_files = 0usize;
    let mut extract_secs_sum = 0f64;
    let workers = worker_count();
    let chunk_len = (files.len().div_ceil(workers)).max(1);
    std::thread::scope(|scope| -> Result<()> {
        let mut handles = Vec::new();
        for (chunk_no, chunk) in files.chunks(chunk_len).enumerate() {
            let chunk = chunk.to_vec();
            let handle = std::thread::Builder::new().spawn_scoped(scope, move || {
                extract_chunk(chunk, base_idx, chunk_no, chunk_len)
            })?;
            handles.push(handle);
        }
        for h in handles {
            let (units, secs) = h
                .join()
                .map_err(|_| EgError::Internal("parse worker panicked".to_string()))??;
            extract_secs_sum += secs;
            merge_units(units, &mut rows, &mut extractions, &mut error_files);
        }
        Ok(())
    })?;
    Ok(FreshExtract {
        rows,
        extractions,
        error_files,
        secs: extract_secs_sum,
    })
}

/// Parse + extract one contiguous chunk on a scoped worker. `chunk_no` and
/// `chunk_len` place each file's extraction index at
/// `base_idx + chunk_no * chunk_len + local`, keeping it aligned with the
/// accumulated `fresh` vector (see `extract_units`).
fn extract_chunk(
    chunk: Vec<crate::walk::WalkedFile>,
    base_idx: usize,
    chunk_no: usize,
    chunk_len: usize,
) -> Result<(
    Vec<(crate::db::FilesRow, crate::extract::FileExtraction)>,
    f64,
)> {
    let mut units = Vec::with_capacity(chunk.len());
    let mut secs = 0f64;
    let mut parser = crate::parse::ParserSlot::new();
    for (local, f) in chunk.into_iter().enumerate() {
        let idx = base_idx + chunk_no * chunk_len + local;
        let (row, fx, one_secs) = extract_one(idx, f, &mut parser)?;
        secs += one_secs;
        units.push((row, fx));
    }
    Ok((units, secs))
}

fn worker_count() -> usize {
    std::env::var("ENGRAM_WORKERS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
        })
        .clamp(1, 8)
}

fn merge_units(
    units: Vec<(crate::db::FilesRow, crate::extract::FileExtraction)>,
    rows: &mut Vec<crate::db::FilesRow>,
    extractions: &mut Vec<crate::extract::FileExtraction>,
    error_files: &mut usize,
) {
    for (row, fx) in units {
        if row.error_nodes > 0 {
            *error_files += 1;
        }
        rows.push(row);
        extractions.push(fx);
    }
}

/// Parse and extract one walked file. Returns its row, extraction, and the
/// extraction-only elapsed seconds (read and parse are excluded, as before).
fn extract_one(
    idx: usize,
    file: crate::walk::WalkedFile,
    parser: &mut crate::parse::ParserSlot,
) -> Result<(crate::db::FilesRow, crate::extract::FileExtraction, f64)> {
    let bytes = crate::walk::read_bounded(&file.abs)?;
    // The walk allowlist guarantees a known extension; an unknown one is an
    // internal invariant break, not a silent JavaScript default.
    let lang = crate::parse::Lang::from_rel(&file.rel)
        .ok_or_else(|| EgError::Internal(format!("unsupported source extension: {}", file.rel)))?;
    let tree = parser.parse(&bytes, lang)?;
    let crate::parse::SyntaxStats {
        error_nodes,
        status,
    } = crate::parse::syntax_stats(tree.root_node());
    let e0 = Instant::now();
    let fx = crate::extract::extract_file_nodes(
        crate::extract::FileInput {
            file_idx: idx,
            rel: &file.rel,
            lang,
            src: &bytes,
        },
        tree.root_node(),
    );
    let secs = e0.elapsed().as_secs_f64();
    Ok((
        crate::db::FilesRow {
            rel: file.rel,
            size: file.size as i64,
            mtime: file.mtime,
            content_hash: crate::hash::fnv1a64(&bytes) as i64,
            parse_status: status,
            error_nodes,
            unresolved: 0,
        },
        fx,
        secs,
    ))
}
