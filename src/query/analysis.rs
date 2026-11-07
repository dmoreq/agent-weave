//! Inventory and analysis queries: dead candidates, complexity, batch resolve,
//! files, unresolved sites, symbols, search, and duplicate candidates.

use super::*;
use crate::extract::EdgeSiteKind;

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

/// The scoped unresolved-set bundle the unresolved helpers share: connection,
/// `WHERE` clause, and its bound parameters travel as one value (the
/// ImpactCtx pattern) instead of a loose `(conn, scope, params)` triple.
struct UnresolvedScope<'a> {
    conn: &'a Connection,
    clause: String,
    params: Vec<rusqlite::types::Value>,
}

/// Unresolved edge names with locations (Amendment 19). Report shape: header,
/// then one `path:line kind name` row per unresolved site.
pub fn unresolved_names(ctx: &QueryCtx<'_>, path: Option<&str>, limit: usize) -> Result<LineQuery> {
    let QueryCtx { conn, .. } = *ctx;
    let (limit, clause, params) = scoped_query(path, limit, "f", "path", 1)?;
    let scope = UnresolvedScope {
        conn,
        clause,
        params,
    };
    let Some((total, files, unlocated)) = unresolved_counts(&scope)? else {
        return Err(AwError::NoResults(
            "0 unresolved names (external/builtin calls are not counted)".to_string(),
        ));
    };
    let rows = unresolved_rows(&scope, limit)?;
    let shown = rows.len().min(limit);
    let displayed: BTreeSet<String> = rows
        .iter()
        .take(shown)
        .map(|row| row.path.clone())
        .collect();
    let (header, stale) = staleness_header(ctx, &displayed);
    let mut out = vec![format!(
        "aw: unresolved={total} rows in {files} files (call/import/inherit)"
    )];
    if let Some(header) = header {
        out.push(header);
    }
    out.extend(rows.into_iter().take(shown).map(|row| {
        let UnresolvedRow {
            path,
            line,
            kind,
            name,
        } = row;
        format!("{path}:{line} {kind} {name}")
    }));
    let cap = finish_capped(&mut out, shown, total as usize, stale, "rows");
    let cap = if unlocated > 0 {
        format!("[aw] {unlocated} unresolved row(s) have site_line=0 and are omitted\n{cap}")
    } else {
        cap
    };
    Ok(LineQuery {
        lines: out,
        stderr: cap,
        shown,
    })
}

