//! Build orchestration: full atomic replacement with the per-file fast path,
//! dirty closure, parse-quality gate, and meta manifest. Shared by the CLI
//! (prints the stats line) and the MCP server (returns it as tool text).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::error::{AwError, Result};

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
            "[aw] skipped {} file(s) over the 1MB size cap\n",
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
        return Err(AwError::Usage(format!(
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
fn finish_index(
    prep: &BuildPrep,
    fe: FreshExtract,
    parse_secs: f64,
) -> Result<(String, String, usize)> {
    let FreshExtract {
        mut rows,
        extractions,
        secs: extract_secs,
        ..
    } = fe;
    let resolution = resolve_index(&prep.old, &prep.cls.carried, extractions)?;
    for (row, unresolved) in rows.iter_mut().zip(&resolution.unresolved_per_file) {
        row.unresolved = *unresolved;
    }

    let is_incremental = prep.old.is_some() && !prep.opts.force;
    let (totals, db_secs) = write_index(prep, &rows, &resolution)?;
    let total_files = prep.stats.files.len();
    crate::journal::record_result(
        crate::journal::OutcomeCounts::uniform(total_files),
        false,
        0,
    );
    let stdout_line = built_index_line(
        prep,
        &totals,
        PhaseSecs {
            parse: parse_secs,
            extract: extract_secs,
            db: db_secs,
        },
    );
    let stderr_hints = incremental_reuse_hints(prep, &resolution, is_incremental);
    let mut stderr = skipped_notice(&prep.stats);
    stderr.push_str(&stderr_hints);
    Ok((stdout_line, stderr, total_files))
}

fn incremental_reuse_hints(
    prep: &BuildPrep,
    resolution: &crate::extract::Resolution,
    is_incremental: bool,
) -> String {
    if !is_incremental {
        return String::new();
    }
    let mut new_names = std::collections::HashSet::new();
    for (s, existing) in resolution.symbols.iter().zip(&resolution.existing_ids) {
        if !s.is_test && existing.is_none() {
            new_names.insert(s.name.clone());
            new_names.insert(s.qualname.clone());
        }
    }
    if new_names.is_empty() {
        return String::new();
    }
    let Ok(conn) = crate::db::open_for_query(&prep.root) else {
        return String::new();
    };
    let Ok(hints) = crate::query::reuse::detect_hints(&conn, &new_names) else {
        return String::new();
    };
    if hints.is_empty() {
        String::new()
    } else {
        hints.join("\n") + "\n"
    }
}

/// Walk, probe the previous index, and compute the carried/fresh split.
fn prepare_build(
    root: &Path,
    force: bool,
    max_parse_error_rate: f64,
    started: Instant,
) -> Result<BuildPrep> {
    let t0 = Instant::now();
    let stats = crate::walk::walk(root, &crate::walk::source_exts())?;
    let opts = BuildOpts {
        force,
        max_parse_error_rate,
        started,
        walk_secs: t0.elapsed().as_secs_f64(),
    };
    let old = if force {
        None
    } else {
        crate::db::probe_old(root)?
    };
    let old_map = match &old {
        Some(c) => crate::db::old_files(c)?,
        None => Default::default(),
    };
    let cls = classify(&stats, &old_map);
    let by_rel: HashMap<String, usize> = stats
        .files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.rel.clone(), i))
        .collect();
    let mut prep = BuildPrep {
        opts,
        stats,
        by_rel,
        root: root.to_path_buf(),
        old,
        old_map,
        cls,
        carried_errors: 0,
    };
    apply_dirty_closure(&mut prep)?;
    if let Some(c) = &prep.old {
        prep.carried_errors =
            crate::db::old_error_count(c, &prep.cls.carried.iter().cloned().collect::<Vec<_>>())?;
    }
    Ok(prep)
}

/// Write the new index, publish it atomically, and return its SQL totals plus
/// the DB phase wall time.
fn write_index(
    prep: &BuildPrep,
    rows: &[crate::db::FilesRow],
    resolution: &crate::extract::Resolution,
) -> Result<(MetaTotals, f64)> {
    let t0 = Instant::now();
    let conn = crate::db::create_fresh(&prep.root)?;
    conn.execute_batch("BEGIN")?;
    let ids = write_rows(&conn, prep, rows, resolution)?;
    let all_fresh = prep.old.is_none() && ids.sym_count == resolution.symbols.len();
    write_fts_indexes(&conn, resolution, &ids, all_fresh)?;
    crate::db::create_secondary_indexes(&conn)?;
    let totals = write_meta_counts(&conn, &prep.root)?;
    write_meta_universe(&conn)?;
    crate::db::set_meta(
        &conn,
        crate::db::meta_keys::UNRESOLVED_CALLEE_COUNT,
        &totals.unres_total.to_string(),
    )?;
    conn.execute_batch("COMMIT")?;
    conn.close()
        .map_err(|(_, e)| crate::error::AwError::from(e))?;
    crate::db::publish_new_index(&prep.root)?;
    Ok((totals, t0.elapsed().as_secs_f64()))
}
fn classify(
    stats: &crate::walk::WalkStats,
    old_map: &HashMap<String, crate::db::FileStat>,
) -> Classified {
    let mut carried: HashSet<String> = HashSet::new();
    // Hash-carried rows (Amendment 9): carried on content match despite an
    // mtime mismatch (restored index.db) — they need their stored mtime
    // refreshed to the fresh checkout's stat.
    let mut mtime_updates: HashMap<String, i64> = HashMap::new();
    let mut fresh: Vec<crate::walk::WalkedFile> = Vec::new();
    for f in &stats.files {
        let mtime = f.mtime;
        match old_map.get(&f.rel) {
            Some(stat) if stat.mtime == mtime && stat.size == f.size as i64 => {
                carried.insert(f.rel.clone());
            }
            // Dual-key (Amendment 9): (mtime, size) mismatch, but same size and
            // same content hash -> carry (the CI cache-restore case). One
            // read+hash instead of parse+extract.
            Some(stat) if stat.size == f.size as i64 => match crate::walk::read_bounded(&f.abs) {
                Ok(bytes) if crate::hash::fnv1a64(&bytes) as i64 == stat.content_hash => {
                    carried.insert(f.rel.clone());
                    mtime_updates.insert(f.rel.clone(), mtime);
                }
                _ => fresh.push(f.clone()),
            },
            _ => fresh.push(f.clone()),
        }
    }
    Classified {
        carried,
        mtime_updates,
        fresh,
    }
}

fn apply_dirty_closure(prep: &mut BuildPrep) -> Result<()> {
    // Dirty closure (fast-path correctness): a carried file with any edge into
    // a changed file must be re-extracted, or its copied edge dies with the old
    // destination id (transitive: callers of re-extracted files too).
    if let Some(c) = &prep.old {
        let mut changed: Vec<String> = prep.cls.fresh.iter().map(|f| f.rel.clone()).collect();
        // A deleted destination heals like a changed one: its old dependents
        // must re-extract so their import/call/inherit edges become unresolved
        // instead of being dropped silently by carry-forward.
        for rel in prep.old_map.keys() {
            if !prep.by_rel.contains_key(rel.as_str()) {
                changed.push(rel.clone());
            }
        }
        if !changed.is_empty() {
            let dirty = crate::db::dirty_closure(c, &changed)?;
            prep.cls.fresh.extend(take_fresh_batch(
                dirty,
                &mut prep.cls.carried,
                &prep.by_rel,
                &prep.stats.files,
            ));
            prep.cls.fresh.sort_by(|a, b| a.rel.cmp(&b.rel));
        }
    }
    Ok(())
}

fn no_op_line(prep: &BuildPrep) -> Result<Option<String>> {
    // No-op fast path (R7 L4): when every walked file cls.carried unchanged and no
    // indexed path disappeared, rebuilding the database would reproduce the
    // same rows (measured 0.05-0.06 s of a 0.10 s build at 2.2k files). Skip
    // the write entirely so a repeated build leaves index.db untouched.
    if let Some(c) = &prep.old {
        let deleted = prep
            .old_map
            .keys()
            .any(|rel| !prep.by_rel.contains_key(rel.as_str()));
        let rate_ok = prep.stats.files.is_empty()
            || prep.carried_errors as f64 / prep.stats.files.len() as f64
                <= prep.opts.max_parse_error_rate;
        if prep.cls.fresh.is_empty() && !deleted && prep.cls.mtime_updates.is_empty() && rate_ok {
            let totals = old_meta_totals(c)?;
            crate::journal::record_result(
                crate::journal::OutcomeCounts::uniform(prep.stats.files.len()),
                false,
                0,
            );
            return Ok(Some(built_index_line(
                prep,
                &totals,
                PhaseSecs {
                    parse: 0.0,
                    extract: 0.0,
                    db: 0.0,
                },
            )));
        }
    }
    Ok(None)
}

/// Totals for the no-op stats line: the previous build's meta counters first
/// (O(1); the same totals stats reads), COUNT only when a counter key is
/// absent — the shared (key, fallback SQL) pairs in db (META_COUNTERS) keep
/// this and `stats` in sync.
fn old_meta_totals(old: &rusqlite::Connection) -> Result<MetaTotals> {
    Ok(MetaTotals {
        sym_total: crate::db::meta_count(old, crate::db::meta_keys::SYMBOL_COUNT)?,
        edge_total: crate::db::meta_count(old, crate::db::meta_keys::EDGE_COUNT)?,
        perr_total: crate::db::meta_count(old, crate::db::meta_keys::PARSE_ERROR_COUNT)?,
        unres_total: crate::db::meta_count(old, crate::db::meta_keys::UNRESOLVED_CALLEE_COUNT)?,
    })
}

fn heal_gaps(prep: &mut BuildPrep, fe: &mut FreshExtract) -> Result<()> {
    let Some(c) = &prep.old else {
        return Ok(());
    };
    let gap = plan_gap_files(c, &prep.old_map, &prep.by_rel, &prep.cls, fe)?;
    if gap.is_empty() {
        return Ok(());
    }
    let second = take_fresh_batch(gap, &mut prep.cls.carried, &prep.by_rel, &prep.stats.files);
    if second.is_empty() {
        return Ok(());
    }
    extract_healed(prep, fe, second)
}

/// Amendment 15 (R6d A4): the fast path must heal resolution gaps. When the
/// fresh extraction introduces symbol names that did not exist in the old
/// index, a carried file whose unresolved call (or AMBIGUOUS edge) mentions
/// one of those names can now resolve — re-extract exactly those files.
/// Bounded by gap rows, not by corpus size.
fn plan_gap_files(
    c: &rusqlite::Connection,
    old_map: &HashMap<String, crate::db::FileStat>,
    by_rel: &HashMap<String, usize>,
    cls: &Classified,
    fe: &FreshExtract,
) -> Result<HashSet<String>> {
    let old_quals = crate::db::symbol_names(c)?;
    let old_qual_files = crate::db::symbol_qualname_files(c)?;
    let fresh = collect_fresh_quals(fe, cls, &old_quals, &old_qual_files);
    let changed = changed_files(cls, old_map, by_rel);
    // ANEW2: a candidate set changes when a defining file is added,
    // removed, or renamed. A shrink leaves no resolved edge for the dirty
    // closure to follow, so find cls.carried sites that still name the old
    // qualname and re-extract them.
    let mut changed_quals = fresh.duplicate_quals;
    changed_quals.extend(changed_qualnames(
        c,
        &changed,
        &cls.carried,
        QualFiles {
            old: &old_qual_files,
            fresh: &fresh.files,
        },
    )?);
    let mut gap = std::collections::HashSet::new();
    if !fresh.new_names.is_empty() {
        gap.extend(crate::db::gap_files_for_new_names(c, &fresh.new_names)?);
    }
    if !changed_quals.is_empty() {
        gap.extend(crate::db::gap_files_for_candidate_changes(
            c,
            &changed_quals,
        )?);
    }
    Ok(gap)
}

#[derive(Default)]
struct FreshQuals {
    new_names: HashSet<String>,
    duplicate_quals: HashSet<String>,
    files: HashMap<String, HashSet<String>>,
}

fn collect_fresh_quals(
    fe: &FreshExtract,
    cls: &Classified,
    old_quals: &HashSet<String>,
    old_qual_files: &HashMap<String, HashSet<String>>,
) -> FreshQuals {
    let mut out = FreshQuals::default();
    for fx in &fe.extractions {
        let rel = cls
            .fresh
            .get(fx.file_idx)
            .map(|f| f.rel.clone())
            .unwrap_or_default();
        for sym in &fx.symbols {
            if !old_quals.contains(&sym.qualname) {
                out.new_names.insert(sym.qualname.clone());
            } else if old_qual_files
                .get(&sym.qualname)
                .is_some_and(|paths| paths.iter().any(|p| p.as_str() != rel))
            {
                out.duplicate_quals.insert(sym.qualname.clone());
            }
            out.files
                .entry(sym.qualname.clone())
                .or_default()
                .insert(rel.clone());
        }
    }
    out
}

fn changed_files(
    cls: &Classified,
    old_map: &HashMap<String, crate::db::FileStat>,
    by_rel: &HashMap<String, usize>,
) -> Vec<String> {
    cls.fresh
        .iter()
        .map(|f| f.rel.clone())
        .chain(
            old_map
                .keys()
                .filter(|rel| !by_rel.contains_key(rel.as_str()))
                .cloned(),
        )
        .collect()
}

struct QualFiles<'a> {
    old: &'a HashMap<String, HashSet<String>>,
    fresh: &'a HashMap<String, HashSet<String>>,
}

