//! Query-name resolution: exact and `Class.method` suffix lookups over the
//! symbol table, with the module-disambiguation and dedupe phases shared by
//! every name-addressed entry point.

use super::*;

/// One symbol row for name resolution: (id, qualname, path, line, kind).
type CandidateRow = (i64, String, String, i64, String);

fn candidate_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<CandidateRow> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
}

/// SELECT/JOIN shared by both `target_ids` statements; the column order is
/// fixed by `candidate_row`.
const CANDIDATE_SELECT: &str = "SELECT s.id, s.qualname, f.path, s.line_start, s.kind
     FROM symbols s JOIN files f ON f.id = s.file_id";

/// Exact-identity predicate shared by the resolver (`target_ids`) and the
/// search pages: a symbol matches the query by bare name or full qualname,
/// bound as parameter ?1. Parenthesized so the `OR` cannot leak into a wider
/// `AND`.
pub(super) const EXACT_IDENTITY_PREDICATE: &str = "(s.name = ?1 OR s.qualname = ?1)";

/// Resolve the query name to target symbol ids. Multiple distinct symbols ->
/// exit 2 with a `path:line qualname` candidate list on stdout (CUJ4).
pub(super) fn target_ids(conn: &Connection, name: &str) -> Result<(Vec<i64>, Vec<NameCandidate>)> {
    let mut stmt = conn.prepare(&format!(
        "{CANDIDATE_SELECT}
         WHERE {EXACT_IDENTITY_PREDICATE}
         ORDER BY s.qualname, f.path",
    ))?;
    let rows = stmt.query_map(rusqlite::params![name], candidate_row)?;
    let mut all: Vec<_> = rows.collect::<std::result::Result<_, _>>()?;
    if all.is_empty() && name.contains('.') {
        all = suffix_candidates(conn, name)?;
    }
    Ok(disambiguate(all))
}

/// `Class.method` suffix form: the full qualname is `pkg.Class.method`, so
/// an exact name/qualname match misses it. Fall back to a suffix scan and
/// confirm the exact suffix in Rust (LIKE alone treats `_` and `%` as
/// wildcards). Multiple matches stay ambiguous through the caller.
fn suffix_candidates(conn: &Connection, name: &str) -> Result<Vec<CandidateRow>> {
    let pattern = format!("%.{name}");
    let mut stmt = conn.prepare(&format!(
        "{CANDIDATE_SELECT}
         WHERE s.qualname LIKE ?1
         ORDER BY s.qualname, f.path",
    ))?;
    let rows = stmt.query_map([&pattern], candidate_row)?;
    let suffix = format!(".{name}");
    Ok(rows
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|r| r.1.ends_with(&suffix))
        .collect())
}

/// Module-filter and dedupe phase: a module shares the bare name of
/// classes/functions (`button` vs `Button`); when a callable candidate exists,
/// module symbols must not create a false ambiguity (R7 F1). Unknown kinds
/// stay non-module, as before the enum.
fn disambiguate(all: Vec<CandidateRow>) -> (Vec<i64>, Vec<NameCandidate>) {
    let has_callable = all
        .iter()
        .any(|r| SymbolKind::parse(&r.4) != Some(SymbolKind::Module));
    let mut ids = Vec::new();
    let mut cands = Vec::new();
    let mut seen_ids: std::collections::HashSet<i64> = Default::default();
    for (id, qualname, path, line, kind) in all {
        if has_callable && SymbolKind::parse(&kind) == Some(SymbolKind::Module) {
            continue;
        }
        // Dedupe by id: a symbol whose bare name equals its qualname matches
        // both WHERE branches and must not create a false ambiguity. O(1)
        // membership (R6f: a Vec::contains scan was quadratic).
        if seen_ids.insert(id) {
            ids.push(id);
            // Render cap only (R6f): the error carries `total` separately and
            // the caller prints at most MAX_AMBIGUOUS_CANDIDATES lines.
            if cands.len() < MAX_AMBIGUOUS_CANDIDATES {
                cands.push((path, line, qualname));
            }
        }
    }
    (ids, cands)
}
