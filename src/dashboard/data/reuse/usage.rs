//! Per-candidate reuse usage from the retained journal.

use std::collections::HashMap;

use serde_json::{json, Value};

/// Per-candidate usage from the retained journal: how many `eg reuse` runs
/// surfaced this pair, and when the last one was. Legacy lines without
/// `args.candidates` contribute nothing (count 0 = never surfaced in window).
pub(super) fn aggregate_candidate_usage(entries: &[Value]) -> HashMap<String, (usize, f64)> {
    let mut usage: HashMap<String, (usize, f64)> = HashMap::new();
    for e in entries {
        if e.get("tool").and_then(Value::as_str) != Some("reuse") {
            continue;
        }
        let Some(repo) = e.get("repo").and_then(Value::as_str) else {
            continue;
        };
        let Some(list) = e
            .get("args")
            .and_then(|a| a.get("candidates"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let ts = e.get("ts").and_then(Value::as_f64).unwrap_or(0.0);
        for pair in list.split('\n').filter(|p| !p.is_empty()) {
            let rec = usage.entry(format!("{repo}|{pair}")).or_insert((0, 0.0));
            rec.0 += 1;
            if ts > rec.1 {
                rec.1 = ts;
            }
        }
    }
    usage
}

pub(super) fn decorate_usage(candidates: &mut [Value], usage: &HashMap<String, (usize, f64)>) {
    for candidate in candidates.iter_mut() {
        let repo_path = candidate
            .get("repo_path")
            .and_then(Value::as_str)
            .unwrap_or("");
        let symbol = candidate
            .get("symbol")
            .and_then(Value::as_str)
            .unwrap_or("");
        let canonical = candidate
            .get("canonical")
            .and_then(Value::as_str)
            .unwrap_or("");
        let key = format!("{repo_path}|{symbol}->{canonical}");
        let (count, last_ts) = usage.get(&key).copied().unwrap_or((0, 0.0));
        candidate["reuse_count"] = json!(count);
        candidate["last_reused_ts"] = json!(last_ts);
        candidate["usage_window"] = json!("retained_journal");
    }
}

#[cfg(test)]
mod usage_tests {
    use super::*;

    #[test]
    fn usage_aggregates_per_repo_and_candidate_pair() {
        let entries = vec![
            json!({
                "tool": "reuse", "repo": "/r/a", "ts": 100.0,
                "args": { "candidates": "f->g\nh->g" }
            }),
            json!({
                "tool": "reuse", "repo": "/r/a", "ts": 250.0,
                "args": { "candidates": "f->g" }
            }),
            // Same pair under another repo must not accrue.
            json!({
                "tool": "reuse", "repo": "/r/b", "ts": 300.0,
                "args": { "candidates": "f->g" }
            }),
            // Legacy line without candidate ids contributes nothing.
            json!({ "tool": "reuse", "repo": "/r/a", "ts": 400.0, "args": {} }),
            // Other tools never contribute.
            json!({
                "tool": "search", "repo": "/r/a", "ts": 500.0,
                "args": { "candidates": "f->g" }
            }),
        ];
        let usage = aggregate_candidate_usage(&entries);
        assert_eq!(usage.get("/r/a|f->g").copied(), Some((2, 250.0)));
        assert_eq!(usage.get("/r/a|h->g").copied(), Some((1, 100.0)));
        assert_eq!(usage.get("/r/b|f->g").copied(), Some((1, 300.0)));
    }

    #[test]
    fn decorate_usage_attaches_counts_and_window() {
        let mut candidates = vec![json!({
            "repo_path": "/r/a", "symbol": "f", "canonical": "g"
        })];
        let mut usage = HashMap::new();
        usage.insert("/r/a|f->g".to_string(), (3usize, 250.0f64));
        decorate_usage(&mut candidates, &usage);
        assert_eq!(candidates[0]["reuse_count"], 3);
        assert_eq!(candidates[0]["last_reused_ts"], 250.0);
        assert_eq!(candidates[0]["usage_window"], "retained_journal");
    }
}
