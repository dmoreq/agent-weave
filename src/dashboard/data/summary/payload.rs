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

/// The graph event window, newest first. Only graph tools are rendered, so no
/// all-tools twin is produced.
fn recent_graph_window(entries: &[Value]) -> Vec<Value> {
    entries
        .iter()
        .rev()
        .filter(|e| is_graph_tool(e.get("tool").and_then(Value::as_str).unwrap_or("")))
        .take(50)
        .cloned()
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
