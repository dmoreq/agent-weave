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
    let (graph_p50, graph_p95, graph_avg) = compute_percentiles(&mut state.graph_latencies_ms);
    let tool_aggs_json = format_tool_aggs(state.tool_aggs);
    let repo_aggs_json = format_repo_aggs(state.repo_aggs);
    let agent_aggs_json = format_agent_aggs(state.agent_aggs);
    let reliability = 100.0 * (total_calls - state.errors) as f64 / total_calls as f64;
    let graph_reliability = if state.graph_calls > 0 {
        100.0 * (state.graph_calls - state.graph_errors) as f64 / state.graph_calls as f64
    } else {
        100.0
    };
    let saved_dollars = state.saved_tokens as f64 * crate::journal::PRICE_PER_TOKEN;
    let graph_saved_dollars = state.graph_saved_tokens as f64 * crate::journal::PRICE_PER_TOKEN;
    let total_sessions = state.sessions.len();
    let memory_sessions = state.memory_sessions.len();
    let session_recall_pct = if total_sessions > 0 {
        100.0 * memory_sessions as f64 / total_sessions as f64
    } else {
        0.0
    };

    let recent_queries: Vec<Value> = entries.iter().rev().take(50).cloned().collect();
    let graph_recent_queries: Vec<Value> = entries
        .iter()
        .rev()
        .filter(|e| is_graph_tool(e.get("tool").and_then(Value::as_str).unwrap_or("")))
        .take(50)
        .cloned()
        .collect();
    state.recent_errors.reverse();
    state.recent_errors.truncate(25);
    state.graph_recent_errors.reverse();
    state.graph_recent_errors.truncate(25);

    json!({
        "total_calls": total_calls, "error_count": state.errors, "warning_count": state.warnings,
        "reliability_pct": reliability, "p50_ms": p50, "p95_ms": p95, "avg_ms": avg,
        "saved_tokens": state.saved_tokens, "saved_dollars": saved_dollars,
        "graph_calls": state.graph_calls, "graph_errors": state.graph_errors,
        "graph_reliability_pct": graph_reliability, "graph_p50_ms": graph_p50,
        "graph_p95_ms": graph_p95, "graph_avg_ms": graph_avg,
        "graph_saved_tokens": state.graph_saved_tokens, "graph_saved_dollars": graph_saved_dollars,
        "reuse_hits": state.reuse_hits, "session_count": total_sessions,
        "memory_session_count": memory_sessions, "session_recall_pct": session_recall_pct,
        "tool_aggs": tool_aggs_json, "repo_aggs": repo_aggs_json, "agent_aggs": agent_aggs_json,
        "recent_queries": recent_queries, "recent_errors": state.recent_errors,
        "graph_recent_queries": graph_recent_queries, "graph_recent_errors": state.graph_recent_errors,
    })
}

fn empty_summary() -> Value {
    json!({
        "total_calls": 0, "error_count": 0, "warning_count": 0, "reliability_pct": 100.0,
        "p50_ms": 0.0, "p95_ms": 0.0, "avg_ms": 0.0, "saved_tokens": 0, "saved_dollars": 0.0,
        "graph_calls": 0, "graph_errors": 0, "graph_reliability_pct": 100.0,
        "graph_p50_ms": 0.0, "graph_p95_ms": 0.0, "graph_avg_ms": 0.0,
        "graph_saved_tokens": 0, "graph_saved_dollars": 0.0, "reuse_hits": 0,
        "session_count": 0, "memory_session_count": 0, "session_recall_pct": 0.0,
        "tool_aggs": {}, "repo_aggs": {}, "agent_aggs": {},
        "recent_queries": [], "recent_errors": [],
        "graph_recent_queries": [], "graph_recent_errors": []
    })
}

fn is_graph_tool(tool: &str) -> bool {
    matches!(
        tool,
        "callers" | "callees" | "path" | "context" | "impact" | "symbols" | "imports" | "build"
    )
}

fn is_reuse_tool(tool: &str) -> bool {
    matches!(tool, "search" | "reuse")
}

