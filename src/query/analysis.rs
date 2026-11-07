//! Inventory and analysis queries: dead candidates, complexity, batch resolve,
//! files, unresolved sites, symbols, search, and duplicate candidates.

use super::*;

/// Referenced-id set shared by the `dead` COUNT and page query: a resolved
/// destination, an AMBIGUOUS qualname match, or a same-file bare-name match.
/// Built per call so the edge-kind vocabulary stays the one shared SQL
/// fragment (`db::edge_kinds_sql`).
fn dead_referenced() -> String {
    let kinds = db::edge_kinds_sql();
    let trait_parent = SymbolKind::Trait.as_str();
    let interface = SymbolKind::Interface.as_str();
    let ambiguous = Confidence::Ambiguous.as_str();
    format!(
        "WITH trait_parents(qualname) AS MATERIALIZED (
           SELECT DISTINCT qualname FROM symbols
            WHERE kind IN ('{trait_parent}', '{interface}')
       ),
       referenced_ids(id) AS MATERIALIZED (
           SELECT dst_id FROM edges
            WHERE kind IN {kinds}
              AND dst_id IS NOT NULL
           UNION
           SELECT s2.id FROM edges e
             JOIN symbols s2 ON s2.qualname = e.dst_name
            WHERE e.kind IN {kinds}
              AND e.confidence = '{ambiguous}'
           UNION
           SELECT s2.id FROM edges e
             JOIN symbols src ON src.id = e.src_id
             JOIN symbols s2 ON s2.name = e.dst_name AND s2.file_id = src.file_id
            WHERE e.kind IN {kinds}
              AND e.confidence = '{ambiguous}'
       )"
    )
}

pub fn dead(
    ctx: &QueryCtx<'_>,
    limit: usize,
    exclude_tests: bool,
    path: Option<&str>,
) -> Result<QueryOutput> {
    let QueryCtx { conn, .. } = *ctx;
    let (limit, scope, params) = scoped_query(path, limit, "f", "path", 1)?;
    let predicate = dead_filter(exclude_tests, &scope);
    let total: usize = conn.query_row(
        &format!(
            "{} SELECT COUNT(*) FROM symbols s JOIN files f ON f.id = s.file_id WHERE {predicate}",
            dead_referenced()
        ),
        rusqlite::params_from_iter(params.iter()),
        |r| r.get::<_, i64>(0),
    )? as usize;
    let sql = format!(
        "{} SELECT s.qualname, f.path, s.line_start
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE {predicate}
         ORDER BY f.path, s.line_start LIMIT {limit}",
        dead_referenced()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        let path: String = r.get(1)?;
        let line: i64 = r.get(2)?;
        Ok(Pointer {
            file: path,
            line,
            tag: "",
            detail: r.get::<_, String>(0)?,
        })
    })?;
    if total == 0 {
        return Err(AwError::NoResultsCaveated {
            caveat: DEAD_CAVEAT,
            detail: "0 dead-code candidates found".to_string(),
        });
    }
    let mut out = collect_output(ctx, rows, limit, "0 dead-code candidates found", total)?;
    prepend_header(&mut out, DEAD_CAVEAT);
    Ok(out)
}

/// Dead-candidate predicate plus its bound path parameter. AMBIGUOUS
/// suppression is per-symbol (R7): a bare-name ambiguous call suppresses only
/// same-named symbols in the caller's own file.
fn dead_filter(exclude_tests: bool, scope: &str) -> String {
    let [function, method, class, trait_kind, interface] = [
        SymbolKind::Function,
        SymbolKind::Method,
        SymbolKind::Class,
        SymbolKind::Trait,
        SymbolKind::Interface,
    ]
    .map(|k| k.as_str());
    let mut predicate = format!(
        "s.kind IN ('{function}', '{method}', '{class}', '{trait_kind}', '{interface}')
           AND s.id NOT IN (SELECT id FROM referenced_ids)
           AND NOT EXISTS (
               SELECT 1 FROM trait_parents tp
                WHERE s.qualname >= tp.qualname || '.'
                  AND s.qualname < tp.qualname || '/'
           )"
    );
    if exclude_tests {
        // Schema v8: `symbols.is_test` classifies once at extraction (test
        // paths, test-shaped names/files, Rust test attributes, TS describe
        // scope), so one column check replaces the v7 LIKE/GLOB list.
        predicate.push_str(" AND s.is_test = 0");
    }
    predicate.push_str(scope);
    predicate
}