fn changed_qualnames(
    c: &rusqlite::Connection,
    changed: &[String],
    carried: &HashSet<String>,
    quals: QualFiles<'_>,
) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for q in crate::db::qualnames_for_files(c, changed)? {
        let Some(old_files) = quals.old.get(&q) else {
            continue;
        };
        let mut new_files: std::collections::HashSet<&str> = old_files
            .iter()
            .filter(|p| carried.contains(*p))
            .map(String::as_str)
            .collect();
        if let Some(files) = quals.fresh.get(&q) {
            new_files.extend(files.iter().map(String::as_str));
        }
        if new_files.len() != old_files.len() || new_files.iter().any(|p| !old_files.contains(*p)) {
            out.push(q);
        }
    }
    Ok(out)
}

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
fn extract_healed(
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
            return Err(AwError::ParseQuality {
                rate,
                files: total_files,
                threshold: prep.opts.max_parse_error_rate,
            });
        }
    }
    Ok(())
}

fn resolve_index(
    old: &Option<rusqlite::Connection>,
    carried: &HashSet<String>,
    extractions: Vec<crate::extract::FileExtraction>,
) -> Result<crate::extract::Resolution> {
    let carried_index = match &old {
        Some(c) => crate::db::load_carried(c, carried)?,
        None => crate::extract::CarriedIndex::default(),
    };
    Ok(crate::extract::resolve(extractions, carried_index))
}

