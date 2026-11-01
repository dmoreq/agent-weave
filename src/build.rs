//! Build orchestration: full atomic replacement with the per-file fast path,
//! dirty closure, parse-quality gate, and meta manifest. Shared by the CLI
//! (prints the stats line) and the MCP server (returns it as tool text).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use crate::error::{KgError, Result};

pub fn run_build(root_arg: Option<&Path>, force: bool, max_parse_error_rate: f64) -> Result<String> {
    if !max_parse_error_rate.is_finite() || !(0.0..=1.0).contains(&max_parse_error_rate) {
        return Err(KgError::Usage(format!(
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
    let old = if force { None } else { crate::db::probe_old(&root)? };
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
        let mtime = crate::db::stat_mtime(&f.abs).unwrap_or(0);
        match old_map.get(&f.rel) {
            Some((om, osz, _oh)) if *om == mtime && *osz == f.size as i64 => {
                carried.insert(f.rel.clone());
            }
            // Dual-key (Amendment 9): (mtime, size) mismatch, but same size and
            // same content hash -> carry (the CI cache-restore case). One
            // read+hash instead of parse+extract.
            Some((_om, osz, oh)) if *osz == f.size as i64 => {
                match std::fs::read(&f.abs) {
                    Ok(bytes) if crate::hash::fnv1a64(&bytes) as i64 == *oh => {
                        carried.insert(f.rel.clone());
                        mtime_updates.insert(f.rel.clone(), mtime);
                    }
                    _ => fresh.push(crate::walk::WalkedFile {
                        abs: f.abs.clone(),
                        rel: f.rel.clone(),
                        size: f.size,
                    }),
                }
            }
            _ => fresh.push(crate::walk::WalkedFile {
                abs: f.abs.clone(),
                rel: f.rel.clone(),
                size: f.size,
            }),
        }
    }

    // Dirty closure (fast-path correctness): a carried file with any edge into
    // a changed file must be re-extracted, or its copied edge dies with the old
    // destination id (transitive: callers of re-extracted files too).
    if let Some(c) = &old {
        let changed: Vec<String> = fresh.iter().map(|f| f.rel.clone()).collect();
        if !changed.is_empty() {
            let dirty = crate::db::dirty_closure(c, &changed)?;
            for rel in dirty {
                if carried.remove(&rel) {
                    if let Some(f) = stats.files.iter().find(|f| f.rel == rel) {
                        fresh.push(crate::walk::WalkedFile {
                            abs: f.abs.clone(),
                            rel: f.rel.clone(),
                            size: f.size,
                        });
                    }
                }
            }
            fresh.sort_by(|a, b| a.rel.cmp(&b.rel));
        }
    }

    // Parse + extract fresh files only (streamed; no corpus accumulation).
    let t0 = Instant::now();
    let mut fresh_rows: Vec<crate::db::FilesRow> = Vec::with_capacity(fresh.len());
    let mut extractions: Vec<crate::extract::FileExtraction> = Vec::with_capacity(fresh.len());
    let mut fresh_error_files = 0usize;
    for (idx, f) in fresh.iter().enumerate() {
        let bytes = std::fs::read(&f.abs)?;
        let parsed = crate::parse::parse_source(&bytes, crate::parse::Lang::from_rel(&f.rel))?;
        if parsed.error_nodes > 0 {
            fresh_error_files += 1;
        }
        extractions.push(crate::extract::extract_file(idx, &f.rel, &bytes, &parsed));
        fresh_rows.push(crate::db::FilesRow {
            rel: f.rel.clone(),
            size: f.size as i64,
            mtime: crate::db::stat_mtime(&f.abs).unwrap_or(0),
            content_hash: crate::hash::fnv1a64(&bytes) as i64,
            parse_status: parsed.status,
            error_nodes: parsed.error_nodes,
            unresolved: 0,
        });
    }
    let parse_secs = t0.elapsed().as_secs_f64();

    // Parse-quality gate (CUJ8): rate over ALL files, abort before DB work.
    let total_files = stats.files.len();
    let carried_errors = match &old {
        Some(c) => crate::db::old_error_count(
            c,
            &carried.iter().cloned().collect::<Vec<_>>(),
        )?,
        None => 0,
    };
    let error_files = fresh_error_files + carried_errors;
    if total_files > 0 && !force {
        let rate = error_files as f64 / total_files as f64;
        if rate > max_parse_error_rate {
            let _ = std::fs::remove_file(crate::db::db_new_path(&root));
            return Err(KgError::ParseQuality {
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
    let extract_secs = t0.elapsed().as_secs_f64();
    // Per-file unresolved counts: parallel to the fresh-file order.
    for (row, unresolved) in fresh_rows
        .iter_mut()
        .zip(&resolution.unresolved_per_file)
    {
        row.unresolved = *unresolved;
    }

    // DB: fresh schema at index.db.new -> carry forward -> insert -> swap.
    let t0 = Instant::now();
    let conn = crate::db::create_fresh(&root)?;
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

    // Symbols: fresh rows insert; carried rows keep their previous ids.
    let fresh_sym_count = resolution.existing_ids.iter().filter(|e| e.is_none()).count();
    let mut fresh_symbols: Vec<crate::extract::NewSymbol> = Vec::with_capacity(fresh_sym_count);
    let mut fresh_pos: Vec<Option<usize>> = Vec::with_capacity(resolution.symbols.len());
    for (sym, existing) in resolution.symbols.iter().zip(&resolution.existing_ids) {
        if existing.is_none() {
            fresh_pos.push(Some(fresh_symbols.len()));
            fresh_symbols.push(sym.clone());
        } else {
            fresh_pos.push(None);
        }
    }
    let fresh_sym_ids = crate::db::insert_symbols(&conn, &fresh_symbols, &fresh_file_ids)?;

    // Edges: endpoints map fresh -> inserted id, carried -> previous row id.
    let mut edge_rows: Vec<crate::db::EdgeRow> = Vec::with_capacity(resolution.edges.len());
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
            dst_name: e.dst_name.clone(),
            kind: e.kind,
            confidence: e.confidence,
            site_line: e.site_line,
        });
    }
    crate::db::insert_edge_rows(&conn, &edge_rows)?;

    // Meta manifest; counts are authoritative SQL totals over the whole index.
    crate::db::set_meta(&conn, "kg_version", env!("CARGO_PKG_VERSION"))?;
    crate::db::set_meta(&conn, "grammar_version", crate::db::GRAMMAR_VERSION)?;
    crate::db::set_meta(
        &conn,
        "built_at",
        // Millisecond precision: the payload journal (Amendment 10c) compares
        // built_at epochs to tell superseded payload entries apart — sub-second
        // builds must not collide on the same epoch value.
        &std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis().to_string())
            .unwrap_or_else(|_| "0".into()),
    )?;
    crate::db::set_meta(&conn, "source_rev", &crate::db::source_rev(&root))?;
    let sql_count = |sql: &str| -> Result<i64> {
        Ok(conn.query_row(sql, [], |r| r.get(0))?)
    };
    let sym_total = sql_count("SELECT COUNT(*) FROM symbols")?;
    let edge_total = sql_count("SELECT COUNT(*) FROM edges")?;
    let file_total = sql_count("SELECT COUNT(*) FROM files")?;
    let perr_total = sql_count("SELECT COALESCE(SUM(parse_status != 'ok'), 0) FROM files")?;
    crate::db::set_meta(&conn, "file_count", &file_total.to_string())?;
    crate::db::set_meta(&conn, "symbol_count", &sym_total.to_string())?;
    crate::db::set_meta(&conn, "edge_count", &edge_total.to_string())?;
    crate::db::set_meta(&conn, "parse_error_count", &perr_total.to_string())?;
    // Unresolved total is the authoritative SQL sum of per-file counts, stable
    // across fast-path builds (carried rows keep their stored count).
    let unres_total = sql_count("SELECT COALESCE(SUM(unresolved), 0) FROM files")?;
    crate::db::set_meta(&conn, "unresolved_callee_count", &unres_total.to_string())?;
    drop(conn);

    // Atomic swap: rename(2) onto index.db — readers see old or new, never
    // missing (no delete step exists).
    std::fs::rename(crate::db::db_new_path(&root), crate::db::db_path(&root))?;
    let db_secs = t0.elapsed().as_secs_f64();
    crate::journal::record_result(total_files, total_files, total_files, false, 0);

    // CPU seconds are measured by the G1 harness (/usr/bin/time -l); the
    // binary reports wall + per-phase timings (std-only constraint).
    if stats.skipped_large > 0 {
        eprintln!("[kg] skipped {} file(s) over the 1MB size cap", stats.skipped_large);
    }
    Ok(format!(
        "kg: built index: files={total} symbols={symbols} edges={edges} parse_errors={perr} unresolved={unres} carried={carried} wall={wall:.2}s (walk={w:.2}s parse={p:.2}s extract={e:.2}s db={d:.2}s)",
        total = total_files,
        symbols = sym_total,
        edges = edge_total,
        perr = perr_total,
        unres = unres_total,
        carried = carried.len(),
        wall = started.elapsed().as_secs_f64(),
        w = walk_secs,
        p = parse_secs,
        e = extract_secs,
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

