//! Usage-log summary aggregation: reliability, latency percentiles, and
//! token-savings accounting folded per tool and repository.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{json, Value};

use crate::error::ExitClass;

pub fn compute_summary(entries: &[Value]) -> Value {
    if entries.is_empty() {
        return empty_summary();
    }
    let mut state = SummaryState::with_capacity(entries.len());
    for e in entries {
        state.record_entry(e);
    }
    summary_json(state, entries)
}

/// Assemble the dashboard payload: percentiles, reliability, token savings,
/// session recall, the recent windows, and the per-dimension rollups.
fn summary_json(mut state: SummaryState, entries: &[Value]) -> Value {
    let total_calls = entries.len();
    let (p50, p95, avg) = compute_percentiles(&mut state.latencies_ms);
    let (graph_p50, graph_p95, graph_avg) = compute_percentiles(&mut state.graph_latencies_ms);
    let (recent_queries, graph_recent_queries) = recent_windows(&mut state, entries);
    let tool_aggs_json = format_tool_aggs(state.tool_aggs);
    let repo_aggs_json = format_aggs(state.repo_aggs, true);
    let agent_aggs_json = format_aggs(state.agent_aggs, false);
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

    json!({
        "total_calls": total_calls, "error_count": state.errors, "warning_count": state.warnings,
        "reliability_pct": reliability, "p50_ms": p50, "p95_ms": p95, "avg_ms": avg,
        "saved_tokens": state.saved_tokens, "saved_dollars": saved_dollars,
        "graph_calls": state.graph_calls, "graph_errors": state.graph_errors,
        "graph_reliability_pct": graph_reliability, "graph_p50_ms": graph_p50,
        "graph_p95_ms": graph_p95, "graph_avg_ms": graph_avg,
        "graph_saved_tokens": state.graph_saved_tokens, "graph_saved_dollars": graph_saved_dollars,
        "reuse_hits": state.reuse_hits, "judged_calls": state.judged_calls,
        "session_count": total_sessions,
        "memory_session_count": memory_sessions, "session_recall_pct": session_recall_pct,
        "tool_aggs": tool_aggs_json, "repo_aggs": repo_aggs_json, "agent_aggs": agent_aggs_json,
        "recent_queries": recent_queries, "recent_errors": state.recent_errors,
        "graph_recent_queries": graph_recent_queries, "graph_recent_errors": state.graph_recent_errors,
    })
}

/// The recent query windows (all tools, then graph tools only) plus the
/// bounded error windows, newest first.
fn recent_windows(state: &mut SummaryState, entries: &[Value]) -> (Vec<Value>, Vec<Value>) {
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
    (recent_queries, graph_recent_queries)
}

