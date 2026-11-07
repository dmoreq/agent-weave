//! Inventory and analysis queries: dead candidates, complexity, batch resolve,
//! files, unresolved sites, symbols, search, and duplicate candidates.

use super::*;

pub fn dead(
    conn: &Connection,
    root: &Path,
    limit: usize,
    exclude_tests: bool,
    path: Option<&str>,
) -> Result<QueryOutput> {
    let limit = clamp_limit(limit)?;
    if let Some(p) = path {
        validate_path_scope(p, false)?;
    }
    // AMBIGUOUS suppression is per-symbol (R7): a bare-name ambiguous call
    // suppresses only same-named symbols in the caller's own file; a symbol
    // elsewhere with the same bare name is not presumed referenced.
    // One materialized referenced-id set, shared by COUNT and the page query.
    // The previous correlated NOT EXISTS re-scanned edges per candidate
    // symbol (R7 review).
    let referenced = "WITH trait_parents(qualname) AS MATERIALIZED (
           SELECT DISTINCT qualname FROM symbols
            WHERE kind IN ('trait', 'interface')
       ),
       referenced_ids(id) AS MATERIALIZED (
           SELECT dst_id FROM edges
            WHERE kind IN ('calls', 'imports', 'inherits', 'type_ref')
              AND dst_id IS NOT NULL
           UNION
           SELECT s2.id FROM edges e
             JOIN symbols s2 ON s2.qualname = e.dst_name
            WHERE e.kind IN ('calls', 'imports', 'inherits', 'type_ref')
              AND e.confidence = 'AMBIGUOUS'
           UNION
           SELECT s2.id FROM edges e
             JOIN symbols src ON src.id = e.src_id
             JOIN symbols s2 ON s2.name = e.dst_name AND s2.file_id = src.file_id
            WHERE e.kind IN ('calls', 'imports', 'inherits', 'type_ref')
              AND e.confidence = 'AMBIGUOUS'
       )";
    let mut predicate = "s.kind IN ('function', 'method', 'class', 'trait', 'interface')
           AND s.id NOT IN (SELECT id FROM referenced_ids)
           AND NOT EXISTS (
               SELECT 1 FROM trait_parents tp
                WHERE s.qualname >= tp.qualname || '.'
                  AND s.qualname < tp.qualname || '/'
           )"
    .to_string();
    if exclude_tests {
        // Schema v8: `symbols.is_test` classifies once at extraction (test
        // paths, test-shaped names/files, Rust test attributes, TS describe
        // scope), so one column check replaces the v7 LIKE/GLOB list.
        predicate.push_str(" AND s.is_test = 0");
    }
    let mut params: Vec<String> = Vec::new();
    if let Some(path) = path {
        predicate.push_str(" AND (f.path = ?1 OR substr(f.path, 1, length(?1) + 1) = ?1 || '/')");
        params.push(
            path.trim_start_matches("./")
                .trim_end_matches('/')
                .to_string(),
        );
    }
    let predicate = predicate.as_str();
    let total: usize = conn.query_row(
        &format!(
            "{referenced} SELECT COUNT(*) FROM symbols s JOIN files f ON f.id = s.file_id WHERE {predicate}"
        ),
        rusqlite::params_from_iter(params.iter()),
        |r| r.get::<_, i64>(0),
    )? as usize;
    let sql = format!(
        "{referenced} SELECT s.qualname, f.path, s.line_start
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE {predicate}
         ORDER BY f.path, s.line_start LIMIT {limit}"
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
    let mut out = collect_output(
        conn,
        root,
        rows,
        limit,
        "0 dead-code candidates found",
        Some(total),
    )?;
    out.header = Some(match out.header.take() {
        Some(stale) => format!("{DEAD_CAVEAT}\n{stale}"),
        None => DEAD_CAVEAT.to_string(),
    });
    Ok(out)
}

