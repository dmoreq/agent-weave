//! Parsed usage-line fields. Kept separate so `render.rs` stays under the
//! module-size cap.

use serde_json::Value;

use super::reuse::{self, ReuseUsage};

/// Parsed usage line; serde_json handles quotes, backslashes, and controls.
pub(crate) struct UsageFields {
    pub(crate) tool: String,
    pub(crate) exit_code: i64,
    pub(crate) result_count: i64,
    /// `None` distinguishes a legacy line with no `shown` key from a real 0.
    pub(crate) shown: Option<i64>,
    pub(crate) total: i64,
    pub(crate) stale: bool,
    pub(crate) ambiguous: i64,
    pub(crate) latency_us: Option<i64>,
    pub(crate) ts: Option<i64>,
    pub(crate) repo: Option<String>,
    pub(crate) repo_name: Option<String>,
    pub(crate) worktree: bool,
    pub(crate) agent: Option<String>,
    pub(crate) mode: Option<String>,
    pub(crate) truncated: bool,
    pub(crate) reuse: Option<ReuseUsage>,
    pub(crate) freshness_state: Option<String>,
    pub(crate) freshness_unknown: usize,
}

pub(crate) fn parse_usage_line(line: &str) -> Option<UsageFields> {
    let v: Value = serde_json::from_str(line).ok()?;
    Some(UsageFields {
        tool: v.get("tool")?.as_str()?.to_string(),
        exit_code: v.get("exit_code").and_then(|x| x.as_i64()).unwrap_or(0),
        result_count: v.get("result_count").and_then(|x| x.as_i64()).unwrap_or(0),
        shown: v.get("shown").and_then(|x| x.as_i64()),
        total: v.get("total").and_then(|x| x.as_i64()).unwrap_or(0),
        stale: v.get("stale").and_then(|x| x.as_bool()).unwrap_or(false),
        ambiguous: v.get("ambiguous").and_then(|x| x.as_i64()).unwrap_or(0),
        latency_us: v.get("latency_us").and_then(|x| x.as_i64()),
        ts: v.get("ts").and_then(|x| x.as_i64()),
        repo: v
            .get("repo")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
        repo_name: v
            .get("repo_name")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
        worktree: v.get("worktree").and_then(|x| x.as_bool()).unwrap_or(false),
        agent: v
            .get("agent")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
        mode: v
            .get("mode")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
        truncated: v
            .get("truncated")
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
        reuse: v.get("reuse").and_then(reuse::parse),
        freshness_state: v
            .get("freshness")
            .and_then(|f| f.get("state"))
            .and_then(Value::as_str)
            .map(str::to_string),
        freshness_unknown: v
            .get("freshness")
            .and_then(|f| f.get("unknown_count"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize,
    })
}