fn write_rows(
    conn: &rusqlite::Connection,
    prep: &BuildPrep,
    rows: &[crate::db::FilesRow],
    resolution: &crate::extract::Resolution,
) -> Result<WriteIds> {
    if !prep.cls.carried.is_empty() {
        let unchanged = carry_forward_rows(&prep.cls, &prep.old_map);
        crate::db::carry_forward_unchanged(conn, &prep.root, &unchanged)?;
    }
    let fresh_file_ids = crate::db::insert_files(conn, rows)?;
    // Schema v6 / Amendment 19: unresolved edge sites per fresh file, so a
    // later build can re-extract exactly the files whose gaps a new symbol
    // could heal.
    let unresolved_rows: Vec<(i64, &str, crate::extract::EdgeSiteKind, usize)> = resolution
        .unresolved_calls
        .iter()
        .map(|u| {
            (
                fresh_file_ids[u.file_idx],
                u.name.as_str(),
                u.kind,
                u.site_line,
            )
        })
        .collect();
    crate::db::insert_unresolved_calls(conn, &unresolved_rows)?;

    // Symbols: fresh rows insert; carried rows keep their previous ids.
    let (fresh_symbols, fresh_pos, fresh_sym_count) = split_fresh_symbols(resolution);
    let fresh_paths: Vec<String> = rows.iter().map(|r| r.rel.clone()).collect();
    let fresh_sym_ids =
        crate::db::insert_symbols(conn, &fresh_symbols, &fresh_file_ids, &fresh_paths)?;

    // Edges: endpoints map fresh -> inserted id, carried -> previous row id.
    let edge_rows = edge_rows_for(resolution, &fresh_pos, &fresh_sym_ids);
    crate::db::insert_edge_rows(conn, &edge_rows)?;
    Ok(WriteIds {
        sym_ids: fresh_sym_ids,
        pos: fresh_pos,
        paths: fresh_paths,
        sym_count: fresh_sym_count,
    })
}