fn is_memory_tool(tool: &str) -> bool {
    tool.starts_with("mem_")
}

#[derive(Default)]
struct SummaryState {
    errors: usize,
    warnings: usize,
    reuse_hits: usize,
    saved_tokens: usize,
    latencies_ms: Vec<f64>,
    graph_calls: usize,
    graph_errors: usize,
    graph_saved_tokens: usize,
    graph_latencies_ms: Vec<f64>,
    sessions: HashSet<String>,
    memory_sessions: HashSet<String>,
    tool_aggs: HashMap<String, ToolAgg>,
    repo_aggs: HashMap<String, RepoAgg>,
    agent_aggs: HashMap<String, AgentAgg>,
    recent_errors: Vec<Value>,
    graph_recent_errors: Vec<Value>,
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

        let is_graph = is_graph_tool(tool);
        let is_reuse = is_reuse_tool(tool);
        let is_mem = is_memory_tool(tool);
        let is_mem_read = matches!(tool, "mem_query" | "mem_get");
        let is_mem_write = matches!(tool, "mem_write" | "mem_touch");
        if is_graph {
            self.graph_calls += 1;
        }

        self.sessions.insert(session_name.to_string());
        if is_mem_read {
            self.memory_sessions.insert(session_name.to_string());
        }

        let t_entry = self.tool_aggs.entry(tool.to_string()).or_default();
        t_entry.calls += 1;
        t_entry.shown += shown;

        let r_entry = self.repo_aggs.entry(repo_name.to_string()).or_default();
        r_entry.calls += 1;
        r_entry.worktree = r_entry.worktree || is_worktree;
        if ts > r_entry.last_ts {
            r_entry.last_ts = ts;
        }
        if is_graph {
            r_entry.graph_calls += 1;
        }
        if is_reuse {
            r_entry.reuse_calls += 1;
        }
        if is_mem {
            r_entry.memory_calls += 1;
        }
        if is_mem_read {
            r_entry.memory_reads += 1;
        }
        if is_mem_write {
            r_entry.memory_writes += 1;
        }

        let a_entry = self.agent_aggs.entry(agent_name.to_string()).or_default();
        a_entry.calls += 1;
        if ts > a_entry.last_ts {
            a_entry.last_ts = ts;
        }
        if is_graph {
            a_entry.graph_calls += 1;
        }
        if is_reuse {
            a_entry.reuse_calls += 1;
        }
        if is_mem {
            a_entry.memory_calls += 1;
        }
        if is_mem_read {
            a_entry.memory_reads += 1;
        }
        if is_mem_write {
            a_entry.memory_writes += 1;
        }

        match ExitClass::of(exit_code) {
            ExitClass::Error => {
                self.errors += 1;
                t_entry.errors += 1;
                r_entry.errors += 1;
                a_entry.errors += 1;
                self.recent_errors.push(e.clone());
                if is_graph {
                    self.graph_errors += 1;
                    r_entry.graph_errors += 1;
                    a_entry.graph_errors += 1;
                    self.graph_recent_errors.push(e.clone());
                }
                if is_reuse {
                    r_entry.reuse_errors += 1;
                    a_entry.reuse_errors += 1;
                }
                if is_mem {
                    r_entry.memory_errors += 1;
                    a_entry.memory_errors += 1;
                }
            }
            ExitClass::Empty | ExitClass::Ambiguous => {
                self.warnings += 1;
            }
            ExitClass::Ok => {
                let saved = crate::journal::estimated_tokens_saved(tool, shown, result_count, 1);
                let reuse_hit = if tool == "reuse" {
                    shown.max(result_count)
                } else {
                    0
                };
                if reuse_hit > 0 {
                    self.reuse_hits += reuse_hit;
                    r_entry.reuse_hits += reuse_hit;
                    a_entry.reuse_hits += reuse_hit;
                }
                self.saved_tokens += saved;
                r_entry.tokens_saved += saved;
                a_entry.tokens_saved += saved;
                if is_graph {
                    self.graph_saved_tokens += saved;
                    r_entry.graph_tokens_saved += saved;
                    a_entry.graph_tokens_saved += saved;
                }
                if is_reuse {
                    r_entry.reuse_tokens_saved += saved;
                    a_entry.reuse_tokens_saved += saved;
                }
            }
        }

