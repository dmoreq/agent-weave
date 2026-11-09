//! Meta manifest, source revision, and mtime/size stat helpers.

use super::*;

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// Read one meta-manifest value (built_at, source_rev, counters).
pub fn meta_get(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .ok()
}

/// Meta key names — the single vocabulary shared by the build-time writers
/// (`build.rs::write_meta_identity` and `build.rs::write_meta_counts`), the
/// carry-forward gate (`schema_compatible`), and the display readers
/// (`doctor`, `insights`), so a new key is registered under one name in all
/// definitions and they cannot drift apart. The `*_SRC_FILES` counters have
/// no `meta_counters` fallback: their readers (`query/impact.rs`) fall back
/// to an edge scan instead.
pub(crate) mod meta_keys {
    // Identity and provenance keys written by `write_meta_identity`.
    pub const ENGRAM_VERSION: &str = "engram_version";
    pub const GRAMMAR_VERSION: &str = "grammar_version";
    pub const SIGNATURE_VERSION: &str = "signature_version";
    pub const METRICS_VERSION: &str = "metrics_version";
    pub const BUILT_AT: &str = "built_at";
    pub const SOURCE_REV: &str = "source_rev";
    pub const FILE_COUNT: &str = "file_count";
    pub const SYMBOL_COUNT: &str = "symbol_count";
    pub const EDGE_COUNT: &str = "edge_count";
    pub const PARSE_ERROR_COUNT: &str = "parse_error_count";
    pub const EDGE_CALLS: &str = "edge_calls";
    pub const EDGE_IMPORTS: &str = "edge_imports";
    pub const EDGE_INHERITS: &str = "edge_inherits";
    pub const UNRESOLVED_CALLEE_COUNT: &str = "unresolved_callee_count";
    pub const EDGE_CALLS_SRC_FILES: &str = "edge_calls_src_files";
    pub const EDGE_IMPORTS_SRC_FILES: &str = "edge_imports_src_files";
    pub const EDGE_INHERITS_SRC_FILES: &str = "edge_inherits_src_files";
    pub const EDGE_ALL_SRC_FILES: &str = "edge_all_src_files";
}

/// Build-time meta counters with their fallback SQL (R7 L4): one (key, SQL)
/// pair per counter, shared by `eg stats` (insights) and the no-op build
/// line (build), so a new counter is registered once and both readers stay
/// in sync. The fallback COUNT keeps pre-counter indexes working. Kind
/// filters render through the enum so the vocabularies stay single-sourced.
fn meta_counters() -> Vec<(&'static str, String)> {
    let edge_count = |kind: EdgeKind| -> String {
        format!(
            "SELECT COUNT(*) FROM edges WHERE kind = '{}'",
            kind.as_str()
        )
    };
    vec![
        (
            meta_keys::FILE_COUNT,
            "SELECT COUNT(*) FROM files".to_string(),
        ),
        (
            meta_keys::SYMBOL_COUNT,
            "SELECT COUNT(*) FROM symbols".to_string(),
        ),
        (
            meta_keys::EDGE_COUNT,
            "SELECT COUNT(*) FROM edges".to_string(),
        ),
        (meta_keys::EDGE_CALLS, edge_count(EdgeKind::Calls)),
        (meta_keys::EDGE_IMPORTS, edge_count(EdgeKind::Imports)),
        (meta_keys::EDGE_INHERITS, edge_count(EdgeKind::Inherits)),
        (
            meta_keys::PARSE_ERROR_COUNT,
            "SELECT COALESCE(SUM(parse_status != 'ok'), 0) FROM files".to_string(),
        ),
        (
            meta_keys::UNRESOLVED_CALLEE_COUNT,
            "SELECT COALESCE(SUM(unresolved), 0) FROM files".to_string(),
        ),
    ]
}

/// Counter value for `key`: the build-time meta entry when present, else the
/// registered fallback COUNT (pre-counter index).
pub(crate) fn meta_count(conn: &Connection, key: &str) -> Result<i64> {
    if let Some(v) = meta_get(conn, key).and_then(|v| v.parse::<i64>().ok()) {
        return Ok(v);
    }
    let counters = meta_counters();
    let sql = counters
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, sql)| sql.as_str())
        .unwrap_or("SELECT 0");
    Ok(conn.query_row(sql, [], |r| r.get(0))?)
}

/// Probe the source repo's git revision with a `-dirty` suffix on a dirty tree.
pub fn source_rev(root: &Path) -> String {
    let head = std::process::Command::new("git")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    let Some(mut rev) = head else {
        return "unknown".to_string();
    };
    let dirty = std::process::Command::new("git")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| !o.stdout.is_empty());
    if dirty {
        rev.push_str("-dirty");
    }
    rev
}

/// Nanosecond-precision mtime from already-captured metadata; `None` when
/// the platform cannot report it. The single copy of the fast-path/staleness
/// arithmetic: the walk stamps `WalkedFile.mtime` through this function and
/// every later comparison goes through `stat_mtime`, so both sides of a
/// comparison share one implementation and cannot drift.
pub fn metadata_mtime(metadata: &std::fs::Metadata) -> Option<i64> {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
}

/// Nanosecond-precision stat mtime as i64 (fast-path and staleness key).
/// Nanoseconds close the same-second, same-size edit hole that silently kept
/// a stale index.
pub fn stat_mtime(path: &Path) -> Option<i64> {
    fs::metadata(path).ok().and_then(|m| metadata_mtime(&m))
}

/// stat size as i64.
pub fn stat_size(path: &Path) -> Option<i64> {
    fs::metadata(path).ok().map(|m| m.len() as i64)
}

// ---------------------------------------------------------------------------
// Project memory store (`.engram/memory.db`) — companion design:
// docs/architecture/memory-store.md. All memory.db
// lifecycle lives here so the write-confinement allowlist stays unchanged.
// Pragma regime (deliberately NOT the index build's OFF/OFF/EXCLUSIVE — the
// design's Storage note): default rollback journal, synchronous = FULL, and a
// busy_timeout, because memory is primary data with concurrent agent writers.
// ---------------------------------------------------------------------------