/// `aw complexity` (Amendment 19, v9 shape): per-symbol effective-LOC ranking.
pub fn complexity(
    ctx: &QueryCtx<'_>,
    min_loc: usize,
    path: Option<&str>,
    limit: usize,
) -> Result<LineQuery> {
    let QueryCtx { conn, .. } = *ctx;
    let (limit, gen_filter, scope_params) = scoped_query(path, limit, "s", "file_path", 2)?;
    let mut params: Vec<rusqlite::types::Value> =
        vec![rusqlite::types::Value::from(min_loc as i64)];
    params.extend(scope_params);
    let total = complexity_total(conn, min_loc, path, &gen_filter, &params)?;
    if total == 0 {
        return Err(AwError::NoResultsCaveated {
            caveat: COMPLEXITY_CAVEAT,
            detail: "0 symbols match the thresholds".to_string(),
        });
    }
    let rows = complexity_rows(conn, path, limit, &gen_filter, &params)?;
    let shown = rows.len().min(limit);
    let displayed: BTreeSet<String> = rows.iter().take(shown).map(|(p, _)| p.clone()).collect();
    let (header, stale) = staleness_header(ctx, &displayed);
    let data_lines: Vec<String> = rows
        .iter()
        .take(shown)
        .map(|(_, line)| line.clone())
        .collect();
    let mut out = vec![COMPLEXITY_CAVEAT.to_string()];
    if let Some(header) = header {
        out.push(header);
    }
    out.extend(data_lines.iter().cloned());
    let cap = finish_capped(&mut out, shown, total as usize, stale, "");
    Ok((out, cap))
}

/// Cap footer, journal line, and the stderr cap notice shared by the report
/// queries. Returns the notice so callers route it through the stderr half of
/// `LineQuery` (the single cap-notice channel) instead of `eprintln!`.
fn finish_capped(
    out: &mut Vec<String>,
    shown: usize,
    total: usize,
    stale: bool,
    unit: &str,
) -> String {
    if let Some(line) = crate::query::cap_line(shown, total, unit) {
        out.push(line);
    }
    crate::journal::record_result(crate::journal::OutcomeCounts::of(shown, total), stale, 0);
    cap_stderr(shown, total)
}

/// Clamp `limit`, validate the optional path, and build the scope clause with
/// its bound parameters.
fn scoped_query(
    path: Option<&str>,
    limit: usize,
    alias: &str,
    column: &str,
    param: usize,
) -> Result<(usize, String, Vec<rusqlite::types::Value>)> {
    let limit = clamp_limit(limit)?;
    if let Some(p) = path {
        validate_path_scope(p, false)?;
    }
    let (scope, params) = scoped_params(alias, column, path, param);
    Ok((limit, scope, params))
}

/// Complexity total: the build-time counter for the default filters, else the
/// filtered COUNT.
fn complexity_total(
    conn: &Connection,
    min_loc: usize,
    path: Option<&str>,
    gen_filter: &str,
    params: &[rusqlite::types::Value],
) -> Result<i64> {
    if min_loc == 0 && path.is_none() {
        return Ok(db::meta_get(conn, db::meta_keys::CALLABLE_COUNT)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0));
    }
    Ok(conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM symbols s
             WHERE s.kind IN ('{}','{}')
               AND s.effective_loc > 0 AND s.effective_loc >= ?1 {gen_filter}",
            SymbolKind::Function.as_str(),
            SymbolKind::Method.as_str()
        ),
        rusqlite::params_from_iter(params.iter()),
        |r| r.get(0),
    )?)
}

