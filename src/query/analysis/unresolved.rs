//! Unresolved-callsite reports: the name-level rows with locations and the
//! per-file count view.

use super::*;
use crate::extract::EdgeSiteKind;

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
    let QueryCtx { conn: _, .. } = *ctx;
    let (limit, clause, params) = scoped_query(path, limit, "f", "path", 1)?;
    let scope = UnresolvedScope {
        conn: ctx.conn,
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
pub fn unresolved(ctx: &QueryCtx<'_>, path: Option<&str>, limit: usize) -> Result<LineQuery> {
    let (limit, clause, params) = scoped_query(path, limit, "f", "path", 1)?;
    let scope = UnresolvedScope {
        conn: ctx.conn,
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