fn carry_forward_rows(
    cls: &Classified,
    old_map: &HashMap<String, crate::db::FileStat>,
) -> Vec<crate::db::CarryRow> {
    cls.carried
        .iter()
        .filter_map(|rel| {
            old_map.get(rel).map(|stat| {
                let current_mtime = cls.mtime_updates.get(rel).copied().unwrap_or(stat.mtime);
                crate::db::CarryRow {
                    rel: rel.clone(),
                    old_mtime: stat.mtime,
                    size: stat.size,
                    current_mtime,
                }
            })
        })
        .collect()
}

fn split_fresh_symbols(
    resolution: &crate::extract::Resolution,
) -> (Vec<&crate::extract::NewSymbol>, Vec<Option<usize>>, usize) {
    let fresh_sym_count = resolution
        .existing_ids
        .iter()
        .filter(|e| e.is_none())
        .count();
    let mut fresh_symbols = Vec::with_capacity(fresh_sym_count);
    let mut fresh_pos = Vec::with_capacity(resolution.symbols.len());
    for (sym, existing) in resolution.symbols.iter().zip(&resolution.existing_ids) {
        if existing.is_none() {
            fresh_pos.push(Some(fresh_symbols.len()));
            fresh_symbols.push(sym);
        } else {
            fresh_pos.push(None);
        }
    }
    (fresh_symbols, fresh_pos, fresh_sym_count)
}

