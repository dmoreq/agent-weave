//! Entry point: subcommand dispatch, build orchestration (full atomic
//! replacement with the per-file fast path), and exit-code mapping.
//! Panic-free boundary: a panic aborts with exit 6, never partial stdout.
#![forbid(unsafe_code)]

mod cli;
mod db;
mod doctor;
mod error;
mod extract;
mod hash;
mod insights;
mod parse;
mod query;
mod walk;

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use clap::Parser;
use error::{KgError, Result};

fn main() {
    let cli = cli::Cli::parse();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(cli)));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            eprintln!("{e}");
            std::process::exit(e.exit_code());
        }
        Err(_panic) => {
            eprintln!("kg: internal: unexpected panic — no output written");
            std::process::exit(6);
        }
    }
}

fn run(cli: cli::Cli) -> Result<()> {
    let root_arg = cli.root.as_deref();
    match cli.cmd {
        cli::Cmd::Build { force, max_parse_error_rate } => {
            run_build(root_arg, force, max_parse_error_rate)
        }
        cli::Cmd::Callers { name, limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            let out = query::callers(&conn, &root, &name, limit)?;
            query::print_output(&out);
            Ok(())
        }
        cli::Cmd::Callees { name, limit, depth } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            let out = query::callees(&conn, &root, &name, limit, depth)?;
            query::print_output(&out);
            Ok(())
        }
        cli::Cmd::Impact { file, limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            let out = query::impact(&conn, &root, &file, limit)?;
            query::print_output(&out);
            Ok(())
        }
        cli::Cmd::Stats {} => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            println!("{}", insights::stats(&conn, &root)?);
            Ok(())
        }
        cli::Cmd::Hotspots { limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in insights::hotspots(&conn, &root, limit)? {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Report {} => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in insights::report(&conn, &root)? {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Doctor { invalidate } => {
            let root = walk::discover_root(root_arg)?;
            if invalidate {
                doctor::invalidate(&root)
            } else {
                doctor::report(&root)
            }
        }
    }
}

fn run_build(root_arg: Option<&Path>, force: bool, max_parse_error_rate: f64) -> Result<()> {
    if !max_parse_error_rate.is_finite() || !(0.0..=1.0).contains(&max_parse_error_rate) {
        return Err(KgError::Usage(format!(
            "--max-parse-error-rate must be a finite value in [0, 1], got {max_parse_error_rate}"
        )));
    }
    let started = Instant::now();
    let root = walk::discover_root(root_arg)?;
    let _lock = db::BuildLock::acquire(&root)?;

    // Walk.
    let t0 = Instant::now();
    let stats = walk::walk(&root, walk::SOURCE_EXTS)?;
    let walk_secs = t0.elapsed().as_secs_f64();

    // Fast path: files unchanged since a compatible previous index.
    let old = if force { None } else { db::probe_old(&root)? };
    let old_map = match &old {
        Some(c) => db::old_files(c)?,
        None => Default::default(),
    };
    let mut carried: HashSet<String> = HashSet::new();
    let mut fresh: Vec<walk::WalkedFile> = Vec::new();
    for f in &stats.files {
        let mtime = db::stat_mtime(&f.abs).unwrap_or(0);
        match old_map.get(&f.rel) {
            Some((om, osz)) if *om == mtime && *osz == f.size as i64 => {
                carried.insert(f.rel.clone());
            }
            _ => fresh.push(walk::WalkedFile {
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
            let dirty = db::dirty_closure(c, &changed)?;
            for rel in dirty {
                if carried.remove(&rel) {
                    if let Some(f) = stats.files.iter().find(|f| f.rel == rel) {
                        fresh.push(walk::WalkedFile {
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
    let mut fresh_rows: Vec<db::FilesRow> = Vec::with_capacity(fresh.len());
    let mut extractions: Vec<extract::FileExtraction> = Vec::with_capacity(fresh.len());
    let mut fresh_error_files = 0usize;
    for (idx, f) in fresh.iter().enumerate() {
        let bytes = std::fs::read(&f.abs)?;
        let parsed = parse::parse_source(&bytes, parse::Lang::from_rel(&f.rel))?;
        if parsed.error_nodes > 0 {
            fresh_error_files += 1;
        }
        extractions.push(extract::extract_file(idx, &f.rel, &bytes, &parsed));
        fresh_rows.push(db::FilesRow {
            rel: f.rel.clone(),
            size: f.size as i64,
            mtime: db::stat_mtime(&f.abs).unwrap_or(0),
            content_hash: hash::fnv1a64(&bytes) as i64,
            parse_status: parsed.status,
            error_nodes: parsed.error_nodes,
            unresolved: 0,
        });
    }
    let parse_secs = t0.elapsed().as_secs_f64();

    // Parse-quality gate (CUJ8): rate over ALL files, abort before DB work.
    let total_files = stats.files.len();
    let carried_errors = match &old {
        Some(c) => db::old_error_count(
            c,
            &carried.iter().cloned().collect::<Vec<_>>(),
        )?,
        None => 0,
    };
    let error_files = fresh_error_files + carried_errors;
    if total_files > 0 && !force {
        let rate = error_files as f64 / total_files as f64;
        if rate > max_parse_error_rate {
            let _ = std::fs::remove_file(db::db_new_path(&root));
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
        Some(c) => db::load_carried(c, &carried)?,
        None => extract::CarriedIndex::default(),
    };
    let resolution = extract::resolve(extractions, carried_index);
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
    let conn = db::create_fresh(&root)?;
    if !carried.is_empty() {
        let unchanged: Vec<(String, i64, i64)> = carried
            .iter()
            .filter_map(|rel| old_map.get(rel).map(|(m, s)| (rel.clone(), *m, *s)))
            .collect();
        db::carry_forward_unchanged(&conn, &root, &unchanged)?;
    }
    let fresh_file_ids = db::insert_files(&conn, &fresh_rows)?;

    // Symbols: fresh rows insert; carried rows keep their previous ids.
    let fresh_sym_count = resolution.existing_ids.iter().filter(|e| e.is_none()).count();
    let mut fresh_symbols: Vec<extract::NewSymbol> = Vec::with_capacity(fresh_sym_count);
    let mut fresh_pos: Vec<Option<usize>> = Vec::with_capacity(resolution.symbols.len());
    for (sym, existing) in resolution.symbols.iter().zip(&resolution.existing_ids) {
        if existing.is_none() {
            fresh_pos.push(Some(fresh_symbols.len()));
            fresh_symbols.push(sym.clone());
        } else {
            fresh_pos.push(None);
        }
    }
    let fresh_sym_ids = db::insert_symbols(&conn, &fresh_symbols, &fresh_file_ids)?;

    // Edges: endpoints map fresh -> inserted id, carried -> previous row id.
    let mut edge_rows: Vec<db::EdgeRow> = Vec::with_capacity(resolution.edges.len());
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
        edge_rows.push(db::EdgeRow {
            src_id,
            dst_id,
            dst_name: e.dst_name.clone(),
            kind: e.kind,
            confidence: e.confidence,
            site_line: e.site_line,
        });
    }
    db::insert_edge_rows(&conn, &edge_rows)?;

    // Meta manifest; counts are authoritative SQL totals over the whole index.
    db::set_meta(&conn, "kg_version", env!("CARGO_PKG_VERSION"))?;
    db::set_meta(&conn, "grammar_version", db::GRAMMAR_VERSION)?;
    db::set_meta(
        &conn,
        "built_at",
        &std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs().to_string())
            .unwrap_or_else(|_| "0".into()),
    )?;
    db::set_meta(&conn, "source_rev", &db::source_rev(&root))?;
    let sql_count = |sql: &str| -> Result<i64> {
        Ok(conn.query_row(sql, [], |r| r.get(0))?)
    };
    let sym_total = sql_count("SELECT COUNT(*) FROM symbols")?;
    let edge_total = sql_count("SELECT COUNT(*) FROM edges")?;
    let file_total = sql_count("SELECT COUNT(*) FROM files")?;
    let perr_total = sql_count("SELECT COALESCE(SUM(parse_status != 'ok'), 0) FROM files")?;
    db::set_meta(&conn, "file_count", &file_total.to_string())?;
    db::set_meta(&conn, "symbol_count", &sym_total.to_string())?;
    db::set_meta(&conn, "edge_count", &edge_total.to_string())?;
    db::set_meta(&conn, "parse_error_count", &perr_total.to_string())?;
    // Unresolved total is the authoritative SQL sum of per-file counts, stable
    // across fast-path builds (carried rows keep their stored count).
    let unres_total = sql_count("SELECT COALESCE(SUM(unresolved), 0) FROM files")?;
    db::set_meta(&conn, "unresolved_callee_count", &unres_total.to_string())?;
    drop(conn);

    // Atomic swap: rename(2) onto index.db — readers see old or new, never
    // missing (no delete step exists).
    std::fs::rename(db::db_new_path(&root), db::db_path(&root))?;
    let db_secs = t0.elapsed().as_secs_f64();

    // CPU seconds are measured by the G1 harness (/usr/bin/time -l); the
    // binary reports wall + per-phase timings (std-only constraint).
    println!(
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
    );
    if stats.skipped_large > 0 {
        eprintln!("[kg] skipped {} file(s) over the 1MB size cap", stats.skipped_large);
    }
    Ok(())
}

fn resolve_sym_id(
    sym_idx: usize,
    fresh_pos: &[Option<usize>],
    fresh_sym_ids: &[i64],
    resolution: &extract::Resolution,
) -> Option<i64> {
    match fresh_pos.get(sym_idx).copied().flatten() {
        Some(p) => fresh_sym_ids.get(p).copied(),
        None => resolution.existing_ids.get(sym_idx).copied().flatten(),
    }
}