/// Bounded per-symbol rows, formatted as `path:line loc=N qualname`.
fn complexity_rows(
    conn: &Connection,
    path: Option<&str>,
    limit: usize,
    gen_filter: &str,
    params: &[rusqlite::types::Value],
) -> Result<Vec<(String, String)>> {
    // A path scope is most selective through symbols_file_path; forcing the
    // complexity index would defeat that plan.
    let index_hint = if path.is_some() {
        ""
    } else {
        " INDEXED BY symbols_complexity_idx"
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT s.file_path, s.line_start, s.effective_loc, s.qualname
         FROM symbols s{index_hint}
         WHERE s.kind IN ('{}','{}')
           AND s.effective_loc > 0 AND s.effective_loc >= ?1 {gen_filter}
         ORDER BY s.effective_loc DESC, s.qualname, s.file_path, s.line_start
         LIMIT {limit}",
        SymbolKind::Function.as_str(),
        SymbolKind::Method.as_str()
    ))?;
    let rows: Vec<(String, String)> = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                format!(
                    "{}:{} loc={} {}",
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?
                ),
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

/// `aw resolve --stdin` (Amendment 19): batch name resolution with per-name
/// found/ambiguous/missing lines and a CLI exit code (0/1/2).
pub fn resolve_names(conn: &Connection, names: &[String]) -> Result<(Vec<String>, i32)> {
    let mut out = Vec::new();
    let mut exit = 0;
    for name in names {
        let name = name.trim();
        if name.is_empty() || name.starts_with('#') {
            continue;
        }
        let (ids, cands) = target_ids(conn, name)?;
        if ids.is_empty() {
            out.push(format!("missing {name}"));
            exit = exit.max(1);
        } else if ids.len() > 1 {
            for (path, line, qualname) in cands.iter() {
                out.push(format!("ambiguous {name} {path}:{line} {qualname}"));
            }
            if ids.len() > cands.len() {
                out.push(format!(
                    "showing {} of {} candidates",
                    cands.len(),
                    ids.len()
                ));
            }
            exit = 2;
        } else if let Some((path, line, qualname)) = cands.first() {
            out.push(format!("found {name} {path}:{line} {qualname}"));
        }
    }
    Ok((out, exit))
}

/// `aw files` (Amendment 19): indexed file inventory, build-time values.
pub fn files(conn: &Connection, path: Option<&str>, limit: usize) -> Result<LineQuery> {
    let (limit, scope, params) = scoped_query(path, limit, "f", "path", 1)?;
    let total: usize = conn.query_row(
        &format!("SELECT COUNT(*) FROM files f WHERE 1=1 {scope}"),
        rusqlite::params_from_iter(params.iter()),
        |r| r.get::<_, i64>(0),
    )? as usize;
    let sql = format!(
        "SELECT f.path, COUNT(s.id), f.size, f.generated
         FROM files f LEFT JOIN symbols s ON s.file_id = f.id
         WHERE 1=1 {scope}
         GROUP BY f.id ORDER BY f.path LIMIT {limit}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<(String, i64, i64, i64)> = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let files_caveat = "aw: files — build-time inventory; files created after the build are absent";
    if rows.is_empty() {
        return Err(AwError::NoResultsCaveated {
            caveat: files_caveat,
            detail: match path.map(normalize_scope) {
                Some(s) if !s.is_empty() => format!("0 indexed files in '{s}'"),
                _ => "0 indexed files".to_string(),
            },
        });
    }
    let shown = rows.len();
    let mut out = vec![files_caveat.to_string()];
    out.extend(rows.into_iter().map(|(path, syms, bytes, gen)| {
        format!("{path} symbols={syms} bytes={bytes} generated={gen}")
    }));
    let cap = finish_capped(&mut out, shown, total, false, "");
    Ok((out, cap))
}

/// Unresolved edge names with locations (Amendment 19). Report shape: header,
/// then one `path:line kind name` row per unresolved site.
pub fn unresolved_names(ctx: &QueryCtx<'_>, path: Option<&str>, limit: usize) -> Result<LineQuery> {
    let QueryCtx { conn, .. } = *ctx;
    let (limit, scope, scope_params) = scoped_query(path, limit, "f", "path", 1)?;
    let Some((total, files, unlocated)) = unresolved_counts(conn, &scope, &scope_params)? else {
        return Err(AwError::NoResults(
            "0 unresolved names (external/builtin calls are not counted)".to_string(),
        ));
    };
    let rows = unresolved_rows(conn, &scope, &scope_params, limit)?;
    let shown = rows.len().min(limit);
    let displayed: BTreeSet<String> = rows
        .iter()
        .take(shown)
        .map(|(p, _, _, _)| p.clone())
        .collect();
    let (header, stale) = staleness_header(ctx, &displayed);
    if unlocated > 0 {
        eprintln!("[aw] {unlocated} unresolved row(s) have site_line=0 and are omitted");
    }
    let mut out = vec![format!(
        "aw: unresolved={total} rows in {files} files (call/import/inherit)"
    )];
    if let Some(header) = header {
        out.push(header);
    }
    out.extend(
        rows.into_iter()
            .take(shown)
            .map(|(path, line, kind, name)| format!("{path}:{line} {kind} {name}")),
    );
    let cap = finish_capped(&mut out, shown, total as usize, stale, "rows");
    Ok((out, cap))
}

/// Total, distinct files, and unlocated rows. `None` when the scoped set is
/// empty, so the caller can return the no-results error without two extra
/// count queries.
fn unresolved_counts(
    conn: &Connection,
    scope: &str,
    scope_params: &[rusqlite::types::Value],
) -> Result<Option<(i64, i64, i64)>> {
    let scalar = |expr: &str, extra: &str| -> Result<i64> {
        Ok(conn.query_row(
            &format!(
                "SELECT {expr} FROM unresolved_calls u JOIN files f ON f.id = u.file_id WHERE 1=1 {extra} {scope}"
            ),
            rusqlite::params_from_iter(scope_params.iter()),
            |r| r.get::<_, i64>(0),
        )?)
    };
    let total = scalar("COUNT(*)", "AND u.site_line > 0")?;
    if total == 0 {
        return Ok(None);
    }
    let files = scalar("COUNT(DISTINCT u.file_id)", "AND u.site_line > 0")?;
    let unlocated = scalar("COUNT(*)", "AND u.site_line = 0")?;
    Ok(Some((total, files, unlocated)))
}

fn unresolved_rows(
    conn: &Connection,
    scope: &str,
    scope_params: &[rusqlite::types::Value],
    limit: usize,
) -> Result<Vec<(String, i64, String, String)>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT f.path, u.site_line, u.kind, u.dst_name
         FROM unresolved_calls u JOIN files f ON f.id = u.file_id
         WHERE u.site_line > 0 {scope}
         ORDER BY f.path, u.site_line, u.kind, u.dst_name LIMIT {limit}"
    ))?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(scope_params.iter()), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

