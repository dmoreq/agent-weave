//! Build orchestration: full atomic replacement with the per-file fast path,
//! dirty closure, parse-quality gate, and meta manifest. Shared by the CLI
//! (prints the stats line) and the MCP server (returns it as tool text).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use crate::error::{AwError, Result};

pub fn run_build(
    root_arg: Option<&Path>,
    force: bool,
    max_parse_error_rate: f64,
) -> Result<String> {
    if !max_parse_error_rate.is_finite() || !(0.0..=1.0).contains(&max_parse_error_rate) {
        return Err(AwError::Usage(format!(
            "--max-parse-error-rate must be a finite value in [0, 1], got {max_parse_error_rate}"
        )));
    }
    let started = Instant::now();
    let root = crate::walk::discover_root(root_arg)?;
    let _lock = crate::db::BuildLock::acquire(&root)?;

    // Walk.
    let t0 = Instant::now();
    let stats = crate::walk::walk(&root, crate::walk::SOURCE_EXTS)?;
    let walk_secs = t0.elapsed().as_secs_f64();

    // Fast path: files unchanged since a compatible previous index.
    let old = if force {
        None
    } else {
        crate::db::probe_old(&root)?
    };
    let old_map = match &old {
        Some(c) => crate::db::old_files(c)?,
        None => Default::default(),
    };
    let mut carried: HashSet<String> = HashSet::new();
    // Hash-carried rows (Amendment 9): carried on content match despite an
    // mtime mismatch (restored index.db) — they need their stored mtime
    // refreshed to the fresh checkout's stat.
    let mut mtime_updates: HashMap<String, i64> = HashMap::new();
    let mut fresh: Vec<crate::walk::WalkedFile> = Vec::new();
    for f in &stats.files {
        let mtime = f.mtime;
        match old_map.get(&f.rel) {
            Some((om, osz, _oh)) if *om == mtime && *osz == f.size as i64 => {
                carried.insert(f.rel.clone());
            }
            // Dual-key (Amendment 9): (mtime, size) mismatch, but same size and
            // same content hash -> carry (the CI cache-restore case). One
            // read+hash instead of parse+extract.
            Some((_om, osz, oh)) if *osz == f.size as i64 => {
                match crate::walk::read_bounded(&f.abs) {
                    Ok(bytes) if crate::hash::fnv1a64(&bytes) as i64 == *oh => {
                        carried.insert(f.rel.clone());
                        mtime_updates.insert(f.rel.clone(), mtime);
                    }
                    _ => fresh.push(crate::walk::WalkedFile {
                        abs: f.abs.clone(),
                        rel: f.rel.clone(),
                        size: f.size,
                        mtime,
                    }),
                }
            }
            _ => fresh.push(crate::walk::WalkedFile {
                abs: f.abs.clone(),
                rel: f.rel.clone(),
                size: f.size,
                mtime,
            }),
        }
    }

    // Dirty closure (fast-path correctness): a carried file with any edge into
    // a changed file must be re-extracted, or its copied edge dies with the old
    // destination id (transitive: callers of re-extracted files too).
    let by_rel: std::collections::HashMap<&str, &crate::walk::WalkedFile> =
        stats.files.iter().map(|f| (f.rel.as_str(), f)).collect();
    if let Some(c) = &old {
        let mut changed: Vec<String> = fresh.iter().map(|f| f.rel.clone()).collect();
        // A deleted destination heals like a changed one: its old dependents
        // must re-extract so their import/call/inherit edges become unresolved
        // instead of being dropped silently by carry-forward.
        for rel in old_map.keys() {
            if !by_rel.contains_key(rel.as_str()) {
                changed.push(rel.clone());
            }
        }
        if !changed.is_empty() {
            let dirty = crate::db::dirty_closure(c, &changed)?;
            for rel in dirty {
                if carried.remove(&rel) {
                    if let Some(f) = by_rel.get(rel.as_str()) {
                        fresh.push(crate::walk::WalkedFile {
                            abs: f.abs.clone(),
                            rel: f.rel.clone(),
                            size: f.size,
                            mtime: f.mtime,
                        });
                    }
                }
            }
            fresh.sort_by(|a, b| a.rel.cmp(&b.rel));
        }
    }

    // No-op fast path (R7 L4): when every walked file carried unchanged and no
    // indexed path disappeared, rebuilding the database would reproduce the
    // same rows (measured 0.05-0.06 s of a 0.10 s build at 2.2k files). Skip
    // the write entirely so a repeated build leaves index.db untouched.
    if let Some(c) = &old {
        let deleted = old_map.keys().any(|rel| !by_rel.contains_key(rel.as_str()));
        let carried_errors =
            crate::db::old_error_count(c, &carried.iter().cloned().collect::<Vec<_>>())?;
        let rate_ok = force
            || stats.files.is_empty()
            || carried_errors as f64 / stats.files.len() as f64 <= max_parse_error_rate;
        if fresh.is_empty() && !deleted && mtime_updates.is_empty() && rate_ok {
            let sql_count = |sql: &str| -> Result<i64> { Ok(c.query_row(sql, [], |r| r.get(0))?) };
            let symbols = sql_count("SELECT COUNT(*) FROM symbols")?;
            let edges = sql_count("SELECT COUNT(*) FROM edges")?;
            let perr = sql_count("SELECT COALESCE(SUM(parse_status != 'ok'), 0) FROM files")?;
            let unres = sql_count("SELECT COALESCE(SUM(unresolved), 0) FROM files")?;
            crate::journal::record_result(
                stats.files.len(),
                stats.files.len(),
                stats.files.len(),
                false,
                0,
            );
            if stats.skipped_large > 0 {
                eprintln!(
                    "[aw] skipped {} file(s) over the 1MB size cap",
                    stats.skipped_large
                );
            }
            return Ok(format!(
                "aw: built index: files={} symbols={symbols} edges={edges} parse_errors={perr} unresolved={unres} carried={} wall={wall:.2}s (walk={w:.2}s parse=0.00s extract=0.00s db=0.00s)",
                stats.files.len(),
                stats.files.len(),
                wall = started.elapsed().as_secs_f64(),
                w = walk_secs,
            ));
        }
    }

    // Parse + extract fresh files only (streamed; no corpus accumulation).
    // Amendment 11c: CPU-bound tree-sitter work on scoped workers — no new
    // dependencies, no unsafe. Trees never cross threads (not Send); only
    // owned extraction rows do. Chunks are contiguous and joined in spawn
    // order, so the merged order equals the serial loop's and file_idx (the
    // index into `fresh`, consumed by insert_files/insert_symbols) is
    // unchanged.
    let t0 = Instant::now();
    let (mut fresh_rows, mut extractions, mut fresh_error_files, mut extract_secs_sum) =
        extract_units(&fresh, 0)?;

    // Amendment 15 (R6d A4): the fast path must heal resolution gaps. When
    // the fresh extraction introduces symbol names that did not exist in the
    // old index, a carried file whose unresolved call (or AMBIGUOUS edge)
    // mentions one of those names can now resolve — re-extract exactly those
    // files. Bounded by gap rows, not by corpus size.
    if let Some(c) = &old {
        let old_quals = crate::db::symbol_names(c)?;
        let old_qual_files = crate::db::symbol_qualname_files(c)?;
        let mut new_names: std::collections::HashSet<String> = Default::default();
        let mut duplicate_quals: std::collections::HashSet<String> = Default::default();
        let mut fresh_qual_files: HashMap<String, HashSet<String>> = HashMap::new();
        for fx in &extractions {
            let rel = fresh
                .get(fx.file_idx)
                .map(|f| f.rel.clone())
                .unwrap_or_default();
            for sym in &fx.symbols {
                if !old_quals.contains(&sym.qualname) {
                    new_names.insert(sym.qualname.clone());
                } else if old_qual_files
                    .get(&sym.qualname)
                    .is_some_and(|paths| paths.iter().any(|p| p.as_str() != rel))
                {
                    duplicate_quals.insert(sym.qualname.clone());
                }
                fresh_qual_files
                    .entry(sym.qualname.clone())
                    .or_default()
                    .insert(rel.clone());
            }
        }
        // ANEW2: a candidate set changes when a defining file is added,
        // removed, or renamed. A shrink leaves no resolved edge for the dirty
        // closure to follow, so find carried sites that still name the old
        // qualname and re-extract them.
        let changed: Vec<String> = fresh
            .iter()
            .map(|f| f.rel.clone())
            .chain(
                old_map
                    .keys()
                    .filter(|rel| !by_rel.contains_key(rel.as_str()))
                    .cloned(),
            )
            .collect();
        let old_changed_quals = crate::db::qualnames_for_files(c, &changed)?;
        let mut changed_quals = duplicate_quals;
        for q in &old_changed_quals {
            let Some(old_files) = old_qual_files.get(q) else {
                continue;
            };
            let mut new_files: std::collections::HashSet<&str> = old_files
                .iter()
                .filter(|p| carried.contains(*p))
                .map(String::as_str)
                .collect();
            if let Some(files) = fresh_qual_files.get(q) {
                new_files.extend(files.iter().map(String::as_str));
            }
            if new_files.len() != old_files.len()
                || new_files.iter().any(|p| !old_files.contains(*p))
            {
                changed_quals.insert(q.clone());
            }
        }
        let mut gap = std::collections::HashSet::new();
        if !new_names.is_empty() {
            gap.extend(crate::db::gap_files_for_new_names(c, &new_names)?);
        }
        if !changed_quals.is_empty() {
            gap.extend(crate::db::gap_files_for_candidate_changes(
                c,
                &changed_quals,
            )?);
        }
        if !gap.is_empty() {
            let mut second: Vec<crate::walk::WalkedFile> = Vec::new();
            for rel in gap {
                if carried.remove(&rel) {
                    if let Some(f) = by_rel.get(rel.as_str()) {
                        second.push(crate::walk::WalkedFile {
                            abs: f.abs.clone(),
                            rel: f.rel.clone(),
                            size: f.size,
                            mtime: f.mtime,
                        });
                    }
                }
            }
            second.sort_by(|a, b| a.rel.cmp(&b.rel));
            if !second.is_empty() {
                let base = fresh.len();
                let mut healed: Vec<String> = second.iter().map(|f| f.rel.clone()).collect();
                let (mut r2, mut e2, err2, sec2) = extract_units(&second, base)?;
                fresh.extend(second);
                fresh_rows.append(&mut r2);
                extractions.append(&mut e2);
                fresh_error_files += err2;
                extract_secs_sum += sec2;

                // ANEW1: gap healing moves carried files to fresh after the
                // initial dirty closure. Their inbound callers are still
                // carried and point at the old symbol ids, so re-apply the
                // inbound closure to each newly fresh set until it is stable.
                while !healed.is_empty() {
                    let inbound = crate::db::dirty_closure(c, &healed)?;
                    let mut third: Vec<crate::walk::WalkedFile> = Vec::new();
                    for rel in inbound {
                        if carried.remove(&rel) {
                            if let Some(f) = by_rel.get(rel.as_str()) {
                                third.push(crate::walk::WalkedFile {
                                    abs: f.abs.clone(),
                                    rel: f.rel.clone(),
                                    size: f.size,
                                    mtime: f.mtime,
                                });
                            }
                        }
                    }
                    third.sort_by(|a, b| a.rel.cmp(&b.rel));
                    if third.is_empty() {
                        break;
                    }
                    let base = fresh.len();
                    let (mut r3, mut e3, err3, sec3) = extract_units(&third, base)?;
                    healed = third.iter().map(|f| f.rel.clone()).collect();
                    fresh.extend(third);
                    fresh_rows.append(&mut r3);
                    extractions.append(&mut e3);
                    fresh_error_files += err3;
                    extract_secs_sum += sec3;
                }
            }
        }
    }
    let parse_secs = t0.elapsed().as_secs_f64();

    // Parse-quality gate (CUJ8): rate over ALL files, abort before DB work.
    let total_files = stats.files.len();
    let carried_errors = match &old {
        Some(c) => crate::db::old_error_count(c, &carried.iter().cloned().collect::<Vec<_>>())?,
        None => 0,
    };
    let error_files = fresh_error_files + carried_errors;
    if total_files > 0 && !force {
        let rate = error_files as f64 / total_files as f64;
        if rate > max_parse_error_rate {
            let _ = std::fs::remove_file(crate::db::db_new_path(&root));
            return Err(AwError::ParseQuality {
                rate,
                files: total_files,
                threshold: max_parse_error_rate,
            });
        }
    }

    // Resolve over fresh extractions + carried symbols. The dirty closure above
    // re-extracts every carried file whose edges touched a changed file, so
    // both fresh->carried and carried->fresh resolution stay intact.
    let t0 = Instant::now();
    let carried_index = match &old {
        Some(c) => crate::db::load_carried(c, &carried)?,
        None => crate::extract::CarriedIndex::default(),
    };
    let resolution = crate::extract::resolve(extractions, carried_index);
    let _resolve_secs = t0.elapsed().as_secs_f64();
    // Per-file unresolved counts: parallel to the fresh-file order.
    for (row, unresolved) in fresh_rows.iter_mut().zip(&resolution.unresolved_per_file) {
        row.unresolved = *unresolved;
    }

    // DB: fresh schema at index.db.new -> carry forward -> insert -> swap.
    // Amendment 11b: one transaction for the whole write (carry-forward +
    // files + symbols + edges + meta) — the per-500-row commits were pure
    // fsync cost on a throwaway file; a crash before rename(2) leaves only an
    // orphan .new, which the next build's lock holder removes.
    let t0 = Instant::now();
    let conn = crate::db::create_fresh(&root)?;
    conn.execute_batch("BEGIN")?;
    if !carried.is_empty() {
        let unchanged: Vec<(String, i64, i64, i64)> = carried
            .iter()
            .filter_map(|rel| {
                old_map.get(rel).map(|(m, s, _)| {
                    let current = mtime_updates.get(rel).copied().unwrap_or(*m);
                    (rel.clone(), *m, *s, current)
                })
            })
            .collect();
        crate::db::carry_forward_unchanged(&conn, &root, &unchanged)?;
    }
    let fresh_file_ids = crate::db::insert_files(&conn, &fresh_rows)?;
    // Schema v6 / Amendment 19: unresolved edge sites per fresh file, so a
    // later build can re-extract exactly the files whose gaps a new symbol
    // could heal.
    let unresolved_rows: Vec<(i64, &str, &'static str, usize)> = resolution
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
    crate::db::insert_unresolved_calls(&conn, &unresolved_rows)?;

    // Symbols: fresh rows insert; carried rows keep their previous ids.
    let fresh_sym_count = resolution
        .existing_ids
        .iter()
        .filter(|e| e.is_none())
        .count();
    let mut fresh_symbols: Vec<&crate::extract::NewSymbol> = Vec::with_capacity(fresh_sym_count);
    let mut fresh_pos: Vec<Option<usize>> = Vec::with_capacity(resolution.symbols.len());
    for (sym, existing) in resolution.symbols.iter().zip(&resolution.existing_ids) {
        if existing.is_none() {
            fresh_pos.push(Some(fresh_symbols.len()));
            fresh_symbols.push(sym);
        } else {
            fresh_pos.push(None);
        }
    }
    let fresh_paths: Vec<String> = fresh_rows.iter().map(|r| r.rel.clone()).collect();
    let fresh_sym_ids =
        crate::db::insert_symbols(&conn, &fresh_symbols, &fresh_file_ids, &fresh_paths)?;

    // Edges: endpoints map fresh -> inserted id, carried -> previous row id.
    let mut edge_rows: Vec<crate::db::EdgeRow<'_>> = Vec::with_capacity(resolution.edges.len());
    for e in &resolution.edges {
        let src_id = match resolve_sym_id(e.src_idx, &fresh_pos, &fresh_sym_ids, &resolution) {
            Some(id) => id,
            None => continue,
        };
        let dst_id = e
            .dst_idx
            .and_then(|d| resolve_sym_id(d, &fresh_pos, &fresh_sym_ids, &resolution));
        if e.dst_idx.is_some() && dst_id.is_none() {
            continue; // defensive: unmapped endpoint (should not happen)
        }
        edge_rows.push(crate::db::EdgeRow {
            src_id,
            dst_id,
            dst_name: e.dst_name.as_str(),
            kind: e.kind,
            confidence: e.confidence,
            site_line: e.site_line,
        });
    }
    crate::db::insert_edge_rows(&conn, &edge_rows)?;
    // FTS (A4): a forced rebuild has every symbol in memory, so insert the FTS
    // rows directly instead of re-reading all symbols with a SELECT + JOIN.
    // Any other build keeps the SELECT path (carried rows are not in memory).
    let all_fresh = old.is_none() && fresh_sym_count == resolution.symbols.len();
    if all_fresh {
        let mut fts_rows: Vec<(i64, &str, &str, &str, &str)> =
            Vec::with_capacity(resolution.symbols.len());
        let mut complete = true;
        for (sym, pos) in resolution.symbols.iter().zip(&fresh_pos) {
            let Some(pos) = pos else {
                complete = false;
                break;
            };
            let Some(path) = fresh_paths.get(sym.file_idx) else {
                complete = false;
                break;
            };
            fts_rows.push((
                fresh_sym_ids[*pos],
                sym.qualname.as_str(),
                sym.name.as_str(),
                path.as_str(),
                sym.signature.as_str(),
            ));
        }
        if complete {
            crate::db::populate_symbols_fts_rows(&conn, &fts_rows)?;
        } else {
            crate::db::populate_symbols_fts(&conn)?;
        }
    } else {
        crate::db::populate_symbols_fts(&conn)?;
    }
    // Deferred indexes (A1): sort each secondary index once, after every row is
    // in place. The final schema is unchanged; queries only see the published DB.
    crate::db::create_secondary_indexes(&conn)?;

    // Meta manifest; counts are authoritative SQL totals over the whole index.
    crate::db::set_meta(&conn, "aw_version", env!("CARGO_PKG_VERSION"))?;
    crate::db::set_meta(&conn, "grammar_version", crate::db::GRAMMAR_VERSION)?;
    crate::db::set_meta(&conn, "signature_version", crate::db::SIGNATURE_VERSION)?;
    crate::db::set_meta(&conn, "metrics_version", crate::db::METRICS_VERSION)?;
    crate::db::set_meta(
        &conn,
        "built_at",
        // Millisecond precision keeps consecutive sub-second rebuilds from
        // colliding on the same epoch value.
        &std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis().to_string())
            .unwrap_or_else(|_| "0".into()),
    )?;
    crate::db::set_meta(&conn, "source_rev", &crate::db::source_rev(&root))?;
    // One scan per table instead of one query per count (A3).
    let (sym_total, callable_symbols, callable_total) = conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(kind IN ('function','method')), 0),
                COALESCE(SUM(kind IN ('function','method') AND effective_loc > 0), 0)
         FROM symbols",
        [],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        },
    )?;
    crate::db::set_meta(&conn, "callable_count", &callable_total.to_string())?;
    let (edge_total, calls_total, imports_total, inherits_total) = conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(kind = 'calls'), 0),
                COALESCE(SUM(kind = 'imports'), 0),
                COALESCE(SUM(kind = 'inherits'), 0)
         FROM edges",
        [],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        },
    )?;
    let (file_total, perr_total, unres_total) = conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(parse_status != 'ok'), 0),
                COALESCE(SUM(unresolved), 0)
         FROM files",
        [],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        },
    )?;
    crate::db::set_meta(&conn, "file_count", &file_total.to_string())?;
    crate::db::set_meta(&conn, "symbol_count", &sym_total.to_string())?;
    crate::db::set_meta(&conn, "edge_count", &edge_total.to_string())?;
    crate::db::set_meta(&conn, "parse_error_count", &perr_total.to_string())?;
    // Unresolved total is the authoritative SQL sum of per-file counts, stable
    // across fast-path builds (carried rows keep their stored count).
    for (key, n) in [
        ("edge_calls", calls_total),
        ("edge_imports", imports_total),
        ("edge_inherits", inherits_total),
    ] {
        crate::db::set_meta(&conn, key, &n.to_string())?;
    }
    // Exact reachable-universe counters for impact early-stop. Distinct source
    // files per kind plus the union; old indexes fall back to an edge scan.
    let (calls_src, imports_src, inherits_src, all_src) = conn.query_row(
        "SELECT COUNT(DISTINCT CASE WHEN e.kind = 'calls' THEN s.file_path END),
                COUNT(DISTINCT CASE WHEN e.kind = 'imports' THEN s.file_path END),
                COUNT(DISTINCT CASE WHEN e.kind = 'inherits' THEN s.file_path END),
                COUNT(DISTINCT CASE WHEN e.kind IN ('calls','imports','inherits') THEN s.file_path END)
         FROM edges e JOIN symbols s ON s.id = e.src_id",
        [],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        },
    )?;
    for (key, n) in [
        ("edge_calls_src_files", calls_src),
        ("edge_imports_src_files", imports_src),
        ("edge_inherits_src_files", inherits_src),
        ("edge_all_src_files", all_src),
    ] {
        crate::db::set_meta(&conn, key, &n.to_string())?;
    }
    crate::db::set_meta(
        &conn,
        "callable_symbol_count",
        &callable_symbols.to_string(),
    )?;
    let test_symbols: i64 =
        conn.query_row("SELECT COUNT(*) FROM symbols WHERE is_test = 1", [], |r| {
            r.get(0)
        })?;
    crate::db::set_meta(&conn, "test_symbol_count", &test_symbols.to_string())?;
    crate::db::set_meta(&conn, "unresolved_callee_count", &unres_total.to_string())?;
    conn.execute_batch("COMMIT")?;
    conn.close()
        .map_err(|(_, e)| crate::error::AwError::from(e))?;

    // Atomic swap: fsync the finished `.new`, then rename(2) onto index.db —
    // readers see old or new, never missing (no delete step exists).
    crate::db::publish_new_index(&root)?;
    let db_secs = t0.elapsed().as_secs_f64();
    crate::journal::record_result(total_files, total_files, total_files, false, 0);

    // CPU seconds are measured by the G1 harness (/usr/bin/time -l); the
    // binary reports wall + per-phase timings (std-only constraint).
    // Amendment 11c: parse= is the parallel phase's wall time and extract= the
    // summed per-worker extraction time — the two overlap on parallel builds.
    if stats.skipped_large > 0 {
        eprintln!(
            "[aw] skipped {} file(s) over the 1MB size cap",
            stats.skipped_large
        );
    }
    Ok(format!(
        "aw: built index: files={total} symbols={symbols} edges={edges} parse_errors={perr} unresolved={unres} carried={carried} wall={wall:.2}s (walk={w:.2}s parse={p:.2}s extract={e:.2}s db={d:.2}s)",
        total = total_files,
        symbols = sym_total,
        edges = edge_total,
        perr = perr_total,
        unres = unres_total,
        carried = carried.len(),
        wall = started.elapsed().as_secs_f64(),
        w = walk_secs,
        p = parse_secs,
        e = extract_secs_sum,
        d = db_secs,
    ))
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
/// parallel parse; R6d Blocker 1: 64 MB stacks). `base_idx` keeps the
/// extraction file index aligned with the caller's accumulated `fresh` vector.
#[allow(clippy::type_complexity)]
fn extract_units(
    files: &[crate::walk::WalkedFile],
    base_idx: usize,
) -> Result<(
    Vec<crate::db::FilesRow>,
    Vec<crate::extract::FileExtraction>,
    usize,
    f64,
)> {
    let mut rows: Vec<crate::db::FilesRow> = Vec::with_capacity(files.len());
    let mut extractions: Vec<crate::extract::FileExtraction> = Vec::with_capacity(files.len());
    let mut error_files = 0usize;
    let mut extract_secs_sum = 0f64;
    let workers = std::env::var("AW_WORKERS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
        })
        .clamp(1, 8);
    let chunk_len = (files.len().div_ceil(workers)).max(1);
    std::thread::scope(|scope| -> Result<()> {
        let mut handles = Vec::new();
        for (chunk_no, chunk) in files.chunks(chunk_len).enumerate() {
            let chunk = chunk.to_vec();
            let handle = std::thread::Builder::new()
                .spawn_scoped(
                    scope,
                    move || -> Result<(Vec<(crate::db::FilesRow, crate::extract::FileExtraction)>, f64)> {
                        let mut units = Vec::with_capacity(chunk.len());
                        let mut secs = 0f64;
                        let mut parser = crate::parse::ParserSlot::new();
                        for (local, f) in chunk.into_iter().enumerate() {
                            let idx = base_idx + chunk_no * chunk_len + local;
                            let bytes = crate::walk::read_bounded(&f.abs)?;
                            let lang = crate::parse::Lang::from_rel(&f.rel);
                            let tree = parser.parse(&bytes, lang)?;
                            let crate::parse::SyntaxStats {
                                error_nodes,
                                status,
                                code_lines,
                            } = crate::parse::syntax_stats(tree.root_node(), &bytes);
                            let e0 = Instant::now();
                            let fx = crate::extract::extract_file_with_code_lines(
                                idx,
                                &f.rel,
                                &bytes,
                                tree.root_node(),
                                code_lines,
                            );
                            secs += e0.elapsed().as_secs_f64();
                            let generated = i64::from(crate::walk::is_generated(&f.rel, &bytes));
                            units.push((
                                crate::db::FilesRow {
                                    rel: f.rel,
                                    size: f.size as i64,
                                    mtime: f.mtime,
                                    content_hash: crate::hash::fnv1a64(&bytes) as i64,
                                    parse_status: status,
                                    error_nodes,
                                    unresolved: 0,
                                    generated,
                                },
                                fx,
                            ));
                        }
                        Ok((units, secs))
                    },
                )?;
            handles.push(handle);
        }
        for h in handles {
            let (units, secs) = h
                .join()
                .map_err(|_| AwError::Internal("parse worker panicked".to_string()))??;
            extract_secs_sum += secs;
            for (row, fx) in units {
                if row.error_nodes > 0 {
                    error_files += 1;
                }
                rows.push(row);
                extractions.push(fx);
            }
        }
        Ok(())
    })?;
    Ok((rows, extractions, error_files, extract_secs_sum))
}