/// `aw complexity` (Amendment 19, v9 shape): per-symbol effective-LOC ranking.
pub fn complexity(
    conn: &Connection,
    root: &Path,
    min_loc: usize,
    path: Option<&str>,
    limit: usize,
) -> Result<Vec<String>> {
    let limit = clamp_limit(limit)?;
    if let Some(p) = path {
        validate_path_scope(p, false)?;
    }
    let (gen_filter, scope_value) = file_path_scope_sql("s", path, 2);
    let mut params: Vec<rusqlite::types::Value> =
        vec![rusqlite::types::Value::from(min_loc as i64)];
    if let Some(scope) = scope_value {
        params.push(rusqlite::types::Value::from(scope));
    }
    let default_filters = min_loc == 0 && path.is_none();
    let total: i64 = if default_filters {
        db::meta_get(conn, "callable_count")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    } else {
        conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM symbols s
                 WHERE s.kind IN ('function','method')
                   AND s.effective_loc > 0 AND s.effective_loc >= ?1 {gen_filter}"
            ),
            rusqlite::params_from_iter(params.iter()),
            |r| r.get(0),
        )?
    };
    if total == 0 {
        return Err(AwError::NoResultsCaveated {
            caveat: "aw: complexity — static line-count ranking; not comparable to hotspots",
            detail: "0 symbols match the thresholds".to_string(),
        });
    }
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
         WHERE s.kind IN ('function','method')
           AND s.effective_loc > 0 AND s.effective_loc >= ?1 {gen_filter}
         ORDER BY s.effective_loc DESC, s.qualname, s.file_path, s.line_start
         LIMIT {limit}"
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
    let shown = rows.len().min(limit);
    let displayed: BTreeSet<&str> = rows.iter().take(shown).map(|(p, _)| p.as_str()).collect();
    let changed = displayed
        .iter()
        .filter(|p| file_changed(conn, root, p))
        .count();
    let stale = changed > 0;
    let data_lines: Vec<String> = rows
        .iter()
        .take(shown)
        .map(|(_, line)| line.clone())
        .collect();
    let mut out =
        vec!["aw: complexity — static line-count ranking; not comparable to hotspots".to_string()];
    if stale {
        out.push(format!(
            "aw: {changed} of {} result files changed since build — run aw build",
            displayed.len()
        ));
    }
    out.extend(data_lines.iter().cloned());
    if (total as usize) > shown {
        out.push(format!("showing {shown} of {total}"));
        eprintln!("[aw] capped at {shown} results ({total} total)");
    }
    crate::journal::record_result(total as usize, shown, total as usize, stale, 0);
    Ok(out)
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
            for (path, line, qualname) in cands.iter().take(MAX_AMBIGUOUS_CANDIDATES) {
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
pub fn files(conn: &Connection, path: Option<&str>, limit: usize) -> Result<Vec<String>> {
    let limit = clamp_limit(limit)?;
    if let Some(p) = path {
        validate_path_scope(p, false)?;
    }
    let scope = path.map(|p| p.trim_start_matches("./").trim_end_matches('/'));
    let mut sql = String::from(
        "SELECT f.path, COUNT(s.id), f.size, f.generated
         FROM files f LEFT JOIN symbols s ON s.file_id = f.id",
    );
    let mut params: Vec<String> = Vec::new();
    if let Some(scope) = scope {
        sql.push_str(" WHERE (f.path = ?1 OR substr(f.path, 1, length(?1) + 1) = ?1 || '/')");
        params.push(scope.to_string());
    }
    let total: usize = if let Some(scope) = scope {
        conn.query_row(
            "SELECT COUNT(*) FROM files f WHERE f.path = ?1 OR substr(f.path, 1, length(?1) + 1) = ?1 || '/'",
            [scope],
            |r| r.get::<_, i64>(0),
        )? as usize
    } else {
        conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get::<_, i64>(0))? as usize
    };
    sql.push_str(&format!(" GROUP BY f.id ORDER BY f.path LIMIT {limit}"));
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<(String, i64, i64, i64)> = if params.is_empty() {
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<std::result::Result<_, _>>()?
    } else {
        stmt.query_map([&params[0]], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<std::result::Result<_, _>>()?
    };
    let files_caveat = "aw: files — build-time inventory; files created after the build are absent";
    if rows.is_empty() {
        return Err(AwError::NoResultsCaveated {
            caveat: files_caveat,
            detail: match scope {
                Some(s) => format!("0 indexed files in '{s}'"),
                None => "0 indexed files".to_string(),
            },
        });
    }
    let shown = rows.len();
    let mut out = vec![files_caveat.to_string()];
    out.extend(rows.into_iter().map(|(path, syms, bytes, gen)| {
        format!("{path} symbols={syms} bytes={bytes} generated={gen}")
    }));
    if total > shown {
        out.push(format!("showing {shown} of {total}"));
        eprintln!("[aw] capped at {shown} results ({total} total)");
    }
    crate::journal::record_result(total, shown, total, false, 0);
    Ok(out)
}

/// Unresolved edge names with locations (Amendment 19). Report shape: header,
/// then one `path:line kind name` row per unresolved site.
pub fn unresolved_names(
    conn: &Connection,
    root: &Path,
    path: Option<&str>,
    limit: usize,
) -> Result<Vec<String>> {
    let limit = clamp_limit(limit)?;
    if let Some(p) = path {
        validate_path_scope(p, false)?;
    }
    let (scope, scope_value) = path_scope_sql("f", path, 1);
    let scope_params: Vec<rusqlite::types::Value> = scope_value
        .map(rusqlite::types::Value::from)
        .into_iter()
        .collect();
    let total: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM unresolved_calls u JOIN files f ON f.id = u.file_id WHERE u.site_line > 0 {scope}"
        ),
        rusqlite::params_from_iter(scope_params.iter()),
        |r| r.get(0),
    )?;
    if total == 0 {
        return Err(AwError::NoResults(
            "0 unresolved names (external/builtin calls are not counted)".to_string(),
        ));
    }
    let files: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT u.file_id) FROM unresolved_calls u JOIN files f ON f.id = u.file_id WHERE u.site_line > 0 {scope}"
        ),
        rusqlite::params_from_iter(scope_params.iter()),
        |r| r.get(0),
    )?;
    let mut stmt = conn.prepare(&format!(
        "SELECT f.path, u.site_line, u.kind, u.dst_name
         FROM unresolved_calls u JOIN files f ON f.id = u.file_id
         WHERE u.site_line > 0 {scope}
         ORDER BY f.path, u.site_line, u.kind, u.dst_name LIMIT {limit}"
    ))?;
    let rows: Vec<(String, i64, String, String)> = stmt
        .query_map(rusqlite::params_from_iter(scope_params.iter()), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let shown = rows.len().min(limit);
    let displayed: BTreeSet<&str> = rows
        .iter()
        .take(shown)
        .map(|(p, _, _, _)| p.as_str())
        .collect();
    let changed = displayed
        .iter()
        .filter(|p| file_changed(conn, root, p))
        .count();
    let stale = changed > 0;
    let unlocated: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM unresolved_calls u JOIN files f ON f.id = u.file_id
             WHERE u.site_line = 0 {scope}"
        ),
        rusqlite::params_from_iter(scope_params.iter()),
        |r| r.get(0),
    )?;
    if unlocated > 0 {
        eprintln!("[aw] {unlocated} unresolved row(s) have site_line=0 and are omitted");
    }
    let mut out = vec![format!(
        "aw: unresolved={total} rows in {files} files (call/import/inherit)"
    )];
    if stale {
        out.push(format!(
            "aw: {changed} of {} result files changed since build — run aw build",
            displayed.len()
        ));
    }
    out.extend(
        rows.into_iter()
            .take(shown)
            .map(|(path, line, kind, name)| format!("{path}:{line} {kind} {name}")),
    );
    if (total as usize) > shown {
        out.push(format!("showing {shown} of {total} rows"));
        eprintln!("[aw] capped at {shown} results ({total} total)");
    }
    crate::journal::record_result(total as usize, shown, total as usize, stale, 0);
    Ok(out)
}