fn edge_rows_for<'a>(
    resolution: &'a crate::extract::Resolution,
    fresh_pos: &[Option<usize>],
    fresh_sym_ids: &[i64],
) -> Vec<crate::db::EdgeRow<'a>> {
    let mut edge_rows = Vec::with_capacity(resolution.edges.len());
    for e in &resolution.edges {
        let src_id = match resolve_sym_id(e.src_idx, fresh_pos, fresh_sym_ids, resolution) {
            Some(id) => id,
            None => continue,
        };
        let dst_id = e
            .dst_idx
            .and_then(|d| resolve_sym_id(d, fresh_pos, fresh_sym_ids, resolution));
        if e.dst_idx.is_some() && dst_id.is_none() {
            continue; // defensive: unmapped endpoint (should not happen)
        }
        edge_rows.push(crate::db::EdgeRow {
            src_id,
            dst_id,
            dst_name: e.dst_name.as_str(),
            kind: e.kind.as_str(),
            confidence: e.confidence,
            site_line: e.site_line,
        });
    }
    edge_rows
}

fn write_fts_indexes(
    conn: &rusqlite::Connection,
    resolution: &crate::extract::Resolution,
    ids: &WriteIds,
    all_fresh: bool,
) -> Result<()> {
    // Fast path: a fully fresh build writes FTS rows directly from the
    // in-memory symbol rows instead of re-reading all symbols with a
    // SELECT + JOIN. Any other build keeps the SELECT path (carried rows
    // are not in memory).
    if all_fresh {
        let mut fts_rows: Vec<(i64, &str, &str, &str, &str)> =
            Vec::with_capacity(resolution.symbols.len());
        let mut complete = true;
        for (sym, pos) in resolution.symbols.iter().zip(&ids.pos) {
            let Some(pos) = pos else {
                complete = false;
                break;
            };
            let Some(path) = ids.paths.get(sym.file_idx) else {
                complete = false;
                break;
            };
            fts_rows.push((
                ids.sym_ids[*pos],
                sym.qualname.as_str(),
                sym.name.as_str(),
                path.as_str(),
                sym.signature.as_str(),
            ));
        }
        if complete {
            crate::db::populate_symbols_fts_rows(conn, &fts_rows)?;
        } else {
            crate::db::populate_symbols_fts(conn)?;
        }
    } else {
        crate::db::populate_symbols_fts(conn)?;
    }
    Ok(())
}