/// Total, distinct files, and unlocated rows. `None` when the scoped set is
/// empty, so the caller can return the no-results error without two extra
/// count queries.
fn unresolved_counts(scope: &UnresolvedScope<'_>) -> Result<Option<(i64, i64, i64)>> {
    let scalar = |expr: &str, extra: &str| -> Result<i64> {
        Ok(scope.conn.query_row(
            &format!(
                "SELECT {expr} FROM unresolved_calls u JOIN files f ON f.id = u.file_id WHERE 1=1 {extra} {}",
                scope.clause
            ),
            rusqlite::params_from_iter(scope.params.iter()),
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

/// One unresolved-site row: file path, site line, site kind, and callee name.
/// Named so the renderer and the SELECT cannot drift apart positionally.
struct UnresolvedRow {
    path: String,
    line: i64,
    kind: String,
    name: String,
}

fn unresolved_rows(scope: &UnresolvedScope<'_>, limit: usize) -> Result<Vec<UnresolvedRow>> {
    // The display limit is the trailing bind parameter after the scope params.
    let limit_param = scope.params.len() + 1;
    let mut params = scope.params.clone();
    params.push(rusqlite::types::Value::from(limit as i64));
    let mut stmt = scope.conn.prepare(&format!(
        "SELECT f.path, u.site_line, u.kind, u.dst_name
         FROM unresolved_calls u JOIN files f ON f.id = u.file_id
         WHERE u.site_line > 0 {}
         ORDER BY f.path, u.site_line, u.kind, u.dst_name LIMIT ?{limit_param}",
        scope.clause
    ))?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok(UnresolvedRow {
                path: r.get(0)?,
                line: r.get(1)?,
                kind: r.get(2)?,
                name: r.get(3)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Unresolved-callee report (Amendment 5): files with counts, descending.
/// Report shape — cap, no staleness; header carries the index total.
pub fn unresolved(conn: &Connection, path: Option<&str>, limit: usize) -> Result<LineQuery> {
    let (limit, clause, params) = scoped_query(path, limit, "f", "path", 1)?;
    let scope = UnresolvedScope {
        conn,
        clause,
        params,
    };
    let rows = unresolved_file_rows(&scope)?;
    let total: i64 = rows.iter().map(|(_, n)| *n).sum();
    if total == 0 {
        return Err(AwError::NoResults(
            "0 unresolved in indexed scope (external/builtin calls are not counted)".to_string(),
        ));
    }
    let n_files = rows.len();
    let shown = n_files.min(limit);
    let (names, internal, distinct) = unresolved_name_view(&scope)?;
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
    Ok(LineQuery {
        lines: out,
        stderr: cap,
        shown,
    })
}

/// Files with unresolved callees, descending by count.
fn unresolved_file_rows(scope: &UnresolvedScope<'_>) -> Result<Vec<(String, i64)>> {
    let mut stmt = scope.conn.prepare(&format!(
        "SELECT f.path, f.unresolved FROM files f WHERE f.unresolved > 0 {}
         ORDER BY f.unresolved DESC, f.path",
        scope.clause
    ))?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(scope.params.iter()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// `(name, count, is_internal)` rows plus the internal-name and distinct-name
/// totals.
type UnresolvedNames = (Vec<(String, i64, bool)>, i64, i64);

/// The `unresolved_calls` scan shape shared by all three name-view
/// statements: the `files` JOIN plus the call-kind filter. Each statement
/// appends its own tail (extra predicates, the scope clause) so the JOIN
/// shape cannot drift between them.
fn unresolved_from_where() -> String {
    format!(
        "FROM unresolved_calls uc JOIN files f ON f.id = uc.file_id
         WHERE uc.kind = '{}'",
        EdgeSiteKind::Call.as_str()
    )
}

/// Name-level view (R7 L4): file counts alone say where resolution failed,
/// not what it missed. `[internal]` marks names that exist in the index — an
/// actionable resolver gap — versus external/builtin calls.
fn unresolved_name_view(scope: &UnresolvedScope<'_>) -> Result<UnresolvedNames> {
    let mut name_stmt = scope.conn.prepare(&format!(
        "SELECT uc.dst_name, COUNT(*) AS n,
                EXISTS(SELECT 1 FROM symbols s
                       WHERE s.name = uc.dst_name OR s.qualname = uc.dst_name)
         {} {}
         GROUP BY uc.dst_name
         ORDER BY n DESC, uc.dst_name
         LIMIT 10",
        unresolved_from_where(),
        scope.clause
    ))?;
    let names = name_stmt
        .query_map(rusqlite::params_from_iter(scope.params.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)? != 0,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let internal: i64 = scope.conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM (
            SELECT uc.dst_name {}
              AND EXISTS(SELECT 1 FROM symbols s
                         WHERE s.name = uc.dst_name OR s.qualname = uc.dst_name)
              {}
            GROUP BY uc.dst_name)",
            unresolved_from_where(),
            scope.clause
        ),
        rusqlite::params_from_iter(scope.params.iter()),
        |r| r.get(0),
    )?;
    let distinct: i64 = scope.conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT uc.dst_name) {} {}",
            unresolved_from_where(),
            scope.clause
        ),
        rusqlite::params_from_iter(scope.params.iter()),
        |r| r.get(0),
    )?;
    Ok((names, internal, distinct))
}

/// Display knobs for `aw symbols` (mirrors `ImpactOpts`): `--outline` adds the
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
            return Err(AwError::not_indexed(file));
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
        return Err(AwError::not_indexed(file));
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
    Ok(LineQuery {
        lines: out,
        stderr: cap,
        shown,
    })
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
    LineQuery {
        lines: out,
        stderr: cap,
        shown,
    }
}

/// Directory mode for `aw symbols <dir>` (Amendment 19): indexed prefix only,
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
/// Returns `(tokens, expr, terms_cap)`; `terms_cap` is `Some` when the input
/// was truncated to `SEARCH_MAX_TERMS`, and the caller routes it through the
/// stderr half of `LineQuery` (the single cap-notice channel).
fn search_expr(conn: &Connection, terms: &str) -> Result<(Vec<String>, String, Option<String>)> {
    let mut toks: Vec<String> = crate::extract::split_ident(terms)
        .split_whitespace()
        .map(|t| t.to_string())
        .collect();
    let mut terms_cap = None;
    if toks.len() > SEARCH_MAX_TERMS {
        toks.truncate(SEARCH_MAX_TERMS);
        terms_cap = Some(format!("[aw] search terms capped at {SEARCH_MAX_TERMS}"));
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
    Ok((toks, expr, terms_cap))
}

/// Columns of a search page row, in `search_pointer` order: file, line,
/// kind, qualname, stored one-line signature, callers count. The callers
/// subquery is `db::CALLER_COUNT_SQL` so the "number of callers" definition
/// has one home (search ranking and reuse calibration share it).
fn search_page_columns() -> String {
    format!(
        "f.path, s.line_start, s.kind, s.qualname, s.signature, {} AS callers",
        db::CALLER_COUNT_SQL
    )
}

/// Map a search row to the interface card pointer shape:
/// `path:line kind qualname [signature] [callers=N]`.
fn search_pointer(r: &rusqlite::Row<'_>) -> rusqlite::Result<Pointer> {
    let kind: String = r.get(2)?;
    let qualname: String = r.get(3)?;
    let signature: String = r.get(4)?;
    let callers: i64 = r.get(5)?;
    let detail = if signature.is_empty() {
        format!("{kind} {qualname} [callers={callers}]")
    } else {
        format!("{kind} {qualname} {signature} [callers={callers}]")
    };
    Ok(Pointer {
        file: r.get::<_, String>(0)?,
        line: r.get::<_, i64>(1)?,
        tag: "",
        detail,
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
    let fetch_limit = (limit * 2).clamp(50, 1000);
    let mut stmt = conn.prepare(&format!(
        "SELECT {}, bm25(symbols_fts) AS bm25_score
         FROM symbols_fts ft
         JOIN symbols s ON s.id = ft.rowid
         JOIN files f ON f.id = s.file_id
         WHERE symbols_fts MATCH ?1
         ORDER BY bm25(symbols_fts), s.qualname, f.path, s.line_start LIMIT ?2",
        search_page_columns()
    ))?;
    let mut candidates = stmt
        .query_map(rusqlite::params![expr, fetch_limit as i64], |r| {
            let pointer = search_pointer(r)?;
            let bm25_score: f64 = r.get(6)?;
            let callers: i64 = r.get(5)?;
            let score = bm25_score - 0.3 * (1.0 + callers as f64).ln();
            let path: String = r.get(0)?;
            let line: i64 = r.get(1)?;
            let qualname: String = r.get(3)?;
            Ok((pointer, score, qualname, path, line))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    candidates.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
            .then_with(|| a.4.cmp(&b.4))
    });
    candidates.truncate(limit);
    let ptrs = candidates.into_iter().map(|c| c.0).collect();
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
        "SELECT {}
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE {EXACT_IDENTITY_PREDICATE}
         ORDER BY {} DESC, f.path, s.line_start, s.qualname LIMIT ?2",
        search_page_columns(),
        db::CALLER_COUNT_SQL
    ))?;
    let ptrs = stmt
        .query_map(rusqlite::params![exact_name, limit as i64], search_pointer)?
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
    let (toks, expr, terms_cap) = search_expr(conn, terms)?;
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
    if total == 0 {
        return Err(AwError::NoResults(format!(
            "0 symbols matched '{terms}' — try fewer or broader terms"
        )));
    }
    let out = collect_output(
        ctx,
        pointers.into_iter().map(Ok::<_, rusqlite::Error>),
        limit,
        total,
    )?;
    let mut result = search_lines(out)?;
    if let Some(notice) = terms_cap {
        result.stderr = format!("{notice}\n{}", result.stderr);
    }
    Ok(result)
}
