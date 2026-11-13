//! Usage-log summary aggregation: reliability, latency percentiles, and
//! token-savings accounting folded per tool and repository.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use serde_json::{json, Value};

use crate::error::ExitClass;
use crate::repo::UNATTRIBUTED;

mod payload;

pub fn compute_summary(entries: &[Value]) -> Value {
    if entries.is_empty() {
        return payload::empty_summary();
    }
    let mut state = SummaryState::with_capacity(entries.len());
    for e in entries {
        state.record_entry(e);
    }
    payload::summary_json(state, entries)
}

/// One classification table for every consumer: the summary fold, the
/// monitor TUI, and the client parity test. Adding a tool means editing this
/// table once, not three divergent copies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ToolClass {
    pub(crate) graph: bool,
    pub(crate) reuse: bool,
    pub(crate) memory: bool,
    pub(crate) mem_read: bool,
    pub(crate) mem_write: bool,
}

pub(crate) fn classify(tool: &str) -> ToolClass {
    ToolClass {
        graph: matches!(
            tool,
            "callers" | "callees" | "path" | "context" | "impact" | "symbols" | "imports" | "build"
        ),
        reuse: matches!(tool, "search" | "reuse"),
        memory: tool.starts_with("mem_"),
        mem_read: matches!(tool, "mem_query" | "mem_get" | "mem_export"),
        mem_write: matches!(
            tool,
            "mem_write" | "mem_touch" | "mem_delete" | "mem_gc" | "mem_import"
        ),
    }
}

fn is_graph_tool(tool: &str) -> bool {
    classify(tool).graph
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
    /// Aggregation key: the journaled absolute path when present, else the
    /// name. A path keeps two repositories with the same directory name apart.
    repo_key: &'a str,
    /// Row label: the main repository's basename when this entry belongs to a
    /// linked worktree, else the journaled `repo_name`.
    repo_display: String,
    /// Main repository root when the entry was recorded in a linked worktree
    /// whose `.git` pointer still resolves; the row folds into it.
    repo_main: Option<String>,
    is_worktree: bool,
    agent_name: &'a str,
    session_name: &'a str,
    session_tagged: bool,
    facts: EntryFacts,
}

