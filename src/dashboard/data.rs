//! Data aggregation engine for the native dashboard.
//! Queries `.aw/usage.log`, `.aw/index.db`, and `.aw/memory.db` with sub-millisecond latency.

mod memories;
mod reuse;
mod summary;

pub use memories::collect_memories;
pub use reuse::collect_reuse_candidates;
pub use summary::compute_summary;

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

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
