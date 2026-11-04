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
            WHERE kind IN ('calls', 'imports', 'inherits') AND dst_id IS NOT NULL
           UNION
           SELECT s2.id FROM edges e
             JOIN symbols s2 ON s2.qualname = e.dst_name
            WHERE e.kind IN ('calls', 'imports', 'inherits')
              AND e.confidence = 'AMBIGUOUS'
           UNION
           SELECT s2.id FROM edges e
             JOIN symbols src ON src.id = e.src_id
             JOIN symbols s2 ON s2.name = e.dst_name AND s2.file_id = src.file_id
            WHERE e.kind IN ('calls', 'imports', 'inherits')
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
        predicate.push_str(
            " AND f.path NOT LIKE 'tests/%' AND f.path NOT LIKE '%/tests/%'
              AND f.path NOT LIKE 'test/%' AND f.path NOT LIKE '%/test/%'
              AND f.path NOT LIKE '__tests__/%' AND f.path NOT LIKE '%/__tests__/%'
              AND f.path NOT LIKE 'test\\_%.py' ESCAPE '\\'
              AND f.path NOT LIKE '%/test\\_%.py' ESCAPE '\\'
              AND f.path NOT LIKE '%\\_test.py' ESCAPE '\\'
              AND f.path NOT LIKE '%.test.%'
              AND f.path NOT LIKE '%.spec.%'
              AND f.path NOT LIKE '%\\_test.dart' ESCAPE '\\'
              AND s.name NOT LIKE 'test\\_%' ESCAPE '\\'
              AND s.name NOT GLOB 'test[A-Z]*'",
        );
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
        "",
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

