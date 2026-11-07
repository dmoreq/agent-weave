//! Path-scope validation and bound-parameter SQL fragments shared by queries.

use super::*;

pub fn validate_path_scope(path: &str, allow_root: bool) -> Result<()> {
    let p = path.trim().trim_start_matches("./").trim_end_matches('/');
    let bad_component = !p.is_empty()
        && p.split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..");
    if (!allow_root && (p.is_empty() || p == "."))
        || bad_component
        || p.starts_with('/')
        || p.contains('\\')
    {
        return Err(AwError::Usage(format!(
            "invalid scope '{path}' (repo-relative path required)"
        )));
    }
    Ok(())
}

/// The repo-relative scope spelling used by every query surface.
pub(crate) fn normalize_scope(path: &str) -> &str {
    path.trim_start_matches("./").trim_end_matches('/')
}

/// Scope clause over `alias.<column>`, plus its one bound value, ready for
/// `params_from_iter`. `column` is a compile-time literal from an internal
/// caller (never user input).
pub(crate) fn scoped_params(
    alias: &str,
    column: &str,
    path: Option<&str>,
    param: usize,
) -> (String, Vec<rusqlite::types::Value>) {
    let Some(path) = path else {
        return (String::new(), Vec::new());
    };
    let scope = normalize_scope(path).to_string();
    let clause = format!(
        " AND ({alias}.{column} = ?{param} OR ({alias}.{column} >= ?{param} || '/' AND {alias}.{column} < ?{param} || '0'))"
    );
    (clause, vec![rusqlite::types::Value::from(scope)])
}

/// Bind one chunk of symbol ids as SQLite parameters.
pub(crate) fn id_params(ids: &[i64]) -> Vec<rusqlite::types::Value> {
    ids.iter()
        .map(|i| rusqlite::types::Value::from(*i))
        .collect()
}
