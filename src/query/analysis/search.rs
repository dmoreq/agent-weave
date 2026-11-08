//! FTS5 symbol search: the exact-identity fast path, the bm25-ranked page,
//! and their deduplicated merge.

use super::*;

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
