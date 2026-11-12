//! `eg symbols`: single-file and directory symbol listings.

use super::*;

/// Display knobs for `eg symbols` (mirrors `ImpactOpts`): `--outline` adds the
/// stored one-line signature.
pub struct SymbolsOpts {
    pub outline: bool,
}

/// Symbols of one indexed file (Amendment 5): `path:line kind qualname` by
/// line; single-file staleness header. With `outline` (Amendment 12a) each
/// line gains the stored one-line signature; without it, byte-identical to
/// the pre-v4 contract.
pub fn symbols(
    ctx: &QueryCtx<'_>,
    file: &str,
    limit: usize,
    opts: SymbolsOpts,
) -> Result<LineQuery> {
    let QueryCtx { conn, .. } = *ctx;
    let SymbolsOpts { outline } = opts;
    let limit = clamp_limit(limit)?;
    let rel = normalize_scope(file);
    // `.` and `./` mean the repository root; the empty prefix matches every
    // indexed file without generating a "/" prefix.
    let rel = if rel == "." {
        String::new()
    } else {
        rel.to_string()
    };
    let Some(rel) = indexed_file_opt(conn, &rel)? else {
        if rel
            .rsplit('.')
            .next()
            .is_some_and(crate::parse::is_source_ext)
        {
            return Err(EgError::not_indexed(file));
        }
        return symbols_directory(ctx, &rel, limit, SymbolsOpts { outline });
    };
    symbols_file(ctx, file, &rel, limit, outline)
}

/// The single-file branch of [`symbols`]: symbol rows with the optional
/// outline signature, the staleness header, and the cap footer. `file` is the
/// original query text, kept for the not-indexed error; `rel` is the indexed
/// path `indexed_file_opt` resolved.
fn symbols_file(
    ctx: &QueryCtx<'_>,
    file: &str,
    rel: &str,
    limit: usize,
    outline: bool,
) -> Result<LineQuery> {
    let QueryCtx { conn, .. } = *ctx;
    let rows = file_symbol_rows(conn, rel, outline)?;
    if rows.is_empty() {
        return Err(EgError::not_indexed(file));
    }
    let files = BTreeSet::from([rel.to_string()]);
    let (header, stale) = staleness_header(ctx, &files);
    let n_symbols = rows.len();
    let shown = n_symbols.min(limit);
    let data_lines: Vec<String> = rows
        .into_iter()
        .take(shown)
        .map(|row| symbol_line(rel, &row))
        .collect();
    let mut out = Vec::new();
    if let Some(header) = header {
        out.push(header);
    }
    out.extend(data_lines);
    let cap = finish_capped(&mut out, shown, n_symbols, stale, "");
    Ok(LineQuery::new(out, cap, shown))
}

fn file_symbol_rows(conn: &Connection, rel: &str, outline: bool) -> Result<Vec<SymbolRow>> {
    let signature_col = if outline { "s.signature" } else { "''" };
    let sql = format!(
        "SELECT s.kind, s.qualname, s.line_start, {signature_col}
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE f.path = ?1
         ORDER BY s.line_start, s.qualname"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([rel], |r| {
            Ok(SymbolRow {
                kind: r.get(0)?,
                qualname: r.get(1)?,
                line: r.get(2)?,
                signature: r.get(3)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// One `path:line kind qualname` row; `--outline` appends the signature.
fn symbol_line(path: &str, row: &SymbolRow) -> String {
    let mut text = format!("{path}:{} {} {}", row.line, row.kind, row.qualname);
    if !row.signature.is_empty() {
        text.push(' ');
        text.push_str(&row.signature);
    }
    text
}

/// Group symbol rows under their file path and finish with the cap/journal
/// tail — the per-file rendering shared by the `symbols` directory arm.
fn dir_line_query(
    mut out: Vec<String>,
    rows: &[(String, SymbolRow)],
    total: usize,
    stale: bool,
) -> LineQuery {
    let mut last_path = "";
    for (path, row) in rows {
        if path != last_path {
            out.push(path.clone());
            last_path = path;
        }
        out.push(symbol_line(path, row));
    }
    let shown = rows.len();
    let cap = finish_capped(&mut out, shown, total, stale, "");
    LineQuery::new(out, cap, shown)
}

/// Directory mode for `eg symbols <dir>` (Amendment 19): indexed prefix only,
/// grouped per file, cap counts symbol data lines.
fn symbols_directory(
    ctx: &QueryCtx<'_>,
    dir: &str,
    limit: usize,
    SymbolsOpts { outline }: SymbolsOpts,
) -> Result<LineQuery> {
    let QueryCtx { conn, .. } = *ctx;
    validate_path_scope(dir, true)?;
    let display = if dir.is_empty() { "." } else { dir };
    let prefix = if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}/")
    };
    let count = |from: &str| -> Result<i64> {
        Ok(conn.query_row(
            &format!("SELECT COUNT(*) FROM {from} WHERE substr(f.path, 1, length(?1)) = ?1"),
            [&prefix],
            |r| r.get::<_, i64>(0),
        )?)
    };
    let total = count("symbols s JOIN files f ON f.id = s.file_id")?;
    if total == 0 {
        return Err(EgError::NoResults(format!(
            "0 symbols in '{dir}' (indexed)"
        )));
    }
    let n_files = count("files f")?;
    let rows = dir_symbol_rows(conn, &prefix, outline, limit)?;
    let displayed_paths: BTreeSet<String> = rows.iter().map(|(p, _)| p.clone()).collect();
    let (header, stale) = staleness_header(ctx, &displayed_paths);
    let mut out = Vec::new();
    if let Some(header) = header {
        out.push(header);
    }
    out.push(format!(
        "eg: dir {display} — {n_files} files, {total} symbols"
    ));
    Ok(dir_line_query(out, &rows, total as usize, stale))
}

fn dir_symbol_rows(
    conn: &Connection,
    prefix: &str,
    outline: bool,
    limit: usize,
) -> Result<Vec<(String, SymbolRow)>> {
    let mut stmt = conn.prepare(
        "SELECT f.path, s.kind, s.qualname, s.line_start, s.signature
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE substr(f.path, 1, length(?1)) = ?1
         ORDER BY f.path, s.line_start, s.qualname LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![prefix, limit as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                SymbolRow {
                    kind: r.get(1)?,
                    qualname: r.get(2)?,
                    line: r.get(3)?,
                    signature: if outline { r.get(4)? } else { String::new() },
                },
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}