/// Unresolved-callee report (Amendment 5): files with counts, descending.
/// Report shape — cap, no staleness; header carries the index total.
pub fn unresolved(conn: &Connection, path: Option<&str>, limit: usize) -> Result<Vec<String>> {
    let limit = clamp_limit(limit)?;
    if let Some(p) = path {
        validate_path_scope(p, false)?;
    }
    let (scope, scope_value) = path_scope_sql("f", path, 1);
    let scope_params: Vec<rusqlite::types::Value> = scope_value
        .map(rusqlite::types::Value::from)
        .into_iter()
        .collect();
    let total: i64 = conn.query_row(
        &format!("SELECT COALESCE(SUM(unresolved), 0) FROM files f WHERE 1=1 {scope}"),
        rusqlite::params_from_iter(scope_params.iter()),
        |r| r.get(0),
    )?;
    if total == 0 {
        return Err(AwError::NoResults(
            "0 unresolved in indexed scope (external/builtin calls are not counted)".to_string(),
        ));
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT f.path, f.unresolved FROM files f WHERE f.unresolved > 0 {scope}
         ORDER BY f.unresolved DESC, f.path"
    ))?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(scope_params.iter()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let n_files = rows.len();
    let shown = n_files.min(limit);
    // Name-level view (R7 L4): file counts alone say where resolution failed,
    // not what it missed. `[internal]` marks names that exist in the index —
    // an actionable resolver gap — versus external/builtin calls.
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
        .query_map(rusqlite::params_from_iter(scope_params.iter()), |r| {
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
        rusqlite::params_from_iter(scope_params.iter()),
        |r| r.get(0),
    )?;
    let distinct: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT uc.dst_name) FROM unresolved_calls uc
             JOIN files f ON f.id = uc.file_id WHERE uc.kind = 'call' {scope}"
        ),
        rusqlite::params_from_iter(scope_params.iter()),
        |r| r.get(0),
    )?;
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
    if n_files > shown {
        out.push(format!("showing {} of {}", shown, n_files));
        eprintln!("[aw] capped at {} results ({} total)", shown, n_files);
    }
    crate::journal::record_result(n_files, shown, n_files, false, 0);
    Ok(out)
}

