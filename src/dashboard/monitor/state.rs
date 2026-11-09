//! In-memory state and metrics aggregation for the interactive TUI monitor.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::dashboard::data;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorTab {
    Graph,
    Reuse,
    Memory,
    Stream,
}

impl MonitorTab {
    pub fn index(&self) -> usize {
        match self {
            Self::Graph => 1,
            Self::Reuse => 2,
            Self::Memory => 3,
            Self::Stream => 4,
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::Graph => "1 Graph",
            Self::Reuse => "2 Anti-Reinvent",
            Self::Memory => "3 Memory",
            Self::Stream => "4 Stream",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Graph => Self::Reuse,
            Self::Reuse => Self::Memory,
            Self::Memory => Self::Stream,
            Self::Stream => Self::Graph,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            Self::Graph => Self::Stream,
            Self::Reuse => Self::Graph,
            Self::Memory => Self::Reuse,
            Self::Stream => Self::Memory,
        }
    }
}

pub struct TuiState {
    pub active_tab: MonitorTab,
    pub log_path: PathBuf,
    pub repo_filter: Option<String>,
    pub errors_only: bool,
    pub summary: Value,
    pub reuse_candidates: Vec<Value>,
    pub reuse_summary: Value,
    pub memories: Vec<Value>,
    pub recent_events: Vec<Value>,
    pub total_repos: usize,
}

impl TuiState {
    pub fn new(
        root: Option<&Path>,
        log_path: PathBuf,
        repo_filter: Option<String>,
        errors_only: bool,
    ) -> Self {
        let mut state = Self {
            active_tab: MonitorTab::Graph,
            log_path,
            repo_filter,
            errors_only,
            summary: json!({}),
            reuse_candidates: Vec::new(),
            reuse_summary: json!({}),
            memories: Vec::new(),
            recent_events: Vec::new(),
            total_repos: 0,
        };
        state.refresh(root);
        state
    }

    pub fn refresh(&mut self, root: Option<&Path>) {
        let entries = data::read_usage_entries(&self.log_path);
        self.summary = data::compute_summary(&entries);

        if let Ok((candidates, summary, _)) = data::collect_reuse_candidates(root, &entries) {
            self.reuse_candidates = candidates;
            self.reuse_summary = summary;
        }

        if let Ok(notes) = data::collect_memories(root, &entries) {
            self.memories = notes;
        }

        self.total_repos = self
            .summary
            .get("repo_aggs")
            .and_then(Value::as_object)
            .map(|m| m.len())
            .unwrap_or(0);

        self.recent_events.clear();
        for e in entries.iter().rev().take(100) {
            if self.matches_filter(e) {
                self.recent_events.push(e.clone());
            }
        }
    }

    pub fn matches_filter(&self, val: &Value) -> bool {
        if self.errors_only {
            let code = val.get("exit_code").and_then(Value::as_i64).unwrap_or(0);
            if code < 3 {
                return false;
            }
        }

        if let Some(ref rf) = self.repo_filter {
            if rf != "ALL" {
                let r_name = val
                    .get("repo_name")
                    .and_then(Value::as_str)
                    .unwrap_or("workspace");
                if r_name != rf {
                    return false;
                }
            }
        }

        true
    }

    pub fn push_event(&mut self, val: Value) {
        if self.matches_filter(&val) {
            self.recent_events.insert(0, val);
            if self.recent_events.len() > 200 {
                self.recent_events.pop();
            }
        }
    }
}

pub fn classify_usp_pillar(tool: &str) -> (&'static str, &'static str) {
    if tool.starts_with("mem_") {
        ("MEMORY", "\x1b[35m")
    } else if tool == "search" || tool == "reuse" {
        ("REUSE", "\x1b[32m")
    } else {
        ("GRAPH", "\x1b[36m")
    }
}

pub fn format_timestamp(ts: i64) -> String {
    if ts <= 0 {
        return "--:--:--".to_string();
    }
    let secs = (ts % 86400) as u32;
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{hours:02}:{mins:02}:{s:02}")
}

pub fn truncate_chars(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((byte_offset, _)) => &s[..byte_offset],
        None => s,
    }
}
