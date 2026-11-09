//! Build orchestration: full atomic replacement with the per-file fast path,
//! dirty closure, parse-quality gate, and meta manifest. Shared by the CLI
//! (prints the stats line) and the MCP server (returns it as tool text).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::error::{EgError, Result};

/// Move `rel` from carried to fresh when the dirty/gap closure names it.
fn take_fresh(
    rel: &str,
    carried: &mut HashSet<String>,
    by_rel: &HashMap<String, usize>,
    files: &[crate::walk::WalkedFile],
) -> Option<crate::walk::WalkedFile> {
    if !carried.remove(rel) {
        return None;
    }
    by_rel.get(rel).and_then(|&i| files.get(i)).cloned()
}

/// Drain a candidate set into a sorted fresh batch.
fn take_fresh_batch(
    rels: impl IntoIterator<Item = String>,
    carried: &mut HashSet<String>,
    by_rel: &HashMap<String, usize>,
    files: &[crate::walk::WalkedFile],
) -> Vec<crate::walk::WalkedFile> {
    let mut batch: Vec<crate::walk::WalkedFile> = rels
        .into_iter()
        .filter_map(|rel| take_fresh(&rel, carried, by_rel, files))
        .collect();
    batch.sort_by(|a, b| a.rel.cmp(&b.rel));
    batch
}

/// Stderr notice for the files skipped by the 1 MiB walk cap, empty when
/// none. It rides the stderr half `run_build` returns (the shared
/// `ArmOut.stderr` channel) instead of `eprintln!`, so the MCP surface keeps
/// the notice in its own output contract instead of leaking it into the
/// server process's stderr.
fn skipped_notice(stats: &crate::walk::WalkStats) -> String {
    if stats.skipped_large > 0 {
        format!(
            "[eg] skipped {} file(s) over the 1MB size cap\n",
            stats.skipped_large
        )
    } else {
        String::new()
    }
}

/// Run-level options and start timestamp shared by the build phases.
struct BuildOpts {
    force: bool,
    max_parse_error_rate: f64,
    started: Instant,
    walk_secs: f64,
}

/// Walk classification: unchanged (carried), hash-carried mtime refresh, and
/// files that need extraction.
struct Classified {
    carried: HashSet<String>,
    mtime_updates: HashMap<String, i64>,
    fresh: Vec<crate::walk::WalkedFile>,
}

/// Fresh extraction output plus its counters.
struct FreshExtract {
    rows: Vec<crate::db::FilesRow>,
    extractions: Vec<crate::extract::FileExtraction>,
    error_files: usize,
    secs: f64,
}

/// Fresh row ids and their position mapping, consumed by the FTS phase.
struct WriteIds {
    sym_ids: Vec<i64>,
    pos: Vec<Option<usize>>,
    paths: Vec<String>,
    sym_count: usize,
}

/// Per-phase wall timings for the final stats line.
struct PhaseSecs {
    parse: f64,
    extract: f64,
    db: f64,
}

/// Authoritative SQL totals for the final stats line.
struct MetaTotals {
    sym_total: i64,
    edge_total: i64,
    perr_total: i64,
    unres_total: i64,
}

/// Walked and classified build state, before extraction starts.
struct BuildPrep {
    opts: BuildOpts,
    stats: crate::walk::WalkStats,
    /// `rel -> position in stats.files`, built once so the dirty closure,
    /// no-op check, and gap healing share one index instead of rebuilding it
    /// per phase.
    by_rel: HashMap<String, usize>,
    root: PathBuf,
    old: Option<rusqlite::Connection>,
    old_map: HashMap<String, crate::db::FileStat>,
    cls: Classified,
    /// Parse-error count over the carried set as of the end of prepare_build
    /// (after the dirty closure). `no_op_line` and `parse_gate` reuse it;
    /// `extract_healed` subtracts every batch it moves out of `cls.carried`
    /// (those files re-extract, so their errors arrive via `FreshExtract`
    /// instead — counting them twice would inflate the gate rate).
    carried_errors: usize,
}

/// Resolve, load, and write one build. Each phase owns its slice of the
/// pipeline; `run_build` keeps the lock and the parse gate, and
/// `finish_index` resolves, writes, and journals. Returns the stdout line,
/// the stderr hints, and the indexed file count (the build's journaled
/// `shown`).
pub fn run_build(
    root_arg: Option<&Path>,
    force: bool,
    max_parse_error_rate: f64,
) -> Result<(String, String, usize)> {
    if !max_parse_error_rate.is_finite() || !(0.0..=1.0).contains(&max_parse_error_rate) {
        return Err(EgError::Usage(format!(
            "--max-parse-error-rate must be a finite value in [0, 1], got {max_parse_error_rate}"
        )));
    }
    let started = Instant::now();
    let root = crate::walk::discover_root(root_arg)?;
    let _lock = crate::db::BuildLock::acquire(&root)?;
    let mut prep = prepare_build(&root, force, max_parse_error_rate, started)?;
    if let Some(line) = no_op_line(&prep)? {
        return Ok((line, skipped_notice(&prep.stats), prep.stats.files.len()));
    }
    let t0 = Instant::now();
    let mut fe = extract_units(&prep.cls.fresh, 0)?;
    heal_gaps(&mut prep, &mut fe)?;
    let parse_secs = t0.elapsed().as_secs_f64();
    parse_gate(&prep, &fe)?;
    finish_index(&prep, fe, parse_secs)
}

/// Resolve, write, and journal the fresh extraction — the second half of
/// `run_build`, from `resolve_index` through the reuse hints. Consumes the
/// fresh extract; returns the stdout line, the stderr hints, and the indexed
/// file count (the build's journaled `shown`).
fn parse_gate(prep: &BuildPrep, fe: &FreshExtract) -> Result<()> {
    // Parse-quality gate (CUJ8): rate over ALL files, abort before DB work.
    // prep.carried_errors tracks the still-carried set: extract_healed
    // subtracts every batch it moves to fresh (those errors arrive via fe).
    let total_files = prep.stats.files.len();
    let error_files = fe.error_files + prep.carried_errors;
    if total_files > 0 && !prep.opts.force {
        let rate = error_files as f64 / total_files as f64;
        if rate > prep.opts.max_parse_error_rate {
            let _ = std::fs::remove_file(crate::db::db_new_path(&prep.root));
            return Err(EgError::ParseQuality {
                rate,
                files: total_files,
                threshold: prep.opts.max_parse_error_rate,
            });
        }
    }
    Ok(())
}

mod extract;
mod plan;
mod write;

use extract::*;
use plan::*;
use write::*;