/// Unresolved-callee report (Amendment 5): files with counts, descending.
/// Report shape — cap, no staleness; header carries the index total.
pub fn unresolved(conn: &Connection, path: Option<&str>, limit: usize) -> Result<LineQuery> {
    let (limit, scope, scope_params) = scoped_query(path, limit, "f", "path", 1)?;
    let rows = unresolved_file_rows(conn, &scope, &scope_params)?;
    let total: i64 = rows.iter().map(|(_, n)| *n).sum();
    if total == 0 {
        return Err(AwError::NoResults(
            "0 unresolved in indexed scope (external/builtin calls are not counted)".to_string(),
        ));
    }
    let n_files = rows.len();
    let shown = n_files.min(limit);
    let (names, internal, distinct) = unresolved_name_view(conn, &scope, &scope_params)?;
    let mut out = vec![format!(
        "aw: unresolved={total} callees in {n_files} files, {distinct} distinct names (internal={internal})"
    )];
    if !names.is_empty() {
        out.push("top unresolved names:".to_string());
        out.extend(names.into_iter().map(|(name, n, internal)| {
            if internal {
                format!("  {name} n={n} [internal]")
            } else {
                format!("  {name} n={n}")
            }
        }));
    }
    out.extend(
        rows.into_iter()
            .take(shown)
            .map(|(path, n)| format!("{path} unresolved={n}")),
    );
    let cap = finish_capped(&mut out, shown, n_files, false, "");
    Ok((out, cap))
}

/// Files with unresolved callees, descending by count.
fn unresolved_file_rows(
    conn: &Connection,
    scope: &str,
    params: &[rusqlite::types::Value],
) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT f.path, f.unresolved FROM files f WHERE f.unresolved > 0 {scope}
         ORDER BY f.unresolved DESC, f.path"
    ))?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// `(name, count, is_internal)` rows plus the internal-name and distinct-name
