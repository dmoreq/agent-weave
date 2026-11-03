//! Path-scope validation and bound-parameter SQL fragments shared by queries.

use super::*;

pub(crate) fn path_in_scope(path: &str, scope: &str) -> bool {
    path == scope || path.starts_with(&format!("{scope}/"))
}

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

pub fn path_scope_sql_pub(
    alias: &str,
    path: Option<&str>,
    param: usize,
) -> (String, Option<String>) {
    path_scope_sql(alias, path, param)
}

/// Scope clause over `alias.path` with one bound parameter at `param`.
pub(crate) fn path_scope_sql(
    alias: &str,
    path: Option<&str>,
    param: usize,
) -> (String, Option<String>) {
    match path {
        None => (String::new(), None),
        Some(path) => {
            let scope = path
                .trim_start_matches("./")
                .trim_end_matches('/')
                .to_string();
            (
                format!(
                    " AND ({alias}.path = ?{param} OR ({alias}.path >= ?{param} || '/' AND {alias}.path < ?{param} || '0'))"
                ),
                Some(scope),
            )
        }
    }
}

/// Scope clause over `alias.file_path` with one bound parameter at `param`.
pub(crate) fn file_path_scope_sql(
    alias: &str,
    path: Option<&str>,
    param: usize,
) -> (String, Option<String>) {
    match path {
        None => (String::new(), None),
        Some(path) => {
            let scope = path
                .trim_start_matches("./")
                .trim_end_matches('/')
                .to_string();
            (
                format!(
                    " AND ({alias}.file_path = ?{param} OR ({alias}.file_path >= ?{param} || '/' AND {alias}.file_path < ?{param} || '0'))"
                ),
                Some(scope),
            )
        }
    }
}

pub(crate) fn edge_kind_sql(kinds: &[&str]) -> String {
    kinds
        .iter()
        .map(|k| format!("'{k}'"))
        .collect::<Vec<_>>()
        .join(",")
}
