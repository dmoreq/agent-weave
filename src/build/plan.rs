//! Build planning and preparation: incremental classification, dirty
//! closure propagation, gap healing plans, and fresh/changed file selection.

use super::*;

/// file count (the build's journaled `shown`).
pub(super) fn finish_index(
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
    let mem_hints = incremental_memory_hints(prep, is_incremental);
    let mut stderr = skipped_notice(&prep.stats);
    stderr.push_str(&stderr_hints);
    stderr.push_str(&mem_hints);
    Ok((stdout_line, stderr, total_files))
}

fn incremental_memory_hints(prep: &BuildPrep, is_incremental: bool) -> String {
    if !is_incremental || prep.cls.fresh.is_empty() {
        return String::new();
    }
    let fresh_paths: Vec<&str> = prep.cls.fresh.iter().map(|f| f.rel.as_str()).collect();
    let guards = crate::memory::find_guarding_notes(&prep.root, &fresh_paths, &[]);
    if guards.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for g in guards.iter().take(3) {
        out.push_str(&format!(
            "eg: note: modified file '{}' is guarded by {} '{}'\n",
            g.target, g.kind, g.key
        ));
    }
    out
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
    let ctx = crate::query::QueryCtx {
        conn: &conn,
        root: &prep.root,
    };
    let Ok(hints) = crate::query::reuse::detect_hints(&ctx, &new_names) else {
        return String::new();
    };
    if hints.is_empty() {
        String::new()
    } else {
        hints.join("\n") + "\n"
    }
}

/// Walk, probe the previous index, and compute the carried/fresh split.
pub(super) fn prepare_build(
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
pub(super) fn write_index(
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
        .map_err(|(_, e)| crate::error::EgError::from(e))?;
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

pub(super) fn no_op_line(prep: &BuildPrep) -> Result<Option<String>> {
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

pub(super) fn heal_gaps(prep: &mut BuildPrep, fe: &mut FreshExtract) -> Result<()> {
    let Some(c) = &prep.old else {
        return Ok(());
    };
    let gap = plan_gap_files(c, prep, fe)?;
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
    prep: &BuildPrep,
    fe: &FreshExtract,
) -> Result<HashSet<String>> {
    let old_quals = crate::db::symbol_names(c)?;
    let old_qual_files = crate::db::symbol_qualname_files(c)?;
    let fresh = collect_fresh_quals(fe, &prep.cls, &old_quals, &old_qual_files);
    let changed = changed_files(&prep.cls, &prep.old_map, &prep.by_rel);
    // ANEW2: a candidate set changes when a defining file is added,
    // removed, or renamed. A shrink leaves no resolved edge for the dirty
    // closure to follow, so find cls.carried sites that still name the old
    // qualname and re-extract them.
    let mut changed_quals = fresh.duplicate_quals;
    changed_quals.extend(changed_qualnames(
        c,
        &changed,
        &prep.cls.carried,
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