/// totals.
type UnresolvedNames = (Vec<(String, i64, bool)>, i64, i64);

/// Name-level view (R7 L4): file counts alone say where resolution failed,
/// not what it missed. `[internal]` marks names that exist in the index — an
/// actionable resolver gap — versus external/builtin calls.
fn unresolved_name_view(
    conn: &Connection,
    scope: &str,
    params: &[rusqlite::types::Value],
) -> Result<UnresolvedNames> {
    let mut name_stmt = conn.prepare(&format!(
        "SELECT uc.dst_name, COUNT(*) AS n,
                EXISTS(SELECT 1 FROM symbols s
                       WHERE s.name = uc.dst_name OR s.qualname = uc.dst_name)
         FROM unresolved_calls uc JOIN files f ON f.id = uc.file_id
         WHERE uc.kind = 'call' {scope}
         GROUP BY uc.dst_name
         ORDER BY n DESC, uc.dst_name
         LIMIT 10"
    ))?;
    let names = name_stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)? != 0,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let internal: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM (
            SELECT uc.dst_name FROM unresolved_calls uc JOIN files f ON f.id = uc.file_id
            WHERE uc.kind = 'call'
              AND EXISTS(SELECT 1 FROM symbols s
                         WHERE s.name = uc.dst_name OR s.qualname = uc.dst_name)
              {scope}
            GROUP BY uc.dst_name)"
        ),
        rusqlite::params_from_iter(params.iter()),
        |r| r.get(0),
    )?;
    let distinct: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT uc.dst_name) FROM unresolved_calls uc
             JOIN files f ON f.id = uc.file_id WHERE uc.kind = 'call' {scope}"
        ),
        rusqlite::params_from_iter(params.iter()),
        |r| r.get(0),
    )?;
    Ok((names, internal, distinct))
}

/// Display knobs for `aw symbols` (mirrors `ImpactOpts`): `--outline` adds the
/// stored one-line signature, `--metrics` the effective LOC.
pub struct SymbolsOpts {
    pub outline: bool,
    pub metrics: bool,
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
    let SymbolsOpts { outline, metrics } = opts;
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
            return Err(AwError::not_indexed(file));
        }
        return symbols_directory(ctx, &rel, limit, SymbolsOpts { outline, metrics });
    };
    let rows = file_symbol_rows(conn, &rel, outline)?;
    if rows.is_empty() {
        return Err(AwError::not_indexed(file));
    }
    let files = BTreeSet::from([rel.clone()]);
    let (header, stale) = staleness_header(ctx, &files);
    let n_symbols = rows.len();
    let shown = n_symbols.min(limit);
    let data_lines: Vec<String> = rows
        .into_iter()
        .take(shown)
        .map(|row| symbol_line(&rel, &row, metrics))
        .collect();
    let mut out = Vec::new();
    if let Some(header) = header {
        out.push(header);
    }
    out.extend(data_lines);
    let cap = finish_capped(&mut out, shown, n_symbols, stale, "");
    Ok((out, cap))
}

