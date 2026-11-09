//! Build-phase inserts: files, symbols, edges, FTS population, and
//! carry-forward of unchanged rows under the caller-owned transaction.

use super::*;

/// Insert unresolved edge sites for the fresh rows (schema v6, Amendment 19).
pub fn insert_unresolved_calls(
    conn: &Connection,
    rows: &[(i64, &str, EdgeSiteKind, usize)],
) -> Result<()> {
    if rows.iter().any(|(_, _, _, site_line)| *site_line == 0) {
        return Err(crate::error::EgError::Internal(
            "unresolved edge row has site_line 0".to_string(),
        ));
    }
    let mut stmt = conn.prepare(
        "INSERT INTO unresolved_calls(file_id, dst_name, kind, site_line) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (file_id, name, kind, site_line) in rows {
        stmt.execute(rusqlite::params![
            file_id,
            name,
            kind.as_str(),
            *site_line as i64
        ])?;
    }
    Ok(())
}

/// Symbols + inherits pairs of the carried set, for resolution tables.
pub fn load_carried(old: &Connection, rels: &HashSet<String>) -> Result<CarriedIndex> {
    if rels.is_empty() {
        return Ok(CarriedIndex {
            symbols: Vec::new(),
            inherits: Vec::new(),
        });
    }
    let mut rel_params: Vec<&str> = rels.iter().map(String::as_str).collect();
    rel_params.sort_unstable();
    Ok(CarriedIndex {
        symbols: load_carried_symbols(old, &rel_params)?,
        inherits: load_carried_inherits(old, &rel_params)?,
    })
}

/// One carried-symbol row: the symbol columns plus its file path. Named so a
/// column reorder in the SELECT cannot silently swap the adjacent `String`
/// columns (kind, name, qualname).
struct CarriedSymbolRow {
    row_id: i64,
    kind: String,
    name: String,
    qualname: String,
    path: String,
}

fn load_carried_symbols(old: &Connection, rel_params: &[&str]) -> Result<Vec<CarriedSymbol>> {
    let mut symbols = Vec::new();
    // Bound the read to the carried files instead of scanning every symbol.
    // Chunked placeholders match the inherits query below; the final sort by
    // symbol id restores the old index order across chunks (resolution tables
    // are order-sensitive for duplicate qualnames).
    let mut rows: Vec<CarriedSymbolRow> = Vec::new();
    for_each_chunk(old, rel_params, |old, placeholders, chunk| {
        let mut stmt = old.prepare(&format!(
            "SELECT s.id, s.kind, s.name, s.qualname, f.path
             FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE f.path IN ({placeholders})"
        ))?;
        let part = stmt.query_map(rusqlite::params_from_iter(chunk.iter().copied()), |r| {
            Ok(CarriedSymbolRow {
                row_id: r.get::<_, i64>(0)?,
                kind: r.get::<_, String>(1)?,
                name: r.get::<_, String>(2)?,
                qualname: r.get::<_, String>(3)?,
                path: r.get::<_, String>(4)?,
            })
        })?;
        for row in part {
            rows.push(row?);
        }
        Ok(())
    })?;
    rows.sort_by_key(|row| row.row_id);
    for row in rows {
        // A carried trait/interface is a valid impl target on the fast path;
        // dropping it made incremental Rust impls permanently unresolved
        // (R12 Critical). Unknown spellings stay dropped, as before.
        let Some(kind) = SymbolKind::parse(&row.kind) else {
            continue;
        };
        let (module, _) = crate::extract::module_path_for(&row.path);
        symbols.push(CarriedSymbol {
            row_id: row.row_id,
            kind,
            name: row.name,
            qualname: row.qualname,
            module,
        });
    }
    Ok(symbols)
}