/// Symbols of one indexed file (Amendment 5): `path:line kind qualname` by
/// line; single-file staleness header. With `outline` (Amendment 12a) each
/// line gains the stored one-line signature; without it, byte-identical to
/// the pre-v4 contract.
pub fn symbols(
    conn: &Connection,
    root: &Path,
    file: &str,
    outline: bool,
    metrics: bool,
    limit: usize,
) -> Result<Vec<String>> {
    let limit = clamp_limit(limit)?;
    let rel = file
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string();
    // `.` and `./` mean the repository root; the empty prefix matches every
    // indexed file without generating a "/" prefix.
    let rel = if rel == "." { String::new() } else { rel };
    let is_file: i64 =
        conn.query_row("SELECT COUNT(*) FROM files WHERE path = ?1", [&rel], |r| {
            r.get(0)
        })?;
    if is_file == 0 {
        if looks_like_source_file(&rel) {
            return Err(AwError::NoResults(format!("path '{file}' not indexed")));
        }
        return symbols_directory(conn, root, &rel, outline, metrics, limit);
    }
    let sql = if outline {
        "SELECT s.kind, s.qualname, s.line_start, s.signature, s.effective_loc
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE f.path = ?1
         ORDER BY s.line_start, s.qualname"
    } else {
        "SELECT s.kind, s.qualname, s.line_start, '', s.effective_loc
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE f.path = ?1
         ORDER BY s.line_start, s.qualname"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map([&rel], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if rows.is_empty() {
        return Err(AwError::NoResults(format!("path '{file}' not indexed")));
    }
    let stale = file_changed(conn, root, &rel);
    let n_symbols = rows.len();
    let shown = n_symbols.min(limit);
    let data_lines: Vec<String> = rows
        .into_iter()
        .take(shown)
        .map(|(kind, qualname, line, signature, loc)| {
            let mut text = format!("{rel}:{line} {kind} {qualname}");
            if !signature.is_empty() {
                text.push(' ');
                text.push_str(&signature);
            }
            if metrics {
                text.push_str(&format!(" loc={loc}"));
            }
            text
        })
        .collect();
    crate::journal::record_result(n_symbols, shown, n_symbols, stale, 0);
    let mut out = Vec::new();
    if stale {
        out.push("aw: 1 of 1 result files changed since build — run aw build".to_string());
    }
    out.extend(data_lines);
    if n_symbols > shown {
        out.push(format!("showing {} of {}", shown, n_symbols));
        eprintln!("[aw] capped at {} results ({} total)", shown, n_symbols);
    }
    Ok(out)
}