/// Authoritative SQL totals for the meta manifest, one scan per table (A3).
struct MetaCounts {
    sym_total: i64,
    edge_total: i64,
    calls_total: i64,
    imports_total: i64,
    inherits_total: i64,
    file_total: i64,
    perr_total: i64,
    unres_total: i64,
}

fn write_meta_counts(conn: &rusqlite::Connection, root: &Path) -> Result<MetaTotals> {
    use crate::db::meta_keys;
    // Meta manifest; counts are authoritative SQL totals over the whole index.
    // Keys come from `db::meta_keys`, the same vocabulary the pre-counter
    // fallback registers in `db::meta_counters`.
    write_meta_identity(conn, root)?;
    let c = meta_counts(conn)?;
    for (key, n) in [
        (meta_keys::FILE_COUNT, c.file_total),
        (meta_keys::SYMBOL_COUNT, c.sym_total),
        (meta_keys::EDGE_COUNT, c.edge_total),
        (meta_keys::PARSE_ERROR_COUNT, c.perr_total),
        (meta_keys::EDGE_CALLS, c.calls_total),
        (meta_keys::EDGE_IMPORTS, c.imports_total),
        (meta_keys::EDGE_INHERITS, c.inherits_total),
    ] {
        crate::db::set_meta(conn, key, &n.to_string())?;
    }
    // Authoritative totals for the stats line.
    Ok(MetaTotals {
        sym_total: c.sym_total,
        edge_total: c.edge_total,
        perr_total: c.perr_total,
        unres_total: c.unres_total,
    })
}

/// Identity and provenance meta: grammar/signature/metrics versions, build
/// time, and the source revision.
fn write_meta_identity(conn: &rusqlite::Connection, root: &Path) -> Result<()> {
    // Millisecond precision keeps consecutive sub-second rebuilds from
    // colliding on the same epoch value.
    let built_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_else(|_| "0".into());
    for (key, value) in [
        (crate::db::meta_keys::AW_VERSION, env!("CARGO_PKG_VERSION")),
        (
            crate::db::meta_keys::GRAMMAR_VERSION,
            crate::db::GRAMMAR_VERSION,
        ),
        (
            crate::db::meta_keys::SIGNATURE_VERSION,
            crate::db::SIGNATURE_VERSION,
        ),
        (
            crate::db::meta_keys::METRICS_VERSION,
            crate::db::METRICS_VERSION,
        ),
        (crate::db::meta_keys::BUILT_AT, built_at.as_str()),
        (
            crate::db::meta_keys::SOURCE_REV,
            crate::db::source_rev(root).as_str(),
        ),
    ] {
        crate::db::set_meta(conn, key, value)?;
    }
    Ok(())
}

/// One scan per table instead of one query per count (A3).
fn meta_counts(conn: &rusqlite::Connection) -> Result<MetaCounts> {
    let sym_total: i64 = conn.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))?;
    let (edge_total, calls_total, imports_total, inherits_total) = conn.query_row(
        &format!(
            "SELECT COUNT(*),
            COALESCE(SUM(kind = '{}'), 0),
            COALESCE(SUM(kind = '{}'), 0),
            COALESCE(SUM(kind = '{}'), 0)
     FROM edges",
            crate::extract::EdgeKind::Calls.as_str(),
            crate::extract::EdgeKind::Imports.as_str(),
            crate::extract::EdgeKind::Inherits.as_str()
        ),
        [],
        crate::db::ints4,
    )?;
    let (file_total, perr_total, unres_total) = conn.query_row(
        "SELECT COUNT(*),
            COALESCE(SUM(parse_status != 'ok'), 0),
            COALESCE(SUM(unresolved), 0)
     FROM files",
        [],
        crate::db::ints3,
    )?;
    Ok(MetaCounts {
        sym_total,
        edge_total,
        calls_total,
        imports_total,
        inherits_total,
        file_total,
        perr_total,
        unres_total,
    })
}