        if let Some(us) = lat_us {
            if us >= 0.0 {
                let ms = us / 1000.0;
                self.latencies_ms.push(ms);
                if is_graph {
                    self.graph_latencies_ms.push(ms);
                }
                t_entry.latencies.push(ms);
            }
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

fn rel_pct(errors: usize, calls: usize) -> f64 {
    if calls > 0 {
        100.0 * (calls - errors) as f64 / calls as f64
    } else {
        100.0
    }
}

fn format_repo_aggs(repo_aggs: HashMap<String, RepoAgg>) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for (k, v) in repo_aggs {
        out.insert(
            k,
            json!({
                "calls": v.calls,
                "errors": v.errors,
                "graph_calls": v.graph_calls,
                "graph_errors": v.graph_errors,
                "worktree": v.worktree,
                "last_ts": v.last_ts,
                "reliability_pct": rel_pct(v.errors, v.calls),
                "graph_reliability_pct": rel_pct(v.graph_errors, v.graph_calls),
                "saved_tokens": v.tokens_saved,
                "graph_saved_tokens": v.graph_tokens_saved,
                "reuse_calls": v.reuse_calls,
                "reuse_errors": v.reuse_errors,
                "reuse_reliability_pct": rel_pct(v.reuse_errors, v.reuse_calls),
                "reuse_hits": v.reuse_hits,
                "reuse_saved_tokens": v.reuse_tokens_saved,
                "memory_calls": v.memory_calls,
                "memory_errors": v.memory_errors,
                "memory_reliability_pct": rel_pct(v.memory_errors, v.memory_calls),
                "memory_reads": v.memory_reads,
                "memory_writes": v.memory_writes,
            }),
        );
    }
    out
}

fn format_agent_aggs(agent_aggs: HashMap<String, AgentAgg>) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for (k, v) in agent_aggs {
        out.insert(
            k,
            json!({
                "calls": v.calls,
                "errors": v.errors,
                "graph_calls": v.graph_calls,
                "graph_errors": v.graph_errors,
                "last_ts": v.last_ts,
                "reliability_pct": rel_pct(v.errors, v.calls),
                "graph_reliability_pct": rel_pct(v.graph_errors, v.graph_calls),
                "saved_tokens": v.tokens_saved,
                "graph_saved_tokens": v.graph_tokens_saved,
                "reuse_calls": v.reuse_calls,
                "reuse_errors": v.reuse_errors,
                "reuse_reliability_pct": rel_pct(v.reuse_errors, v.reuse_calls),
                "reuse_hits": v.reuse_hits,
                "reuse_saved_tokens": v.reuse_tokens_saved,
                "memory_calls": v.memory_calls,
                "memory_errors": v.memory_errors,
                "memory_reliability_pct": rel_pct(v.memory_errors, v.memory_calls),
                "memory_reads": v.memory_reads,
                "memory_writes": v.memory_writes,
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
    graph_calls: usize,
    graph_errors: usize,
    worktree: bool,
    tokens_saved: usize,
    graph_tokens_saved: usize,
    reuse_calls: usize,
    reuse_errors: usize,
    reuse_hits: usize,
    reuse_tokens_saved: usize,
    memory_calls: usize,
    memory_errors: usize,
    memory_reads: usize,
    memory_writes: usize,
    last_ts: f64,
}

#[derive(Default)]
struct AgentAgg {
    calls: usize,
    errors: usize,
    graph_calls: usize,
    graph_errors: usize,
    tokens_saved: usize,
    graph_tokens_saved: usize,
    reuse_calls: usize,
    reuse_errors: usize,
    reuse_hits: usize,
    reuse_tokens_saved: usize,
    memory_calls: usize,
    memory_errors: usize,
    memory_reads: usize,
    memory_writes: usize,
    last_ts: f64,
}