impl<'a> EntryView<'a> {
    /// `None` when the entry does not name a live repository: only live
    /// repositories contribute to the summary, so there is no unattributed
    /// bucket to fold into. The readers and the pruner already drop dead and
    /// identityless lines; this keeps direct callers honest.
    fn parse(e: &'a Value) -> Option<EntryView<'a>> {
        let tool = e.get("tool").and_then(Value::as_str).unwrap_or("unknown");
        let repo_key = e
            .get("repo")
            .and_then(Value::as_str)
            .filter(|s| crate::journal::liveness::repo_is_live(Some(s)))?;
        let repo_name = e
            .get("repo_name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        let is_worktree = e.get("worktree").and_then(Value::as_bool).unwrap_or(false);
        let repo_main = if is_worktree {
            crate::repo::cached_worktree_main_root(Path::new(repo_key))
                .map(|p| p.to_string_lossy().into_owned())
        } else {
            None
        };
        let repo_display = match &repo_main {
            Some(main) => main_display(main),
            None => repo_name.unwrap_or(UNATTRIBUTED).to_string(),
        };
        Some(EntryView {
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
            repo_name: repo_name.unwrap_or(UNATTRIBUTED),
            repo_key,
            repo_display,
            repo_main,
            is_worktree,
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
            session_tagged: e
                .get("session")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty()),
            facts: {
                let class = classify(tool);
                EntryFacts {
                    is_graph: class.graph,
                    is_reuse: class.reuse,
                    is_mem: class.memory,
                    is_mem_read: class.mem_read,
                    is_mem_write: class.mem_write,
                }
            },
        })
    }
}

#[derive(Default)]
struct SummaryState {
    /// Entries that survived the live-repository guard. `total_calls` counts
    /// these, never the raw slice.
    recorded: usize,
    errors: usize,
    saved_tokens: usize,
    latencies_ms: Vec<f64>,
    graph_calls: usize,
    graph_errors: usize,
    graph_saved_tokens: usize,
    graph_latencies_ms: Vec<f64>,
    /// Distinct sessions that carried a real `ENGRAM_SESSION` tag. Untagged
    /// calls create no synthetic session: the recall rate would measure the
    /// tagging discipline, not memory use.
    sessions: HashSet<String>,
    memory_sessions: HashSet<String>,
    tool_aggs: HashMap<String, ToolAgg>,
    repo_aggs: HashMap<String, RepoAgg>,
    agent_aggs: HashMap<String, RepoAgg>,
}

impl SummaryState {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            latencies_ms: Vec::with_capacity(capacity),
            ..Default::default()
        }
    }

    fn record_entry(&mut self, e: &Value) {
        let Some(v) = EntryView::parse(e) else {
            return;
        };
        self.recorded += 1;
        self.record_session(&v);
        self.record_calls(&v);
        self.record_outcome(&v);
        self.record_latency(&v);
    }

    fn tool_mut(&mut self, tool: &str) -> &mut ToolAgg {
        self.tool_aggs.entry(tool.to_string()).or_default()
    }

    fn repo_mut(&mut self, key: &str, display: &str) -> &mut RepoAgg {
        let entry = self.repo_aggs.entry(key.to_string()).or_default();
        if entry.display.is_empty() {
            entry.display = display.to_string();
        }
        entry
    }

    fn agent_mut(&mut self, agent: &str) -> &mut RepoAgg {
        self.agent_aggs.entry(agent.to_string()).or_default()
    }

    fn record_session(&mut self, v: &EntryView<'_>) {
        if v.facts.is_graph {
            self.graph_calls += 1;
        }
        if !v.session_tagged {
            return;
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
        t.results += v.result_count;
        let r = self.repo_mut(folded_key(v), &v.repo_display);
        record_entry_into(r, v.facts, v.ts, v.is_worktree && v.repo_main.is_none());
        if v.repo_main.is_some() {
            r.worktrees
                .entry(v.repo_name.to_string())
                .or_default()
                .record(v.ts);
        }
        let a = self.agent_mut(v.agent_name);
        record_entry_into(a, v.facts, v.ts, false);
    }

    fn record_outcome(&mut self, v: &EntryView<'_>) {
        match ExitClass::of(v.exit_code) {
            ExitClass::Error => self.record_error(v),
            // Empty and ambiguous calls are not failures and carry no savings;
            // there is nothing to count.
            ExitClass::Empty | ExitClass::Ambiguous => {}
            ExitClass::Ok => self.record_success(v),
        }
    }

    fn record_error(&mut self, v: &EntryView<'_>) {
        self.errors += 1;
        self.tool_mut(v.tool).errors += 1;
        self.repo_mut(folded_key(v), &v.repo_display).errors += 1;
        self.agent_mut(v.agent_name).errors += 1;
        if v.facts.is_graph {
            self.graph_errors += 1;
            self.repo_mut(folded_key(v), &v.repo_display).graph_errors += 1;
            self.agent_mut(v.agent_name).graph_errors += 1;
        }
        if v.facts.is_reuse {
            self.repo_mut(folded_key(v), &v.repo_display).reuse_errors += 1;
            self.agent_mut(v.agent_name).reuse_errors += 1;
        }
        if v.facts.is_mem {
            self.repo_mut(folded_key(v), &v.repo_display).memory_errors += 1;
            self.agent_mut(v.agent_name).memory_errors += 1;
        }
        if v.repo_main.is_some() {
            if let Some(wt) = self
                .repo_mut(folded_key(v), &v.repo_display)
                .worktrees
                .get_mut(v.repo_name)
            {
                wt.errors += 1;
            }
        }
    }

    fn record_success(&mut self, v: &EntryView<'_>) {
        let saved = crate::journal::estimated_tokens_saved(v.tool, v.shown, v.result_count, 1);
        {
            let t = self.tool_mut(v.tool);
            t.ok_calls += 1;
            t.ok_shown += v.shown;
            t.ok_results += v.result_count;
        }
        let reuse_hit = if v.tool == "reuse" {
            v.shown.max(v.result_count)
        } else {
            0
        };
        if reuse_hit > 0 {
            self.repo_mut(folded_key(v), &v.repo_display).reuse_hits += reuse_hit;
            self.agent_mut(v.agent_name).reuse_hits += reuse_hit;
        }
        self.saved_tokens += saved;
        self.repo_mut(folded_key(v), &v.repo_display).tokens_saved += saved;
        self.agent_mut(v.agent_name).tokens_saved += saved;
        if v.facts.is_graph {
            self.graph_saved_tokens += saved;
            self.repo_mut(folded_key(v), &v.repo_display)
                .graph_tokens_saved += saved;
            self.agent_mut(v.agent_name).graph_tokens_saved += saved;
        }
        if v.facts.is_reuse {
            self.repo_mut(folded_key(v), &v.repo_display)
                .reuse_tokens_saved += saved;
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

/// Aggregation key for one entry: a linked worktree folds into its main
/// repository, every other entry keeps its own repository key.
fn folded_key<'a>(v: &'a EntryView<'_>) -> &'a str {
    match &v.repo_main {
        Some(main) => main.as_str(),
        None => v.repo_key,
    }
}

/// Row label for a folded main repository: the main root's basename, which is
/// the `repo_name` the journal writes for that root.
fn main_display(main: &str) -> String {
    Path::new(main)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| main.to_string())
}

/// Stamp one journal entry with `repo_main` and `repo_main_name` when it was
/// recorded in a linked worktree whose `.git` pointer still resolves. Feeds
/// show the worktree name plus the repository it belongs to, and the summary
/// window reuses the same stamp so both stay consistent.
pub(crate) fn annotate_worktree_main(entry: &mut Value) {
    let Some(repo) = entry.get("repo").and_then(Value::as_str) else {
        return;
    };
    if !entry
        .get("worktree")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return;
    }
    let Some(main) = crate::repo::cached_worktree_main_root(Path::new(repo)) else {
        return;
    };
    if let Some(name) = main.file_name() {
        let name = name.to_string_lossy().into_owned();
        entry["repo_main"] = json!(main.to_string_lossy());
        entry["repo_main_name"] = json!(name);
    }
}

#[derive(Default)]
struct ToolAgg {
    calls: usize,
    errors: usize,
    shown: usize,
    results: usize,
    /// Success-only inputs for the token-savings arm; mirrors the global fold.
    ok_calls: usize,
    ok_shown: usize,
    ok_results: usize,
    latencies: Vec<f64>,
}

#[derive(Default)]
struct RepoAgg {
    /// Label for this row: the journaled `repo_name`, or `(unattributed)`.
    display: String,
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
    /// Per-worktree breakdown under this repository: the dashboard renders
    /// these as expandable sub-rows, so only the counts a sub-row shows are
    /// kept. Empty unless linked-worktree entries folded into this row.
    worktrees: BTreeMap<String, WtAgg>,
}

/// One linked worktree's contribution to its main repository's row.
#[derive(Default)]
struct WtAgg {
    calls: usize,
    errors: usize,
    last_ts: f64,
}

impl WtAgg {
    fn record(&mut self, ts: f64) {
        self.calls += 1;
        if ts > self.last_ts {
            self.last_ts = ts;
        }
    }
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

#[cfg(test)]
mod tests;
