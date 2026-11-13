//! Dashboard payload assembly: field set, provenance markers, and the recent
//! graph window. Split out of `summary.rs` to stay inside the 500-LOC gate.

use super::*;

/// Assemble the dashboard payload: percentiles, reliability, token savings,
/// session recall, the graph event window, and the per-dimension rollups.
/// Field set and units are documented in `docs/agent-integration.md`; the
/// golden key-set test in `src/dashboard.rs` pins presence and absence.
pub(super) fn summary_json(mut state: SummaryState, entries: &[Value]) -> Value {
    // Only entries that survived the live-repository guard are counted; the
    // raw slice can carry skipped lines when a caller feeds it directly.
    let total_calls = state.recorded;
    let (p50, p95, avg) = compute_percentiles(&mut state.latencies_ms);
    let (graph_p50, graph_p95, graph_avg) = compute_percentiles(&mut state.graph_latencies_ms);
    let graph_recent_queries = recent_graph_window(entries);
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
        "schema_version": 2,
        "generated_at": crate::db::unix_now(),
        // Readers cover the current log plus its one rotated generation, so a
        // lifetime label would overstate the window after rotation.
        "window": "retained_journal",
        "total_calls": total_calls, "error_count": state.errors,
        "reliability_pct": reliability, "p50_ms": p50, "p95_ms": p95, "avg_ms": avg,
        "saved_tokens": state.saved_tokens, "saved_dollars": saved_dollars,
        "saved_tokens_estimated": true, "saved_dollars_estimated": true,
        "graph_calls": state.graph_calls, "graph_errors": state.graph_errors,
        "graph_reliability_pct": graph_reliability, "graph_p50_ms": graph_p50,
        "graph_p95_ms": graph_p95, "graph_avg_ms": graph_avg,
        "graph_saved_tokens": state.graph_saved_tokens, "graph_saved_dollars": graph_saved_dollars,
        "graph_saved_tokens_estimated": true, "graph_saved_dollars_estimated": true,
        "session_count": total_sessions,
        "memory_session_count": memory_sessions, "session_recall_pct": session_recall_pct,
        "tool_aggs": tool_aggs_json, "repo_aggs": repo_aggs_json, "agent_aggs": agent_aggs_json,
        "graph_recent_queries": graph_recent_queries,
    })
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
        let p95_t = if vn > 0 {
            v.latencies[(0.95 * vn as f64).ceil() as usize - 1]
        } else {
            0.0
        };
        // Success-only inputs: the fold prices a call exactly as the global
        // KPI does, so `sum(tool saved_tokens) == saved_tokens` always holds.
        let saved_tokens =
            crate::journal::estimated_tokens_saved(&k, v.ok_shown, v.ok_results, v.ok_calls);
        out.insert(
            k,
            json!({
                "calls": v.calls,
                "errors": v.errors,
                "shown": v.shown,
                "p50": p50_t,
                "p95": p95_t,
                "saved_tokens": saved_tokens,
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
            "display": v.display,
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
            // Linked worktrees fold into their main repository's row; the
            // per-worktree breakdown feeds the dashboard's expandable
            // sub-rows. Agent rows carry no breakdown.
            if !v.worktrees.is_empty() {
                let worktrees: BTreeMap<&str, Value> = v
                    .worktrees
                    .iter()
                    .map(|(name, w)| {
                        (
                            name.as_str(),
                            json!({
                                "calls": w.calls,
                                "errors": w.errors,
                                "last_ts": w.last_ts,
                            }),
                        )
                    })
                    .collect();
                entry["worktrees"] = json!(worktrees);
            }
        }
        out.insert(k, entry);
    }
    out
}

/// The graph event window, newest first. Only graph tools are rendered, so no
/// all-tools twin is produced. Linked-worktree rows are stamped with their
/// main repository so the feed can note the provenance.
fn recent_graph_window(entries: &[Value]) -> Vec<Value> {
    entries
        .iter()
        .rev()
        .filter(|e| is_graph_tool(e.get("tool").and_then(Value::as_str).unwrap_or("")))
        .take(50)
        .map(|e| {
            let mut row = e.clone();
            crate::dashboard::data::annotate_worktree_main(&mut row);
            row
        })
        .collect()
}

pub(super) fn empty_summary() -> Value {
    json!({
        "schema_version": 2, "generated_at": crate::db::unix_now(),
        "window": "retained_journal",
        "total_calls": 0, "error_count": 0, "reliability_pct": 100.0,
        "p50_ms": 0.0, "p95_ms": 0.0, "avg_ms": 0.0, "saved_tokens": 0, "saved_dollars": 0.0,
        "saved_tokens_estimated": true, "saved_dollars_estimated": true,
        "graph_calls": 0, "graph_errors": 0, "graph_reliability_pct": 100.0,
        "graph_p50_ms": 0.0, "graph_p95_ms": 0.0, "graph_avg_ms": 0.0,
        "graph_saved_tokens": 0, "graph_saved_dollars": 0.0,
        "graph_saved_tokens_estimated": true, "graph_saved_dollars_estimated": true,
        "session_count": 0, "memory_session_count": 0, "session_recall_pct": 0.0,
        "tool_aggs": {}, "repo_aggs": {}, "agent_aggs": {},
        "graph_recent_queries": []
    })
}