fn load_carried_inherits(old: &Connection, rel_params: &[&str]) -> Result<Vec<(String, String)>> {
    let mut inherits = Vec::new();
    for_each_chunk(old, rel_params, |old, placeholders, chunk| {
        let mut stmt = old.prepare(&format!(
            "SELECT s.qualname, e.dst_name
             FROM edges e JOIN symbols s ON s.id = e.src_id
             JOIN files f ON f.id = s.file_id
             WHERE e.kind = '{}' AND e.dst_name IS NOT NULL
               AND f.path IN ({placeholders})",
            EdgeKind::Inherits.as_str()
        ))?;
        let pairs = stmt.query_map(rusqlite::params_from_iter(chunk.iter().copied()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for p in pairs {
            inherits.push(p?);
        }
        Ok(())
    })?;
    Ok(inherits)
}

/// One hash-carry row for `carry_forward_unchanged`: the SELECT locates the
/// old row by (rel, old_mtime, size) and refreshes its stored mtime to
/// `current_mtime` when the fresh checkout differs (Amendment 9).
pub struct CarryRow {
    pub rel: String,
    pub old_mtime: i64,
    pub size: i64,
    pub current_mtime: i64,
}

/// Copy rows for files matching the fast-path keys: the SELECT identifies the
/// old row by (path, old_mtime, size); hash-carried rows (Amendment 9) then
/// get their stored mtime refreshed to the fresh checkout's stat so the
/// staleness contract stays truthful. Attaches the old DB itself (SQLite
/// ATTACH takes no bound parameters).
/// Amendment 11b: the caller owns the build transaction — no BEGIN/COMMIT here.
pub fn carry_forward_unchanged(
    new: &Connection,
    root: &Path,
    unchanged: &[CarryRow],
) -> Result<()> {
    let old_path = db_path(root);
    let quoted = old_path.to_string_lossy().replace('\'', "''");
    new.execute_batch(&format!("ATTACH DATABASE '{quoted}' AS olddb"))?;
    {
        let mut stmt = new.prepare(
            "INSERT INTO files(id, path, size, mtime, content_hash, parse_status, error_nodes, unresolved)
             SELECT id, path, size, mtime, content_hash, parse_status, error_nodes, unresolved
             FROM olddb.files WHERE path = ?1 AND mtime = ?2 AND size = ?3",
        )?;
        let mut refresh_mtime = new.prepare("UPDATE files SET mtime = ?1 WHERE path = ?2")?;
        for row in unchanged {
            let inserted = stmt.execute(rusqlite::params![row.rel, row.old_mtime, row.size])?;
            if inserted > 0 && row.current_mtime != row.old_mtime {
                refresh_mtime.execute(rusqlite::params![row.current_mtime, row.rel])?;
            }
        }
    }
    new.execute_batch(
        "INSERT INTO symbols(id, kind, name, qualname, file_id, file_path, line_start, line_end, signature, is_test)
         SELECT id, kind, name, qualname, file_id, file_path, line_start, line_end, signature, is_test
         FROM olddb.symbols WHERE file_id IN (SELECT id FROM files);
         INSERT INTO edges(id, src_id, dst_id, dst_name, kind, confidence, site_line)
         SELECT id, src_id, dst_id, dst_name, kind, confidence, site_line
         FROM olddb.edges
         WHERE src_id IN (SELECT id FROM symbols)
           AND (dst_id IS NULL OR dst_id IN (SELECT id FROM symbols));
         INSERT INTO unresolved_calls(file_id, dst_name, kind, site_line)
         SELECT file_id, dst_name, kind, site_line FROM olddb.unresolved_calls
         WHERE file_id IN (SELECT id FROM files);",
    )?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct FilesRow {
    pub rel: String,
    pub size: i64,
    pub mtime: i64,
    pub content_hash: i64,
    pub parse_status: &'static str,
    pub error_nodes: usize,
    /// Per-file unresolved-callee count (filled after resolve()).
    pub unresolved: usize,
}

/// Amendment 11b: caller holds the build transaction; 500-row prepared
/// batches remain (statement reuse), the per-chunk BEGIN/COMMIT fsyncs do not.
pub fn insert_files(conn: &Connection, rows: &[FilesRow]) -> Result<Vec<i64>> {
    let mut ids = Vec::with_capacity(rows.len());
    let mut stmt = conn.prepare(
        "INSERT INTO files(path, size, mtime, content_hash, parse_status, error_nodes, unresolved)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for r in rows {
        stmt.execute(rusqlite::params![
            r.rel,
            r.size,
            r.mtime,
            r.content_hash,
            r.parse_status,
            r.error_nodes as i64,
            r.unresolved as i64
        ])?;
        ids.push(conn.last_insert_rowid());
    }
    Ok(ids)
}

pub fn insert_symbols(
    conn: &Connection,
    symbols: &[&NewSymbol],
    file_ids: &[i64],
    file_paths: &[String],
) -> Result<Vec<i64>> {
    let mut ids = Vec::with_capacity(symbols.len());
    let mut stmt = conn.prepare(
        "INSERT INTO symbols(kind, name, qualname, file_id, file_path, line_start, line_end, signature, is_test)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )?;
    for s in symbols {
        stmt.execute(rusqlite::params![
            s.kind.as_str(),
            s.name,
            s.qualname,
            file_ids[s.file_idx],
            file_paths[s.file_idx].as_str(),
            s.line_start as i64,
            s.line_end as i64,
            s.signature,
            i64::from(s.is_test)
        ])?;
        ids.push(conn.last_insert_rowid());
    }
    Ok(ids)
}

/// Fully resolved edge row: endpoint ids already mapped (fresh -> inserted id,
/// carried -> previous row id).
#[derive(Debug)]
pub struct EdgeRow<'a> {
    pub src_id: i64,
    pub dst_id: Option<i64>,
    pub dst_name: &'a str,
    pub kind: &'static str,
    pub confidence: Confidence,
    pub site_line: usize,
}

pub fn insert_edge_rows(conn: &Connection, rows: &[EdgeRow<'_>]) -> Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO edges(src_id, dst_id, dst_name, kind, confidence, site_line)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for e in rows {
        stmt.execute(rusqlite::params![
            e.src_id,
            e.dst_id,
            e.dst_name,
            e.kind,
            e.confidence.as_str(),
            e.site_line as i64
        ])?;
    }
    Ok(())
}

/// Populate the FTS5 symbol-search index from the fully-inserted symbols
/// table (Amendment 12b): one pass covering fresh AND carried rows, called
/// inside the build transaction after `insert_edge_rows`. Amendment 14
/// widens the indexed text with path and signature tokens — the two
/// haystacks grep already matches and a name-only index could not.
/// FTS text for one symbol (Amendment 12b/14/R7): qualname and name repeat so
/// bm25 ranks identity matches above signature-only matches now that signatures
/// carry full parameter lists. Shared by the SELECT and in-memory paths so the
/// indexed bytes cannot drift.
fn fts_text(qualname: &str, name: &str, path: &str, signature: &str) -> String {
    format!(
        "{q} {q} {n} {n} {p} {s}",
        q = crate::extract::split_ident(qualname),
        n = crate::extract::split_ident(name),
        p = crate::extract::split_ident(path),
        s = crate::extract::split_ident(signature)
    )
}

pub fn populate_symbols_fts(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.qualname, s.name, f.path, s.signature
         FROM symbols s JOIN files f ON f.id = s.file_id",
    )?;
    let rows: Vec<(i64, String, String, String, String)> = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let refs: Vec<(i64, &str, &str, &str, &str)> = rows
        .iter()
        .map(|(id, qualname, name, path, signature)| {
            (
                *id,
                qualname.as_str(),
                name.as_str(),
                path.as_str(),
                signature.as_str(),
            )
        })
        .collect();
    populate_symbols_fts_rows(conn, &refs)
}

/// Populate FTS from in-memory rows (force path): one row is
/// `(symbol id, qualname, name, file path, signature)`. Avoids the SELECT +
/// JOIN over every inserted symbol. The indexed bytes match `fts_text` exactly.
pub fn populate_symbols_fts_rows(
    conn: &Connection,
    rows: &[(i64, &str, &str, &str, &str)],
) -> Result<()> {
    let mut ins = conn.prepare("INSERT INTO symbols_fts(rowid, text) VALUES (?1, ?2)")?;
    for &(id, qualname, name, path, signature) in rows {
        ins.execute(rusqlite::params![
            id,
            fts_text(qualname, name, path, signature)
        ])?;
    }
    Ok(())
}