fn file_symbol_rows(conn: &Connection, rel: &str, outline: bool) -> Result<Vec<SymbolRow>> {
    let signature_col = if outline { "s.signature" } else { "''" };
    let sql = format!(
        "SELECT s.kind, s.qualname, s.line_start, {signature_col}, s.effective_loc
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
                loc: r.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// One `path:line kind qualname` row; `--outline` appends the signature and
/// `--metrics` the effective LOC.
fn symbol_line(path: &str, row: &SymbolRow, metrics: bool) -> String {
    let mut text = format!("{path}:{} {} {}", row.line, row.kind, row.qualname);
    if !row.signature.is_empty() {
        text.push(' ');
        text.push_str(&row.signature);
    }
    if metrics {
        text.push_str(&format!(" loc={}", row.loc));
    }
    text
}

/// Directory mode for `aw symbols <dir>` (Amendment 19): indexed prefix only,
/// grouped per file, cap counts symbol data lines.
fn symbols_directory(
    ctx: &QueryCtx<'_>,
    dir: &str,
    limit: usize,
    SymbolsOpts { outline, metrics }: SymbolsOpts,
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
        return Err(AwError::NoResults(format!(
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
        "aw: dir {display} — {n_files} files, {total} symbols"
    ));
    let mut last_path = "";
    for (path, row) in &rows {
        if path != last_path {
            out.push(path.clone());
            last_path = path;
        }
        out.push(symbol_line(path, row, metrics));
    }
    let shown = rows.len();
    let cap = finish_capped(&mut out, shown, total as usize, stale, "");
    Ok((out, cap))
}

fn dir_symbol_rows(
    conn: &Connection,
    prefix: &str,
    outline: bool,
    limit: usize,
) -> Result<Vec<(String, SymbolRow)>> {
    let sql = format!(
        "SELECT f.path, s.kind, s.qualname, s.line_start, s.signature, s.effective_loc
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE substr(f.path, 1, length(?1)) = ?1
         ORDER BY f.path, s.line_start, s.qualname LIMIT {limit}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([prefix], |r| {
            Ok((
                r.get::<_, String>(0)?,
                SymbolRow {
                    kind: r.get(1)?,
                    qualname: r.get(2)?,
                    line: r.get(3)?,
                    signature: if outline { r.get(4)? } else { String::new() },
                    loc: r.get(5)?,
                },
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

/// Cardinality of the exact ∪ FTS result set by symbol identity. The capped
/// page overlap cannot stand in for the full overlap.
fn search_union_total(conn: &Connection, exact_name: &str, fts_expr: &str) -> Result<usize> {
    let n: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM (
             SELECT s.id FROM symbols s WHERE {EXACT_IDENTITY_PREDICATE}
             UNION
             SELECT s.id FROM symbols_fts ft JOIN symbols s ON s.id = ft.rowid
              WHERE symbols_fts MATCH ?2
         )"
        ),
        rusqlite::params![exact_name, fts_expr],
        |r| r.get(0),
    )?;
    Ok(n as usize)
}

/// Bound the OR expression (R6f Important 7): FTS5 cost is quadratic in the
/// number of prefix terms under OR.
const SEARCH_MAX_TERMS: usize = 32;

/// Build the OR prefix expression and apply the qualified-identity fast path.
fn search_expr(conn: &Connection, terms: &str) -> Result<(Vec<String>, String)> {
    let mut toks: Vec<String> = crate::extract::split_ident(terms)
        .split_whitespace()
        .map(|t| t.to_string())
        .collect();
    if toks.len() > SEARCH_MAX_TERMS {
        toks.truncate(SEARCH_MAX_TERMS);
        eprintln!("[aw] search terms capped at {SEARCH_MAX_TERMS}");
    }
    let mut expr = toks
        .iter()
        .map(|tok| format!("\"{}\"*", tok.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ");
    if expr.is_empty() {
        return Err(AwError::Usage("empty search terms".to_string()));
    }
    // Identity fast path (R7): a qualified input (`.`, `/`, `::`) matched as
    // an adjacent-token phrase preserves exact identity and avoids ranking
    // the whole corpus (1M-symbol: ~500 ms -> <1 ms). Fall back to OR when
    // the phrase has no hits, so recall does not regress.
    if terms.contains('.') || terms.contains('/') || terms.contains("::") {
        let phrase = format!("\"{}\"", toks.join(" ").replace('"', ""));
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM symbols_fts WHERE symbols_fts MATCH ?1",
            [&phrase],
            |r| r.get(0),
        )?;
        if n > 0 {
            expr = phrase;
        }
    }
    Ok((toks, expr))
}

/// Columns of a search page row, in `search_pointer` order: file, line,
/// qualname, stored one-line signature. Both search pages interpolate it so
/// their column order cannot drift off the shared mapper.
const SEARCH_PAGE_COLUMNS: &str = "f.path, s.line_start, s.qualname, s.signature";

/// Map a search row to the shared pointer shape. Amendment 14b: hits carry
/// the stored one-line signature after the qualname; an empty signature keeps
/// the 12b byte shape.
fn search_pointer(r: &rusqlite::Row<'_>) -> rusqlite::Result<Pointer> {
    let qualname: String = r.get(2)?;
    let signature: String = r.get(3)?;
    Ok(Pointer {
        file: r.get::<_, String>(0)?,
        line: r.get::<_, i64>(1)?,
        tag: "",
        detail: if signature.is_empty() {
            qualname
        } else {
            format!("{qualname} {signature}")
        },
    })
}

/// FTS page and total.
fn search_fts_page(conn: &Connection, expr: &str, limit: usize) -> Result<(Vec<Pointer>, usize)> {
    let total: usize = conn.query_row(
        "SELECT COUNT(*) FROM symbols_fts ft
         JOIN symbols s ON s.id = ft.rowid JOIN files f ON f.id = s.file_id
         WHERE symbols_fts MATCH ?1",
        [expr],
        |r| r.get::<_, i64>(0),
    )? as usize;
    let mut stmt = conn.prepare(&format!(
        "SELECT {SEARCH_PAGE_COLUMNS}
         FROM symbols_fts ft
         JOIN symbols s ON s.id = ft.rowid
         JOIN files f ON f.id = s.file_id
         WHERE symbols_fts MATCH ?1
         -- Tie-break on stable columns: rowids differ between fresh and
         -- carried builds, so bm25 alone is not deterministic (R6d A9).
         ORDER BY bm25(symbols_fts), s.qualname, f.path, s.line_start LIMIT {limit}"
    ))?;
    let rows = stmt.query_map([expr], search_pointer)?;
    let ptrs = rows.collect::<std::result::Result<_, _>>()?;
    Ok((ptrs, total))
}

/// Exact-identity page and total, empty when the exact set has no rows.
fn search_exact_page(
    conn: &Connection,
    exact_name: &str,
    limit: usize,
) -> Result<(Vec<Pointer>, usize)> {
    let exact_total: usize = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE {EXACT_IDENTITY_PREDICATE}"
        ),
        [exact_name],
        |r| r.get::<_, i64>(0),
    )? as usize;
    if exact_total == 0 {
        return Ok((Vec::new(), 0));
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT {SEARCH_PAGE_COLUMNS}
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE {EXACT_IDENTITY_PREDICATE}
         ORDER BY f.path, s.line_start, s.qualname LIMIT {limit}"
    ))?;
    let ptrs = stmt
        .query_map([exact_name], search_pointer)?
        .collect::<std::result::Result<_, _>>()?;
    Ok((ptrs, exact_total))
}

/// Exact identities first, then the deduplicated FTS page.
fn merge_search_pointers(exact: Vec<Pointer>, fts: Vec<Pointer>) -> Vec<Pointer> {
    let mut seen: std::collections::HashSet<(String, i64, String)> =
        std::collections::HashSet::new();
    let mut merged = Vec::with_capacity(exact.len() + fts.len());
    for p in exact {
        seen.insert((p.file.clone(), p.line, p.detail.clone()));
        merged.push(p);
    }
    for p in fts {
        if seen.insert((p.file.clone(), p.line, p.detail.clone())) {
            merged.push(p);
        }
    }
    merged
}

/// Symbol-level search (Amendment 12b): FTS5 `bm25()` over split identifier
/// tokens. Hits are symbol identities — the tier field position stays empty
/// (only call-edge pointers carry ` [ambiguous]`). Reuses `collect_output`
/// for cap and post-cap staleness.
pub fn search(ctx: &QueryCtx<'_>, terms: &str, limit: usize) -> Result<LineQuery> {
    let QueryCtx { conn, .. } = *ctx;
    let limit = clamp_limit(limit)?;
    let (toks, expr) = search_expr(conn, terms)?;
    let (fts_ptrs, total) = search_fts_page(conn, &expr, limit)?;
    let (pointers, total) = if toks.len() == 1 {
        let exact_name = toks.join(" ");
        let (exact_ptrs, exact_total) = search_exact_page(conn, &exact_name, limit)?;
        if exact_total > 0 {
            // The capped page overlap is not the full overlap: count the
            // union of exact and FTS identities.
            (
                merge_search_pointers(exact_ptrs, fts_ptrs),
                search_union_total(conn, &exact_name, &expr)?,
            )
        } else {
            (fts_ptrs, total)
        }
    } else {
        (fts_ptrs, total)
    };
    let out = collect_output(
        ctx,
        pointers.into_iter().map(Ok::<_, rusqlite::Error>),
        limit,
        &format!("0 symbols matched '{terms}' — try fewer or broader terms"),
        total,
    )?;
    search_lines(out)
}
