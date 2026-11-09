//! Usage-log summary aggregation: reliability, latency percentiles, and
//! token-savings accounting folded per tool and repository.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{json, Value};

use crate::error::ExitClass;

pub fn compute_summary(entries: &[Value]) -> Value {
    let total_calls = entries.len();
    if total_calls == 0 {
        return empty_summary();
    }

    let mut state = SummaryState::with_capacity(entries.len());
    for e in entries {
        state.record_entry(e);
    }

    let (p50, p95, avg) = compute_percentiles(&mut state.latencies_ms);
    let tool_aggs_json = format_tool_aggs(state.tool_aggs);
    let repo_aggs_json = format_repo_aggs(state.repo_aggs);
    let agent_aggs_json = format_agent_aggs(state.agent_aggs);
    let reliability = 100.0 * (total_calls - state.errors) as f64 / total_calls as f64;
    let saved_dollars = state.saved_tokens as f64 * crate::journal::PRICE_PER_TOKEN;
    let total_sessions = state.sessions.len();
    let memory_sessions = state.memory_sessions.len();
    let session_recall_pct = if total_sessions > 0 {
        100.0 * memory_sessions as f64 / total_sessions as f64
    } else {
        0.0
    };

    let recent_queries: Vec<Value> = entries.iter().rev().take(50).cloned().collect();
    state.recent_errors.reverse();
    state.recent_errors.truncate(25);

    json!({
        "total_calls": total_calls,
        "error_count": state.errors,
        "warning_count": state.warnings,
        "reliability_pct": reliability,
        "p50_ms": p50,
        "p95_ms": p95,
        "avg_ms": avg,
        "saved_tokens": state.saved_tokens,
        "saved_dollars": saved_dollars,
        "reuse_hits": state.reuse_hits,
        "session_count": total_sessions,
        "memory_session_count": memory_sessions,
        "session_recall_pct": session_recall_pct,
        "tool_aggs": tool_aggs_json,
        "repo_aggs": repo_aggs_json,
        "agent_aggs": agent_aggs_json,
        "recent_queries": recent_queries,
        "recent_errors": state.recent_errors,
    })
}

fn empty_summary() -> Value {
    json!({
        "total_calls": 0,
        "error_count": 0,
        "warning_count": 0,
        "reliability_pct": 100.0,
        "p50_ms": 0.0,
        "p95_ms": 0.0,
        "avg_ms": 0.0,
        "saved_tokens": 0,
        "saved_dollars": 0.0,
        "reuse_hits": 0,
        "session_count": 0,
        "memory_session_count": 0,
        "session_recall_pct": 0.0,
        "tool_aggs": {},
        "repo_aggs": {},
        "agent_aggs": {},
        "recent_queries": [],
        "recent_errors": []
    })
}

#[derive(Default)]
struct SummaryState {
    errors: usize,
    warnings: usize,
    reuse_hits: usize,
    saved_tokens: usize,
    latencies_ms: Vec<f64>,
    sessions: HashSet<String>,
    memory_sessions: HashSet<String>,
    tool_aggs: HashMap<String, ToolAgg>,
    repo_aggs: HashMap<String, RepoAgg>,
    agent_aggs: HashMap<String, AgentAgg>,
    recent_errors: Vec<Value>,
}

