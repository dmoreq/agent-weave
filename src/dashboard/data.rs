//! Data aggregation engine for the native dashboard.
//! Queries `.aw/usage.log`, `.aw/index.db`, and `.aw/memory.db` with sub-millisecond latency.

mod memories;
mod reuse;
mod summary;

pub use memories::collect_memories;
pub use reuse::collect_reuse_candidates;
pub use summary::compute_summary;

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Discover candidate repo roots for a dashboard panel into `repos`. An
/// explicit root short-circuits the scan (its entry replaces any same-named
/// one); otherwise every journaled repo with `.aw/<db_rel>` present is added
/// without displacing existing entries, and the current directory joins last
/// as "workspace". Shared by the reuse and memories panels so their store
/// discovery cannot drift.
fn discover_repos(
    root: Option<&Path>,
    entries: &[Value],
    db_rel: &str,
    repos: &mut HashMap<String, PathBuf>,
) {
    if let Some(r) = root {
        let db_path = r.join(".aw").join(db_rel);
        if db_path.is_file() {
            let name = r.file_name().and_then(|n| n.to_str()).unwrap_or("repo");
            repos.insert(name.to_string(), r.to_path_buf());
        }
    } else {
        for e in entries {
            if let (Some(r), Some(r_name)) = (
                e.get("repo").and_then(Value::as_str),
                e.get("repo_name").and_then(Value::as_str),
            ) {
                let p = PathBuf::from(r);
                if p.join(".aw").join(db_rel).is_file() && !repos.contains_key(r_name) {
                    repos.insert(r_name.to_string(), p);
                }
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            let name = cwd
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("workspace");
            if cwd.join(".aw").join(db_rel).is_file() && !repos.contains_key(name) {
                repos.insert(name.to_string(), cwd);
            }
        }
    }
}

pub struct AggregatedData {
    pub summary: Value,
    pub reuse: Value,
    pub memories: Value,
}

pub fn read_usage_entries(log_path: &Path) -> Vec<Value> {
    let mut entries = Vec::new();
    let parent = log_path.parent().unwrap_or_else(|| Path::new("."));

    for filename in &["usage.log.1", "usage.log"] {
        let p = parent.join(filename);
        if let Ok(file) = File::open(&p) {
            let reader = BufReader::new(file);
            for line in reader.lines().map_while(std::result::Result::ok) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
                    let is_worktree = val
                        .get("worktree")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    if is_worktree {
                        if let Some(repo) = val.get("repo").and_then(Value::as_str) {
                            if !Path::new(repo).exists() {
                                continue;
                            }
                        }
                    }
                    entries.push(val);
                }
            }
        }
    }
    entries
}

pub fn read_snippet(file_path: &Path, line_start: i64, line_end: i64, max_lines: usize) -> String {
    if !file_path.is_file() {
        return String::new();
    }
    if let Ok(file) = File::open(file_path) {
        let reader = BufReader::new(file);
        let start = (line_start.max(1) - 1) as usize;
        let end = (line_end.max(line_start) as usize).max(start + 1);

        let lines: Vec<String> = reader.lines().map_while(std::result::Result::ok).collect();
        if start >= lines.len() {
            return String::new();
        }

        let slice_end = end.min(lines.len());
        let sub = &lines[start..slice_end];
        let mut out = Vec::new();
        for (idx, line) in sub.iter().take(max_lines).enumerate() {
            out.push(format!("{:4} | {}", line_start as usize + idx, line));
        }
        if sub.len() > max_lines {
            out.push(format!(" ... ({} more lines)", sub.len() - max_lines));
        }
        return out.join("\n");
    }
    String::new()
}
