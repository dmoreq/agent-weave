//! Index writing and finishing: row inserts, carry-forward, FTS population,
//! meta counts/identity/universe, and the resolved-index swap.

use super::*;

pub(super) fn resolve_index(
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

pub(super) fn write_rows(
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

pub(super) fn write_fts_indexes(
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

pub(super) fn write_meta_counts(conn: &rusqlite::Connection, root: &Path) -> Result<MetaTotals> {
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

pub(super) fn write_meta_universe(conn: &rusqlite::Connection) -> Result<()> {
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
pub(super) fn built_index_line(prep: &BuildPrep, totals: &MetaTotals, secs: PhaseSecs) -> String {
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
