//! Inventory and analysis queries: dead candidates, complexity, batch resolve,
//! files, unresolved sites, symbols, search, and duplicate candidates.

use super::*;

mod search;
mod symbols;
mod unresolved;

pub use self::search::search;
pub use self::symbols::{symbols, SymbolsOpts};
pub use unresolved::{unresolved, unresolved_names};

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