/// `aw complexity` (Amendment 19): per-symbol metric ranking.
#[allow(clippy::too_many_arguments)] // CLI surface; a params struct adds no value here.
pub fn complexity(
    conn: &Connection,
    root: &Path,
    min_cyc: usize,
    min_loc: usize,
    min_branches: usize,
    min_nest: usize,
    path: Option<&str>,
    limit: usize,
) -> Result<Vec<String>> {
    let limit = clamp_limit(limit)?;
    if let Some(p) = path {
        validate_path_scope(p, false)?;
    }
    let (gen_filter, scope_value) = file_path_scope_sql("s", path, 5);
    let mut params: Vec<rusqlite::types::Value> = vec![
        rusqlite::types::Value::from(min_cyc as i64),
        rusqlite::types::Value::from(min_loc as i64),
        rusqlite::types::Value::from(min_branches as i64),
        rusqlite::types::Value::from(min_nest as i64),
    ];
    if let Some(scope) = scope_value {
        params.push(rusqlite::types::Value::from(scope));
    }
    let default_filters =
        min_cyc <= 1 && min_loc == 0 && min_branches == 0 && min_nest == 0 && path.is_none();
    let total: i64 = if default_filters {
        db::meta_get(conn, "callable_count")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    } else {
        conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM symbols s
                 WHERE s.kind IN ('function','method')
                   AND s.cyclomatic > 0 AND s.cyclomatic >= ?1
                   AND s.effective_loc >= ?2 AND s.branches >= ?3
                   AND s.nesting >= ?4 {gen_filter}"
            ),
            rusqlite::params_from_iter(params.iter()),
            |r| r.get(0),
        )?
    };
    if total == 0 {
        return Err(AwError::NoResultsCaveated {
            caveat: "aw: complexity — static AST proxy, not cognitive complexity; not comparable to hotspots/file cyclomatic",
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
        "SELECT s.file_path, s.line_start, s.cyclomatic, s.branches, s.nesting,
                s.effective_loc, s.qualname
         FROM symbols s{index_hint}
         WHERE s.kind IN ('function','method')
           AND s.cyclomatic > 0 AND s.cyclomatic >= ?1 AND s.effective_loc >= ?2
           AND s.branches >= ?3 AND s.nesting >= ?4 {gen_filter}
         ORDER BY s.cyclomatic DESC, s.effective_loc DESC, s.qualname,
                  s.file_path, s.line_start LIMIT {limit}"
    ))?;
    let rows: Vec<(String, String)> = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                format!(
                    "{}:{} cyc={} branches={} nest={} loc={} {}",
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, String>(6)?
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
    let mut out = vec![
        "aw: complexity — static AST proxy, not cognitive complexity; not comparable to hotspots/file cyclomatic".to_string(),
    ];
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
    super::record_payload_meta(
        conn,
        path.unwrap_or("complexity"),
        &data_lines,
        stale,
        shown,
        total as usize,
    );
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
        "SELECT s.kind, s.qualname, s.line_start, s.signature,
                s.cyclomatic, s.branches, s.nesting, s.effective_loc
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE f.path = ?1
         ORDER BY s.line_start, s.qualname"
    } else {
        "SELECT s.kind, s.qualname, s.line_start, '',
                s.cyclomatic, s.branches, s.nesting, s.effective_loc
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
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
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
        .map(|(kind, qualname, line, signature, cyc, br, nest, loc)| {
            let mut text = format!("{rel}:{line} {kind} {qualname}");
            if !signature.is_empty() {
                text.push(' ');
                text.push_str(&signature);
            }
            if metrics {
                text.push_str(&format!(" cyc={cyc} branches={br} nest={nest} loc={loc}"));
            }
            text
        })
        .collect();
    // Payload capture (Amendment 10a): the `path:line kind qualname` data
    // lines only — never the staleness header or the cap footer.
    super::record_payload_meta(conn, &rel, &data_lines, stale, shown, n_symbols);
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
        "SELECT f.path, s.kind, s.qualname, s.line_start, s.signature,
                s.cyclomatic, s.branches, s.nesting, s.effective_loc
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
                    r.get::<_, i64>(6)?,
                    r.get::<_, i64>(7)?,
                    r.get::<_, i64>(8)?,
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
    let mut data_lines = Vec::new();
    let mut last_path = "";
    for (path, (kind, qualname, line, signature, cyc, br, nest, loc)) in &rows {
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
            text.push_str(&format!(" cyc={cyc} branches={br} nest={nest} loc={loc}"));
        }
        data_lines.push(text.clone());
        out.push(text);
    }
    let shown = rows.len();
    if total as usize > shown {
        out.push(format!("showing {shown} of {total}"));
        eprintln!("[aw] capped at {shown} results ({total} total)");
    }
    super::record_payload_meta(conn, dir, &data_lines, changed > 0, shown, total as usize);
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

/// FTS page, total, and the expression actually used (morphology fallback).
fn search_fts_page(
    conn: &Connection,
    toks: &[String],
    mut expr: String,
    limit: usize,
) -> Result<(Vec<Pointer>, usize, String)> {
    let mut total: usize = conn.query_row(
        "SELECT COUNT(*) FROM symbols_fts ft
         JOIN symbols s ON s.id = ft.rowid JOIN files f ON f.id = s.file_id
         WHERE symbols_fts MATCH ?1",
        [&expr],
        |r| r.get::<_, i64>(0),
    )? as usize;
    // Morphology fallback (R7): agents ask "handlers" or "rotation" while the
    // index stores the stem. Retry once with bounded derived forms, only on
    // zero hits, so recall improves without changing ranked results.
    if total == 0 {
        let mut variants: Vec<String> = Vec::new();
        for tok in toks {
            derived_forms(tok, &mut variants);
        }
        variants.retain(|v| !v.is_empty() && v.len() >= 3);
        variants.dedup();
        variants.truncate(SEARCH_MAX_TERMS);
        if !variants.is_empty() {
            let alt = variants
                .iter()
                .map(|tok| format!("\"{}\"*", tok.replace('"', "")))
                .collect::<Vec<_>>()
                .join(" OR ");
            let alt_total: usize = conn.query_row(
                "SELECT COUNT(*) FROM symbols_fts WHERE symbols_fts MATCH ?1",
                [&alt],
                |r| r.get::<_, i64>(0),
            )? as usize;
            if alt_total > 0 {
                expr = alt;
                total = alt_total;
            }
        }
    }
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
/// for cap, post-cap staleness, and payload capture.
pub fn search(conn: &Connection, root: &Path, terms: &str, limit: usize) -> Result<Vec<String>> {
    let limit = clamp_limit(limit)?;
    let (toks, expr) = search_expr(conn, terms)?;
    let (fts_ptrs, total, expr) = search_fts_page(conn, &toks, expr, limit)?;
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
                terms,
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
        terms,
        fts_ptrs.into_iter().map(Ok::<_, rusqlite::Error>),
        limit,
        &format!("0 symbols matched '{terms}' — try fewer or broader terms"),
        Some(total),
    )?;
    search_lines(out)
}

/// Clone candidates (Amendment 12d): merge adjacent matching shingle windows
/// per file, pair the merged regions, filter by `min_lines`, emit
/// `pathA:startA-endA pathB:startB-endB` under the standard cap contract.
/// Caveat line always first — candidates, never proof. NOT payload-journaled
/// (Amendment 12 deviation: pair lines are not single pointers, so the
/// `usage --verify` structural checks do not apply).
type CloneGroup = Vec<(String, i64, i64)>;
type ClonePair = (String, i64, i64, String, i64, i64);

pub fn clones(
    conn: &Connection,
    root: &Path,
    min_lines: usize,
    path: Option<&str>,
    limit: usize,
) -> Result<Vec<String>> {
    let limit = clamp_limit(limit)?;
    if let Some(p) = path {
        validate_path_scope(p, false)?;
    }
    let min_lines = min_lines.max(crate::clones::WINDOW);
    let groups = clone_groups(conn)?;
    let (pairs, capped) = clone_pairs(groups);
    let pairs = merge_clone_pairs(pairs, min_lines, path);
    render_clones(conn, root, &pairs, capped, limit)
}

/// Load clone shingle groups that occur at least twice.
fn clone_groups(conn: &Connection) -> Result<std::collections::BTreeMap<i64, CloneGroup>> {
    let mut stmt = conn.prepare(
        "SELECT cg.shingle_hash, f.path, cg.line_start, cg.line_end
         FROM clone_groups cg JOIN files f ON f.id = cg.file_id
         WHERE cg.shingle_hash IN (
             SELECT shingle_hash FROM clone_groups
             GROUP BY shingle_hash HAVING COUNT(*) >= 2)
         ORDER BY cg.shingle_hash, f.path, cg.line_start",
    )?;
    let mut groups: std::collections::BTreeMap<i64, Vec<(String, i64, i64)>> = Default::default();
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    for r in rows {
        let (h, path, s, e) = r?;
        groups.entry(h).or_default().push((path, s, e));
    }
    Ok(groups)
}

/// Bounded pair generation. A shared header can put thousands of regions in
/// one shingle group and the full pair set is quadratic, so caps report a
/// lower bound instead of unbounded work.
fn clone_pairs(groups: std::collections::BTreeMap<i64, CloneGroup>) -> (Vec<ClonePair>, bool) {
    const MAX_REGIONS_PER_GROUP: usize = 64;
    const MAX_PAIRS: usize = 20_000;
    let mut capped = false;
    let mut seen = std::collections::BTreeSet::new();
    let mut pairs: Vec<ClonePair> = Vec::new();
    'groups: for regions in groups.values() {
        // Merge overlapping / near-adjacent windows (gap < window) per file:
        // one long clone produces many overlapping windows, one region.
        let mut merged: Vec<(String, i64, i64)> = Vec::new();
        for (path, s, e) in regions {
            if let Some(last) = merged.last_mut() {
                if last.0 == *path && *s <= last.2 + 4 {
                    last.2 = (*e).max(last.2);
                    continue;
                }
            }
            merged.push((path.clone(), *s, *e));
        }
        if merged.len() < 2 {
            continue;
        }
        if merged.len() > MAX_REGIONS_PER_GROUP {
            merged.truncate(MAX_REGIONS_PER_GROUP);
            capped = true;
        }
        for i in 0..merged.len() {
            for j in (i + 1)..merged.len() {
                if pairs.len() >= MAX_PAIRS {
                    capped = true;
                    break 'groups;
                }
                let (pa, sa, ea) = &merged[i];
                let (pb, sb, eb) = &merged[j];
                let (fa, sta, ena, fb, stb, enb) =
                    if (pa.as_str(), *sa, *ea) <= (pb.as_str(), *sb, *eb) {
                        (pa, *sa, *ea, pb, *sb, *eb)
                    } else {
                        (pb, *sb, *eb, pa, *sa, *ea)
                    };
                let key = (fa.clone(), sta, ena, fb.clone(), stb, enb);
                if seen.insert(key) {
                    pairs.push((fa.clone(), sta, ena, fb.clone(), stb, enb));
                }
            }
        }
    }
    (pairs, capped)
}

