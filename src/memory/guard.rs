//! Ambient memory retrieval: co-retrieve active gotchas/decisions
//! referencing target definitions, files, or symbols.

use std::collections::HashSet;
use std::path::Path;

/// Compact guarded note info for ambient retrieval.
#[derive(Debug, Clone)]
pub struct GuardedNote {
    pub key: String,
    pub kind: String,
    pub summary: String,
    pub target: String,
}

/// Check if a note's `refs` matches a target path or symbol.
pub fn ref_matches_target(refs: &str, target: &str) -> bool {
    let target = target.trim();
    if target.is_empty() {
        return false;
    }
    let target_norm = target.trim_start_matches("./");
    refs.split_whitespace().any(|r| {
        let r_norm = r.trim_start_matches("./");
        r_norm == target_norm
            || r_norm.ends_with(&format!("/{target_norm}"))
            || target_norm.ends_with(&format!("/{r_norm}"))
            || r_norm.ends_with(&format!("::{target_norm}"))
            || target_norm.ends_with(&format!("::{r_norm}"))
            || r_norm.ends_with(&format!(".{target_norm}"))
            || target_norm.ends_with(&format!(".{r_norm}"))
            || target_norm.split("::").any(|seg| seg == r_norm)
            || target_norm.split('.').any(|seg| seg == r_norm)
            || r_norm.split("::").any(|seg| seg == target_norm)
            || r_norm.split('.').any(|seg| seg == target_norm)
    })
}

/// First line of body, capped safely at 80 characters.
pub fn compact_summary(body: &str) -> String {
    let first_line = body.lines().next().unwrap_or("").trim();
    if first_line.chars().count() > 80 {
        let truncated: String = first_line.chars().take(77).collect();
        format!("{truncated}...")
    } else {
        first_line.to_string()
    }
}

/// Find active gotchas and decisions in local and global memory stores
/// that reference any of the provided paths or symbols.
pub fn find_guarding_notes(root: &Path, paths: &[&str], symbols: &[&str]) -> Vec<GuardedNote> {
    let now = crate::db::unix_now();
    let filter = crate::db::MemFilter {
        kind: None,
        scope: None,
        now,
        include_superseded: false,
    };

    let mut results = Vec::new();
    let mut seen_keys = HashSet::new();

    if let Ok(local_conn) = crate::db::open_memory_query(root) {
        collect_guards(
            &local_conn,
            &filter,
            paths,
            symbols,
            &mut seen_keys,
            &mut results,
        );
    }

    if let Ok(global_conn) = crate::db::open_global_memory_query() {
        collect_guards(
            &global_conn,
            &filter,
            paths,
            symbols,
            &mut seen_keys,
            &mut results,
        );
    }

    results
}

fn collect_guards(
    conn: &rusqlite::Connection,
    filter: &crate::db::MemFilter,
    paths: &[&str],
    symbols: &[&str],
    seen_keys: &mut HashSet<String>,
    out: &mut Vec<GuardedNote>,
) {
    let Ok(rows) = crate::db::memory_current_rows(conn, filter, None) else {
        return;
    };
    for row in rows {
        if (row.kind != "decision" && row.kind != "gotcha") || row.refs.is_empty() {
            continue;
        }
        if seen_keys.contains(&row.key) {
            continue;
        }
        let matched = paths
            .iter()
            .chain(symbols.iter())
            .find(|target| ref_matches_target(&row.refs, target));
        if let Some(target) = matched {
            seen_keys.insert(row.key.clone());
            out.push(GuardedNote {
                key: row.key,
                kind: row.kind,
                summary: compact_summary(&row.body),
                target: target.to_string(),
            });
        }
    }
}