/// A missing path with a source extension is an unknown file, not a directory
/// scope; keep the pre-P8 `path '<file>' not indexed` contract for it.
fn looks_like_source_file(rel: &str) -> bool {
    matches!(
        rel.rsplit('.').next(),
        Some("py" | "ts" | "tsx" | "js" | "jsx" | "mts" | "cts" | "mjs" | "cjs" | "rs")
    )
}

/// Directory mode for `aw symbols <dir>` (Amendment 19): indexed prefix only,
/// grouped per file, cap counts symbol data lines.
fn symbols_directory(
    conn: &Connection,
    root: &Path,
    dir: &str,
    outline: bool,
    metrics: bool,
    limit: usize,
) -> Result<Vec<String>> {
    validate_path_scope(dir, true)?;
    let display = if dir.is_empty() { "." } else { dir };
    let prefix = if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}/")
    };
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE substr(f.path, 1, length(?1)) = ?1",
        [&prefix],
        |r| r.get(0),
    )?;
    if total == 0 {
        return Err(AwError::NoResults(format!(
            "0 symbols in '{dir}' (indexed)"
        )));
    }
    let n_files: i64 = conn.query_row(
        "SELECT COUNT(*) FROM files f WHERE substr(f.path, 1, length(?1)) = ?1",
        [&prefix],
        |r| r.get(0),
    )?;
    let sql = format!(
        "SELECT f.path, s.kind, s.qualname, s.line_start, s.signature, s.effective_loc
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE substr(f.path, 1, length(?1)) = ?1
         ORDER BY f.path, s.line_start, s.qualname LIMIT {limit}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<(String, SymbolRow)> = stmt
        .query_map([&prefix], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    if outline {
                        r.get::<_, String>(4)?
                    } else {
                        String::new()
                    },
                    r.get::<_, i64>(5)?,
                ),
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let displayed_paths: std::collections::BTreeSet<&str> =
        rows.iter().map(|(p, _)| p.as_str()).collect();
    let changed = displayed_paths
        .iter()
        .filter(|p| file_changed(conn, root, p))
        .count();
    let mut out = Vec::new();
    if changed > 0 {
        out.push(format!(
            "aw: {changed} of {} result files changed since build — run aw build",
            displayed_paths.len()
        ));
    }
    out.push(format!(
        "aw: dir {display} — {n_files} files, {total} symbols"
    ));
    let mut last_path = "";
    for (path, (kind, qualname, line, signature, loc)) in &rows {
        if path != last_path {
            out.push(path.clone());
            last_path = path;
        }
        let mut text = format!("{path}:{line} {kind} {qualname}");
        if !signature.is_empty() {
            text.push(' ');
            text.push_str(signature);
        }
        if metrics {
            text.push_str(&format!(" loc={loc}"));
        }
        out.push(text);
    }
    let shown = rows.len();
    if total as usize > shown {
        out.push(format!("showing {shown} of {total}"));
        eprintln!("[aw] capped at {shown} results ({total} total)");
    }
    crate::journal::record_result(total as usize, shown, total as usize, changed > 0, 0);
    Ok(out)
}