/// Merge per-window pairs into maximal spans, then apply min-lines and path
/// filters. Merge-before-filter is required: filtering 5-line windows first
/// made every min-lines > 5 a false negative.
fn merge_clone_pairs(
    pairs: Vec<ClonePair>,
    min_lines: usize,
    path: Option<&str>,
) -> Vec<ClonePair> {
    // Group by file pair first: sorting globally interleaves different pairs,
    // so merging only against the previous row leaves overlaps unmerged.
    type SpanPair = (i64, i64, i64, i64);
    let mut by_pair: std::collections::BTreeMap<(String, String), Vec<SpanPair>> =
        Default::default();
    for (fa, sa, ea, fb, sb, eb) in pairs {
        by_pair.entry((fa, fb)).or_default().push((sa, ea, sb, eb));
    }
    let mut merged_pairs: Vec<(String, i64, i64, String, i64, i64)> = Vec::new();
    for ((fa, fb), mut spans) in by_pair {
        spans.sort_unstable();
        let mut cur: Option<(i64, i64, i64, i64)> = None;
        for (sa, ea, sb, eb) in spans {
            match cur {
                Some((csa, cea, csb, ceb)) if sa <= cea && sb <= ceb => {
                    cur = Some((csa, cea.max(ea), csb, ceb.max(eb)));
                }
                Some(done) => {
                    merged_pairs.push((fa.clone(), done.0, done.1, fb.clone(), done.2, done.3));
                    cur = Some((sa, ea, sb, eb));
                }
                None => cur = Some((sa, ea, sb, eb)),
            }
        }
        if let Some((sa, ea, sb, eb)) = cur {
            merged_pairs.push((fa, sa, ea, fb, sb, eb));
        }
    }
    merged_pairs.retain(|(_, sa, ea, _, sb, eb)| {
        (*ea - *sa + 1) >= min_lines as i64 && (*eb - *sb + 1) >= min_lines as i64
    });
    if let Some(scope) = path {
        let scope = scope.trim_start_matches("./").trim_end_matches('/');
        merged_pairs
            .retain(|(fa, _, _, fb, _, _)| path_in_scope(fa, scope) && path_in_scope(fb, scope));
    }
    // Longest merged spans first, deterministic tie-break by paths/spans.
    merged_pairs.sort_by(|a, b| {
        let la = (a.2 - a.1).max(a.5 - a.4);
        let lb = (b.2 - b.1).max(b.5 - b.4);
        lb.cmp(&la).then_with(|| {
            (a.0.as_str(), a.1, a.3.as_str(), a.4).cmp(&(b.0.as_str(), b.1, b.3.as_str(), b.4))
        })
    });
    merged_pairs
}