fn empty_summary() -> Value {
    json!({
        "total_calls": 0, "error_count": 0, "warning_count": 0, "reliability_pct": 100.0,
        "p50_ms": 0.0, "p95_ms": 0.0, "avg_ms": 0.0, "saved_tokens": 0, "saved_dollars": 0.0,
        "graph_calls": 0, "graph_errors": 0, "graph_reliability_pct": 100.0,
        "graph_p50_ms": 0.0, "graph_p95_ms": 0.0, "graph_avg_ms": 0.0,
        "graph_saved_tokens": 0, "graph_saved_dollars": 0.0, "reuse_hits": 0, "judged_calls": 0,
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

/// One parsed usage entry: every field the record phases need, plus the
/// tool's dimension flags. Parsing lives here alone so the fallback defaults
/// cannot drift between phases.
struct EntryView<'a> {
    tool: &'a str,
    exit_code: i64,
    shown: usize,
    result_count: usize,
    lat_us: Option<f64>,
    ts: f64,
    repo_name: &'a str,
    is_worktree: bool,
    agent_name: &'a str,
    session_name: &'a str,
    judged: bool,
    facts: EntryFacts,
}

impl<'a> EntryView<'a> {
    fn parse(e: &'a Value) -> EntryView<'a> {
        let tool = e.get("tool").and_then(Value::as_str).unwrap_or("unknown");
        EntryView {
            tool,
            exit_code: e.get("exit_code").and_then(Value::as_i64).unwrap_or(0),
            shown: e
                .get("shown")
                .or_else(|| e.get("result_count"))
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize,
            result_count: e.get("result_count").and_then(Value::as_u64).unwrap_or(0) as usize,
            lat_us: e.get("latency_us").and_then(Value::as_f64),
            ts: e.get("ts").and_then(Value::as_f64).unwrap_or(0.0),
            repo_name: e
                .get("repo_name")
                .and_then(Value::as_str)
                .unwrap_or("workspace"),
            is_worktree: e.get("worktree").and_then(Value::as_bool).unwrap_or(false),
            agent_name: e
                .get("agent")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("user"),
            session_name: e
                .get("session")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("default"),
            judged: e.get("judged").and_then(Value::as_bool).unwrap_or(false),
            facts: EntryFacts {
                is_graph: is_graph_tool(tool),
                is_reuse: is_reuse_tool(tool),
                is_mem: is_memory_tool(tool),
                is_mem_read: matches!(tool, "mem_query" | "mem_get"),
                is_mem_write: matches!(tool, "mem_write" | "mem_touch"),
            },
        }
    }
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
    agent_aggs: HashMap<String, RepoAgg>,
    recent_errors: Vec<Value>,
    graph_recent_errors: Vec<Value>,
    judged_calls: usize,
}

impl SummaryState {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            latencies_ms: Vec::with_capacity(capacity),
            ..Default::default()
        }
    }

    fn record_entry(&mut self, e: &Value) {
        let v = EntryView::parse(e);
        if v.judged {
            self.judged_calls += 1;
        }
        self.record_session(&v);
        self.record_calls(&v);
        self.record_outcome(&v, e);
        self.record_latency(&v);
    }

    fn tool_mut(&mut self, tool: &str) -> &mut ToolAgg {
        self.tool_aggs.entry(tool.to_string()).or_default()
    }

    fn repo_mut(&mut self, repo: &str) -> &mut RepoAgg {
        self.repo_aggs.entry(repo.to_string()).or_default()
    }

    fn agent_mut(&mut self, agent: &str) -> &mut RepoAgg {
        self.agent_aggs.entry(agent.to_string()).or_default()
    }

    fn record_session(&mut self, v: &EntryView<'_>) {
        if v.facts.is_graph {
            self.graph_calls += 1;
        }
        self.sessions.insert(v.session_name.to_string());
        if v.facts.is_mem_read {
            self.memory_sessions.insert(v.session_name.to_string());
        }
    }

    fn record_calls(&mut self, v: &EntryView<'_>) {
        let t = self.tool_mut(v.tool);
        t.calls += 1;
        t.shown += v.shown;
        let r = self.repo_mut(v.repo_name);
        record_entry_into(r, v.facts, v.ts, v.is_worktree);
        let a = self.agent_mut(v.agent_name);
        record_entry_into(a, v.facts, v.ts, false);
    }

    fn record_outcome(&mut self, v: &EntryView<'_>, e: &Value) {
        match ExitClass::of(v.exit_code) {
            ExitClass::Error => self.record_error(v, e),
            ExitClass::Empty | ExitClass::Ambiguous => self.record_warning(),
            ExitClass::Ok => self.record_success(v),
        }
    }

    fn record_warning(&mut self) {
        self.warnings += 1;
    }

    fn record_error(&mut self, v: &EntryView<'_>, e: &Value) {
        self.errors += 1;
        self.recent_errors.push(e.clone());
        self.tool_mut(v.tool).errors += 1;
        self.repo_mut(v.repo_name).errors += 1;
        self.agent_mut(v.agent_name).errors += 1;
        if v.facts.is_graph {
            self.graph_errors += 1;
            self.graph_recent_errors.push(e.clone());
            self.repo_mut(v.repo_name).graph_errors += 1;
            self.agent_mut(v.agent_name).graph_errors += 1;
        }
        if v.facts.is_reuse {
            self.repo_mut(v.repo_name).reuse_errors += 1;
            self.agent_mut(v.agent_name).reuse_errors += 1;
        }
        if v.facts.is_mem {
            self.repo_mut(v.repo_name).memory_errors += 1;
            self.agent_mut(v.agent_name).memory_errors += 1;
        }
    }

    fn record_success(&mut self, v: &EntryView<'_>) {
        let saved = crate::journal::estimated_tokens_saved(v.tool, v.shown, v.result_count, 1);
        let reuse_hit = if v.tool == "reuse" {
            v.shown.max(v.result_count)
        } else {
            0
        };
        if reuse_hit > 0 {
            self.reuse_hits += reuse_hit;
            self.repo_mut(v.repo_name).reuse_hits += reuse_hit;
            self.agent_mut(v.agent_name).reuse_hits += reuse_hit;
        }
        self.saved_tokens += saved;
        self.repo_mut(v.repo_name).tokens_saved += saved;
        self.agent_mut(v.agent_name).tokens_saved += saved;
        if v.facts.is_graph {
            self.graph_saved_tokens += saved;
            self.repo_mut(v.repo_name).graph_tokens_saved += saved;
            self.agent_mut(v.agent_name).graph_tokens_saved += saved;
        }
        if v.facts.is_reuse {
            self.repo_mut(v.repo_name).reuse_tokens_saved += saved;
            self.agent_mut(v.agent_name).reuse_tokens_saved += saved;
        }
    }

    fn record_latency(&mut self, v: &EntryView<'_>) {
        let Some(us) = v.lat_us else {
            return;
        };
        if us < 0.0 {
            return;
        }
        let ms = us / 1000.0;
        self.latencies_ms.push(ms);
        if v.facts.is_graph {
            self.graph_latencies_ms.push(ms);
        }
        self.tool_mut(v.tool).latencies.push(ms);
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

fn format_aggs(aggs: HashMap<String, RepoAgg>, with_worktree: bool) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for (k, v) in aggs {
        let mut entry = json!({
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
        });
        if with_worktree {
            entry["worktree"] = json!(v.worktree);
        }
        out.insert(k, entry);
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

/// Dimension counters shared by the repo and agent rollups, so the two
/// aggregates cannot drift.
#[derive(Clone, Copy)]
struct EntryFacts {
    is_graph: bool,
    is_reuse: bool,
    is_mem: bool,
    is_mem_read: bool,
    is_mem_write: bool,
}

fn record_entry_into(agg: &mut RepoAgg, facts: EntryFacts, ts: f64, worktree: bool) {
    agg.calls += 1;
    agg.worktree = agg.worktree || worktree;
    if ts > agg.last_ts {
        agg.last_ts = ts;
    }
    if facts.is_graph {
        agg.graph_calls += 1;
    }
    if facts.is_reuse {
        agg.reuse_calls += 1;
    }
    if facts.is_mem {
        agg.memory_calls += 1;
    }
    if facts.is_mem_read {
        agg.memory_reads += 1;
    }
    if facts.is_mem_write {
        agg.memory_writes += 1;
    }
}
