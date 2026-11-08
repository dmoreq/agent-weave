//! Usage-log summary aggregation: reliability, latency percentiles, and
//! token-savings accounting folded per tool and repository.

use std::collections::{BTreeMap, HashMap};

use serde_json::{json, Value};

pub fn compute_summary(entries: &[Value]) -> Value {
    let total_calls = entries.len();
    if total_calls == 0 {
        return json!({
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
            "tool_aggs": {},
            "repo_aggs": {},
            "recent_queries": [],
            "recent_errors": []
        });
    }

    let mut errors = 0usize;
    let mut warnings = 0usize;
    let mut reuse_hits = 0usize;
    let mut saved_tokens = 0usize;
    let mut latencies_ms: Vec<f64> = Vec::with_capacity(entries.len());
    let mut tool_aggs: HashMap<String, ToolAgg> = HashMap::new();
    let mut repo_aggs: HashMap<String, RepoAgg> = HashMap::new();
    let mut recent_errors: Vec<Value> = Vec::new();

    for e in entries {
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

        let t_entry = tool_aggs.entry(tool.to_string()).or_default();
        t_entry.calls += 1;
        t_entry.shown += shown;

        let r_entry = repo_aggs.entry(repo_name.to_string()).or_default();
        r_entry.calls += 1;
        r_entry.worktree = is_worktree;
        if ts > r_entry.last_ts {
            r_entry.last_ts = ts;
        }

        if exit_code >= 3 {
            errors += 1;
            t_entry.errors += 1;
            r_entry.errors += 1;
            recent_errors.push(e.clone());
        } else if exit_code == 1 || exit_code == 2 {
            warnings += 1;
        }

        if let Some(us) = lat_us {
            if us >= 0.0 {
                let ms = us / 1000.0;
                latencies_ms.push(ms);
                t_entry.latencies.push(ms);
            }
        }

        if exit_code == 0 {
            // The journal's single pricing table is authoritative for BOTH
            // telemetry surfaces (see journal/tokens.rs); `calls` is 1 because
            // one usage entry is one journaled call.
            if tool == "reuse" {
                reuse_hits += shown.max(result_count);
            }
            saved_tokens += crate::journal::estimated_tokens_saved(tool, shown, result_count, 1);
        }
    }

    latencies_ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = latencies_ms.len();
    let p50 = if n > 0 {
        latencies_ms[(0.50 * n as f64).ceil() as usize - 1]
    } else {
        0.0
    };
    let p95 = if n > 0 {
        latencies_ms[(0.95 * n as f64).ceil() as usize - 1]
    } else {
        0.0
    };
    let avg = if n > 0 {
        latencies_ms.iter().sum::<f64>() / n as f64
    } else {
        0.0
    };

    let mut tool_aggs_json = BTreeMap::new();
    for (k, mut v) in tool_aggs {
        v.latencies
            .sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let vn = v.latencies.len();
        let p50_t = if vn > 0 {
            v.latencies[(0.50 * vn as f64).ceil() as usize - 1]
        } else {
            0.0
        };
        tool_aggs_json.insert(
            k,
            json!({
                "calls": v.calls,
                "errors": v.errors,
                "shown": v.shown,
                "p50": p50_t,
            }),
        );
    }

    let mut repo_aggs_json = BTreeMap::new();
    for (k, v) in repo_aggs {
        let rel = if v.calls > 0 {
            100.0 * (v.calls - v.errors) as f64 / v.calls as f64
        } else {
            100.0
        };
        repo_aggs_json.insert(
            k,
            json!({
                "calls": v.calls,
                "errors": v.errors,
                "worktree": v.worktree,
                "last_ts": v.last_ts,
                "reliability_pct": rel,
            }),
        );
    }

    let reliability = 100.0 * (total_calls - errors) as f64 / total_calls as f64;
    let saved_dollars = saved_tokens as f64 * crate::journal::PRICE_PER_TOKEN;

    let recent_queries: Vec<Value> = entries.iter().rev().take(50).cloned().collect();
    recent_errors.reverse();
    recent_errors.truncate(25);

    json!({
        "total_calls": total_calls,
        "error_count": errors,
        "warning_count": warnings,
        "reliability_pct": reliability,
        "p50_ms": p50,
        "p95_ms": p95,
        "avg_ms": avg,
        "saved_tokens": saved_tokens,
        "saved_dollars": saved_dollars,
        "reuse_hits": reuse_hits,
        "tool_aggs": tool_aggs_json,
        "repo_aggs": repo_aggs_json,
        "recent_queries": recent_queries,
        "recent_errors": recent_errors,
    })
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
    last_ts: f64,
}