/// Render merged pairs with staleness/journal accounting and enclosing-symbol
/// identity. Journaling stays at this public-surface phase.
fn render_clones(
    conn: &Connection,
    root: &Path,
    pairs: &[ClonePair],
    capped: bool,
    limit: usize,
) -> Result<Vec<String>> {
    if pairs.is_empty() {
        return Err(AwError::NoResultsCaveated {
            caveat: CLONES_CAVEAT,
            detail: "0 clone candidates found".to_string(),
        });
    }
    if capped {
        eprintln!("[aw] clone pair generation capped — result is a lower bound");
    }
    let shown = pairs.len().min(limit);
    let displayed = &pairs[..shown];
    let files: std::collections::BTreeSet<&String> = displayed
        .iter()
        .flat_map(|(a, _, _, b, _, _)| [a, b])
        .collect();
    let n_files = files.len();
    let mut changed = 0usize;
    for f in &files {
        if file_changed(conn, root, f) {
            changed += 1;
        }
    }
    let stale = changed > 0;
    crate::journal::record_result(shown, shown, pairs.len(), stale, 0);
    let mut out = vec![CLONES_CAVEAT.to_string()];
    if stale {
        out.push(format!(
            "aw: {changed} of {n_files} result files changed since build — run aw build"
        ));
    }
    // Identity (R7 L4): a bare span forces a file read to learn what the
    // duplicated block is. Name the enclosing symbol per side, when any.
    let enclosing = |path: &str, start: i64, end: i64| -> Option<String> {
        // Largest overlap wins: a merged span can begin in the previous
        // function's last line, and the shallowest containing symbol is
        // rarely the duplicated unit.
        conn.query_row(
            "SELECT s.qualname FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE f.path = ?1 AND s.line_start <= ?3 AND s.line_end >= ?2
             ORDER BY (MIN(s.line_end, ?3) - MAX(s.line_start, ?2)) DESC,
                      s.line_start DESC
             LIMIT 1",
            rusqlite::params![path, start, end],
            |r| r.get::<_, String>(0),
        )
        .ok()
    };
    for (pa, sa, ea, pb, sb, eb) in displayed {
        let ta = enclosing(pa, *sa, *ea)
            .map(|q| format!(" ({q})"))
            .unwrap_or_default();
        let tb = enclosing(pb, *sb, *eb)
            .map(|q| format!(" ({q})"))
            .unwrap_or_default();
        out.push(format!("{pa}:{sa}-{ea}{ta} {pb}:{sb}-{eb}{tb}"));
    }
    if pairs.len() > shown {
        out.push(format!("showing {} of {}", shown, pairs.len()));
        eprintln!("[aw] capped at {} results ({} total)", shown, pairs.len());
    }
    Ok(out)
}