impl SummaryState {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            latencies_ms: Vec::with_capacity(capacity),
            ..Default::default()
        }
    }

    fn record_entry(&mut self, e: &Value) {
        let tool = e.get("tool").and_then(Value::as_str).unwrap_or("unknown");
        let exit_code = e.get("exit_code").and_then(Value::as_i64).unwrap_or(0);
        let shown = e
            .get("shown")
            .or_else(|| e.get("result_count"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        let result_count = e.get("result_count").and_then(Value::as_u64).unwrap_or(0) as usize;
        let lat_us = e.get("latency_us").and_then(Value::as_f64);
        let ts = e.get("ts").and_then(Value::as_f64).unwrap_or(0.0);
        let repo_name = e
            .get("repo_name")
            .and_then(Value::as_str)
            .unwrap_or("workspace");
        let is_worktree = e.get("worktree").and_then(Value::as_bool).unwrap_or(false);
        let agent_name = e
            .get("agent")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("user");
        let session_name = e
            .get("session")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("default");

        self.sessions.insert(session_name.to_string());
        if tool == "mem_query" || tool == "mem_get" {
            self.memory_sessions.insert(session_name.to_string());
        }

        let t_entry = self.tool_aggs.entry(tool.to_string()).or_default();
        t_entry.calls += 1;
        t_entry.shown += shown;

        let r_entry = self.repo_aggs.entry(repo_name.to_string()).or_default();
        r_entry.calls += 1;
        r_entry.worktree = is_worktree;
        if ts > r_entry.last_ts {
            r_entry.last_ts = ts;
        }

        let a_entry = self.agent_aggs.entry(agent_name.to_string()).or_default();
        a_entry.calls += 1;
        if ts > a_entry.last_ts {
            a_entry.last_ts = ts;
        }

        self.record_exit_status(exit_code, e, tool, repo_name, agent_name);

        if let Some(us) = lat_us {
            if us >= 0.0 {
                let ms = us / 1000.0;
                self.latencies_ms.push(ms);
                self.tool_aggs
                    .entry(tool.to_string())
                    .or_default()
                    .latencies
                    .push(ms);
            }
        }

        if exit_code == 0 {
            if tool == "reuse" {
                self.reuse_hits += shown.max(result_count);
            }
            let saved = crate::journal::estimated_tokens_saved(tool, shown, result_count, 1);
            self.saved_tokens += saved;
            self.repo_aggs
                .entry(repo_name.to_string())
                .or_default()
                .tokens_saved += saved;
            self.agent_aggs
                .entry(agent_name.to_string())
                .or_default()
                .tokens_saved += saved;
        }
    }

    fn record_exit_status(
        &mut self,
        exit_code: i64,
        e: &Value,
        tool: &str,
        repo: &str,
        agent: &str,
    ) {
        let t_entry = self.tool_aggs.entry(tool.to_string()).or_default();
        let r_entry = self.repo_aggs.entry(repo.to_string()).or_default();
        let a_entry = self.agent_aggs.entry(agent.to_string()).or_default();
        match ExitClass::of(exit_code) {
            ExitClass::Error => {
                self.errors += 1;
                t_entry.errors += 1;
                r_entry.errors += 1;
                a_entry.errors += 1;
                self.recent_errors.push(e.clone());
            }
            ExitClass::Empty | ExitClass::Ambiguous => {
                self.warnings += 1;
            }
            ExitClass::Ok => {}
        }
    }
}

fn compute_percentiles(latencies: &mut [f64]) -> (f64, f64, f64) {
    if latencies.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = latencies.len();
    let p50 = latencies[(0.50 * n as f64).ceil() as usize - 1];
    let p95 = latencies[(0.95 * n as f64).ceil() as usize - 1];
    let avg = latencies.iter().sum::<f64>() / n as f64;
    (p50, p95, avg)
}

fn format_tool_aggs(tool_aggs: HashMap<String, ToolAgg>) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for (k, mut v) in tool_aggs {
        v.latencies
            .sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let vn = v.latencies.len();
        let p50_t = if vn > 0 {
            v.latencies[(0.50 * vn as f64).ceil() as usize - 1]
        } else {
            0.0
        };
        out.insert(
            k,
            json!({
                "calls": v.calls,
                "errors": v.errors,
                "shown": v.shown,
                "p50": p50_t,
            }),
        );
    }
    out
}

fn format_repo_aggs(repo_aggs: HashMap<String, RepoAgg>) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for (k, v) in repo_aggs {
        let rel = if v.calls > 0 {
            100.0 * (v.calls - v.errors) as f64 / v.calls as f64
        } else {
            100.0
        };
        out.insert(
            k,
            json!({
                "calls": v.calls,
                "errors": v.errors,
                "worktree": v.worktree,
                "last_ts": v.last_ts,
                "reliability_pct": rel,
                "saved_tokens": v.tokens_saved,
            }),
        );
    }
    out
}

fn format_agent_aggs(agent_aggs: HashMap<String, AgentAgg>) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for (k, v) in agent_aggs {
        let rel = if v.calls > 0 {
            100.0 * (v.calls - v.errors) as f64 / v.calls as f64
        } else {
            100.0
        };
        out.insert(
            k,
            json!({
                "calls": v.calls,
                "errors": v.errors,
                "last_ts": v.last_ts,
                "reliability_pct": rel,
                "saved_tokens": v.tokens_saved,
            }),
        );
    }
    out
}

#[derive(Default)]
struct ToolAgg {
    calls: usize,
    errors: usize,
    shown: usize,
    latencies: Vec<f64>,
}

#[derive(Default)]
struct RepoAgg {
    calls: usize,
    errors: usize,
    worktree: bool,
    tokens_saved: usize,
    last_ts: f64,
}

#[derive(Default)]
struct AgentAgg {
    calls: usize,
    errors: usize,
    tokens_saved: usize,
    last_ts: f64,
}