fn write_meta_universe(conn: &rusqlite::Connection) -> Result<()> {
    // files per kind plus the union; old indexes fall back to an edge scan.
    let (calls, imports, inherits) = (
        crate::extract::EdgeKind::Calls,
        crate::extract::EdgeKind::Imports,
        crate::extract::EdgeKind::Inherits,
    );
    let (calls_src, imports_src, inherits_src, all_src) = conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT CASE WHEN e.kind = '{}' THEN s.file_path END),
            COUNT(DISTINCT CASE WHEN e.kind = '{}' THEN s.file_path END),
            COUNT(DISTINCT CASE WHEN e.kind = '{}' THEN s.file_path END),
            COUNT(DISTINCT CASE WHEN e.kind IN ('{}','{}','{}') THEN s.file_path END)
     FROM edges e JOIN symbols s ON s.id = e.src_id",
            calls.as_str(),
            imports.as_str(),
            inherits.as_str(),
            calls.as_str(),
            imports.as_str(),
            inherits.as_str()
        ),
        [],
        crate::db::ints4,
    )?;
    for (key, n) in [
        (crate::db::meta_keys::EDGE_CALLS_SRC_FILES, calls_src),
        (crate::db::meta_keys::EDGE_IMPORTS_SRC_FILES, imports_src),
        (crate::db::meta_keys::EDGE_INHERITS_SRC_FILES, inherits_src),
        (crate::db::meta_keys::EDGE_ALL_SRC_FILES, all_src),
    ] {
        crate::db::set_meta(conn, key, &n.to_string())?;
    }
    Ok(())
}

/// The `aw: built index: ...` stats line shared by the full build and the
/// no-op path (which passes zero write-phase timings), so the field contract
/// stays byte-identical across both branches.
fn built_index_line(prep: &BuildPrep, totals: &MetaTotals, secs: PhaseSecs) -> String {
    // CPU seconds are measured by the G1 harness (/usr/bin/time -l); the
    // binary reports wall + per-phase timings (std-only constraint).
    // Amendment 11c: parse= is the parallel phase's wall time and extract= the
    // summed per-worker extraction time — the two overlap on parallel builds.
    format!(
        "aw: built index: files={total} symbols={symbols} edges={edges} parse_errors={perr} unresolved={unres} carried={carried} wall={wall:.2}s (walk={w:.2}s parse={p:.2}s extract={e:.2}s db={d:.2}s)",
        total = prep.stats.files.len(),
        symbols = totals.sym_total,
        edges = totals.edge_total,
        perr = totals.perr_total,
        unres = totals.unres_total,
        carried = prep.cls.carried.len(),
        wall = prep.opts.started.elapsed().as_secs_f64(),
        w = prep.opts.walk_secs,
        p = secs.parse,
        e = secs.extract,
        d = secs.db,
    )
}

fn resolve_sym_id(
    sym_idx: usize,
    fresh_pos: &[Option<usize>],
    fresh_sym_ids: &[i64],
    resolution: &crate::extract::Resolution,
) -> Option<i64> {
    match fresh_pos.get(sym_idx).copied().flatten() {
        Some(p) => fresh_sym_ids.get(p).copied(),
        None => resolution.existing_ids.get(sym_idx).copied().flatten(),
    }
}

/// Parse + extract one batch of walked files on scoped workers (Amendment 11c
/// parallel parse; R6d Blocker 1: 64 MB stacks). Trees never cross threads (not
/// Send); only owned extraction rows do. Chunks are contiguous and joined in
/// spawn order, so the merged order equals the serial loop's. `base_idx` keeps
/// the extraction file index aligned with the accumulated `fresh` vector.
fn extract_units(files: &[crate::walk::WalkedFile], base_idx: usize) -> Result<FreshExtract> {
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
                .map_err(|_| AwError::Internal("parse worker panicked".to_string()))??;
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
    std::env::var("AW_WORKERS")
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
        .ok_or_else(|| AwError::Internal(format!("unsupported source extension: {}", file.rel)))?;
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