/// Cardinality of the exact ∪ FTS result set by symbol identity. The capped
/// page overlap cannot stand in for the full overlap.
fn search_union_total(conn: &Connection, exact_name: &str, fts_expr: &str) -> Result<usize> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM (
             SELECT s.id FROM symbols s WHERE (s.name = ?1 OR s.qualname = ?1)
             UNION
             SELECT s.id FROM symbols_fts ft JOIN symbols s ON s.id = ft.rowid
              WHERE symbols_fts MATCH ?2
         )",
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

/// FTS page, total, and the expression actually used.
fn search_fts_page(
    conn: &Connection,
    expr: String,
    limit: usize,
) -> Result<(Vec<Pointer>, usize, String)> {
    let total: usize = conn.query_row(
        "SELECT COUNT(*) FROM symbols_fts ft
         JOIN symbols s ON s.id = ft.rowid JOIN files f ON f.id = s.file_id
         WHERE symbols_fts MATCH ?1",
        [&expr],
        |r| r.get::<_, i64>(0),
    )? as usize;
    let mut stmt = conn.prepare(&format!(
        "SELECT f.path, s.line_start, s.qualname, s.signature
         FROM symbols_fts ft
         JOIN symbols s ON s.id = ft.rowid
         JOIN files f ON f.id = s.file_id
         WHERE symbols_fts MATCH ?1
         -- Tie-break on stable columns: rowids differ between fresh and
         -- carried builds, so bm25 alone is not deterministic (R6d A9).
         ORDER BY bm25(symbols_fts), s.qualname, f.path, s.line_start LIMIT {limit}"
    ))?;
    let rows = stmt.query_map([&expr], search_pointer)?;
    let ptrs = rows.collect::<std::result::Result<_, _>>()?;
    Ok((ptrs, total, expr))
}

/// Exact-identity page and total, empty when the exact set has no rows.
fn search_exact_page(
    conn: &Connection,
    exact_name: &str,
    limit: usize,
) -> Result<(Vec<Pointer>, usize)> {
    let exact_total: usize = conn.query_row(
        "SELECT COUNT(*) FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE (s.name = ?1 OR s.qualname = ?1)",
        [exact_name],
        |r| r.get::<_, i64>(0),
    )? as usize;
    if exact_total == 0 {
        return Ok((Vec::new(), 0));
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT f.path, s.line_start, s.qualname, s.signature
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE (s.name = ?1 OR s.qualname = ?1)
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
pub fn search(conn: &Connection, root: &Path, terms: &str, limit: usize) -> Result<Vec<String>> {
    let limit = clamp_limit(limit)?;
    let (toks, expr) = search_expr(conn, terms)?;
    let (fts_ptrs, total, expr) = search_fts_page(conn, expr, limit)?;
    if toks.len() == 1 {
        let exact_name = toks.join(" ");
        let (exact_ptrs, exact_total) = search_exact_page(conn, &exact_name, limit)?;
        if exact_total > 0 {
            let merged = merge_search_pointers(exact_ptrs, fts_ptrs);
            // The capped page overlap is not the full overlap: count the
            // union of exact and FTS identities.
            let merged_total = search_union_total(conn, &exact_name, &expr)?;
            let out = collect_output(
                conn,
                root,
                merged.into_iter().map(Ok::<_, rusqlite::Error>),
                limit,
                &format!("0 symbols matched '{terms}' — try fewer or broader terms"),
                Some(merged_total),
            )?;
            return search_lines(out);
        }
    }
    let out = collect_output(
        conn,
        root,
        fts_ptrs.into_iter().map(Ok::<_, rusqlite::Error>),
        limit,
        &format!("0 symbols matched '{terms}' — try fewer or broader terms"),
        Some(total),
    )?;
    search_lines(out)
}
