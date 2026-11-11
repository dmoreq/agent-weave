//! Audit collection: journal generations, strict line validation, window and
//! baseline split, one-pass metrics, and source discovery. No writes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::error::{EgError, Result};

use super::index::{self, RepoFacts};
use super::memory::{self, MemoryFacts};

pub enum Mode {
    Local(PathBuf),
    Global,
}

impl Mode {
    pub fn is_global(&self) -> bool {
        matches!(self, Mode::Global)
    }
}

pub struct Window {
    pub days: u32,
    pub start_ts: i64,
    pub end_ts: i64,
}

pub struct BaselineInfo {
    pub days: u32,
    pub start_ts: i64,
    pub end_ts: i64,
    pub sufficient: bool,
    pub successful_calls: usize,
    pub active_days: usize,
    pub clipped: bool,
}

pub struct GenInfo {
    pub name: &'static str,
    pub status: &'static str,
    pub bytes: u64,
    pub truncated: bool,
}

#[derive(Default)]
pub struct Coverage {
    pub window_entries: usize,
    pub baseline_entries: usize,
    pub lines: usize,
    pub malformed_lines: usize,
    pub invalid_lines: usize,
    pub legacy_lines: usize,
    pub dropped_dead_repos: usize,
    pub future_ts_lines: usize,
    pub outside_scope_lines: usize,
    pub oldest_ts: Option<i64>,
    pub newest_ts: Option<i64>,
    pub truncated: bool,
    pub generations: Vec<GenInfo>,
    pub repos_active: usize,
    pub repos_checked: usize,
    pub repos_truncated: bool,
    pub stores_checked: usize,
    pub stores_truncated: bool,
}

#[derive(Default, Clone)]
pub struct ToolAgg {
    pub calls: usize,
    pub successes: usize,
    pub empty: usize,
    pub latencies_ms: Vec<f64>,
}

impl ToolAgg {
    /// Nearest-rank p95, the same convention as the CLI and dashboard.
    pub fn p95(&self) -> f64 {
        if self.latencies_ms.is_empty() {
            return 0.0;
        }
        let mut sorted = self.latencies_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sorted[(0.95 * sorted.len() as f64).ceil() as usize - 1]
    }
}

#[derive(Default, Clone)]
pub struct Metrics {
    pub entries: usize,
    pub successes: usize,
    pub ops_critical: usize,
    pub lock_errors: usize,
    pub nav_successes: usize,
    pub prewrite_successes: usize,
    pub mem_reads: usize,
    pub mem_writes: usize,
    pub priced_successes: usize,
    pub savings: usize,
    pub nav_empty: usize,
    pub search_successes: usize,
    pub search_empty: usize,
    pub mem_query_successes: usize,
    pub mem_query_empty: usize,
    pub active_days: BTreeSet<i64>,
    pub repos: BTreeSet<PathBuf>,
    pub oldest_ts: Option<i64>,
    pub tools: BTreeMap<String, ToolAgg>,
}

impl Metrics {
    fn add(&mut self, e: &Entry) {
        self.entries += 1;
        self.active_days.insert(e.ts.div_euclid(86400));
        self.oldest_ts = Some(self.oldest_ts.map_or(e.ts, |v| v.min(e.ts)));
        self.repos.insert(e.repo.clone());
        let tool = self.tools.entry(e.tool.clone()).or_default();
        tool.calls += 1;
        if let Some(us) = e.latency_us {
            tool.latencies_ms.push(us as f64 / 1000.0);
        }
        if e.exit_code != 0 {
            match e.exit_code {
                3 | 5 | 6 | 7 => self.ops_critical += 1,
                4 => self.lock_errors += 1,
                _ => {}
            }
            return;
        }
        self.successes += 1;
        tool.successes += 1;
        if e.result_count == 0 {
            tool.empty += 1;
            match e.tool.as_str() {
                "callers" | "callees" | "context" | "path" | "impact" | "imports" => {
                    self.nav_empty += 1
                }
                "search" => self.search_empty += 1,
                "mem_query" => self.mem_query_empty += 1,
                _ => {}
            }
        }
        match e.tool.as_str() {
            "callers" | "callees" | "context" | "path" | "impact" | "imports" => {
                self.nav_successes += 1
            }
            "search" => {
                self.search_successes += 1;
                self.prewrite_successes += 1;
            }
            "reuse" => self.prewrite_successes += 1,
            "mem_query" => {
                self.mem_query_successes += 1;
                self.mem_reads += 1;
            }
            "mem_get" => self.mem_reads += 1,
            "mem_write" | "mem_touch" | "mem_delete" | "mem_gc" | "mem_import" => {
                self.mem_writes += 1
            }
            _ => {}
        }
        if is_priced(&e.tool) {
            self.priced_successes += 1;
            self.savings += crate::journal::estimated_tokens_saved(
                &e.tool,
                e.shown.max(0) as usize,
                e.result_count.max(0) as usize,
                1,
            );
        }
    }
}

pub struct AuditData {
    pub mode: &'static str,
    pub now: i64,
    pub window: Window,
    pub baseline: BaselineInfo,
    pub coverage: Coverage,
    pub window_metrics: Metrics,
    pub baseline_metrics: Metrics,
    pub recent7: Metrics,
    pub repos: Vec<RepoFacts>,
    pub memory: MemoryFacts,
}

struct Entry {
    tool: String,
    exit_code: i64,
    result_count: i64,
    shown: i64,
    latency_us: Option<i64>,
    ts: i64,
    repo: PathBuf,
}

enum LineOutcome {
    Valid(Entry),
    Malformed,
    Invalid,
    Legacy,
    DeadRepo,
    Future,
    OutsideScope,
}

pub fn collect(mode: &Mode, now: i64, days: u32, baseline_days: u32) -> Result<AuditData> {
    let log_path = match mode {
        Mode::Local(root) => crate::journal::usage_log_path(root),
        Mode::Global => crate::db::global_usage_log_path()?,
    };
    let local_root = match mode {
        Mode::Local(root) => Some(root.as_path()),
        Mode::Global => None,
    };
    let w_start = now.saturating_sub(days as i64 * 86_400);
    let b_start = now.saturating_sub((days + baseline_days) as i64 * 86_400);
    let recent_start = now.saturating_sub(7 * 86_400);

    let mut coverage = Coverage::default();
    let mut window_metrics = Metrics::default();
    let mut baseline_metrics = Metrics::default();
    let mut recent7 = Metrics::default();
    let log = crate::journal::read_usage_log(&log_path);
    for gen in &log.generations {
        if gen.status == crate::journal::GenerationStatus::Error {
            return Err(EgError::Internal("audit: io-error".into()));
        }
        coverage.truncated |= gen.truncated;
        coverage.generations.push(GenInfo {
            name: gen.name,
            status: match gen.status {
                crate::journal::GenerationStatus::Absent => "absent",
                crate::journal::GenerationStatus::Read => "read",
                crate::journal::GenerationStatus::Error => "error",
            },
            bytes: gen.bytes,
            truncated: gen.truncated,
        });
        if gen.status != crate::journal::GenerationStatus::Read {
            continue;
        }
        let current = gen.name == "usage.log";
        let mut segments: Vec<&str> = gen.text.split('\n').collect();
        if current && !gen.text.ends_with('\n') {
            // A concurrent append can leave one torn tail; it is not data loss.
            segments.pop();
        }
        for raw in segments {
            let line = raw.trim_end_matches('\r').trim();
            if line.is_empty() {
                continue;
            }
            coverage.lines += 1;
            match parse_line(line, now, local_root) {
                LineOutcome::Valid(e) => {
                    coverage.oldest_ts = Some(coverage.oldest_ts.map_or(e.ts, |v| v.min(e.ts)));
                    coverage.newest_ts = Some(coverage.newest_ts.map_or(e.ts, |v| v.max(e.ts)));
                    if e.ts >= recent_start {
                        recent7.add(&e);
                    }
                    if e.ts >= w_start {
                        coverage.window_entries += 1;
                        window_metrics.add(&e);
                    } else if e.ts >= b_start {
                        coverage.baseline_entries += 1;
                        baseline_metrics.add(&e);
                    } else {
                        coverage.outside_scope_lines += 1;
                    }
                }
                LineOutcome::Malformed => coverage.malformed_lines += 1,
                LineOutcome::Invalid => coverage.invalid_lines += 1,
                LineOutcome::Legacy => coverage.legacy_lines += 1,
                LineOutcome::DeadRepo => coverage.dropped_dead_repos += 1,
                LineOutcome::Future => coverage.future_ts_lines += 1,
                LineOutcome::OutsideScope => coverage.outside_scope_lines += 1,
            }
        }
    }

    let baseline_oldest = baseline_metrics.oldest_ts;
    let clipped = baseline_oldest.map(|t| t > b_start).unwrap_or(false);
    let sufficient = baseline_metrics.successes >= 20
        && baseline_metrics.active_days.len() >= 3
        && !clipped
        && baseline_metrics.entries > 0;

    let mut repo_paths: Vec<PathBuf> = match mode {
        Mode::Local(root) => vec![root.clone()],
        Mode::Global => {
            let mut set = window_metrics.repos.clone();
            set.extend(baseline_metrics.repos.iter().cloned());
            set.into_iter().collect()
        }
    };
    repo_paths.sort();
    let repos_truncated = repo_paths.len() > 50;
    repo_paths.truncate(50);
    coverage.repos_active = repo_paths.len();
    let repos = index::collect(&repo_paths, now);
    coverage.repos_checked = repos.len();
    coverage.repos_truncated = repos_truncated;

    let memory = memory::collect(mode, &repo_paths, now);
    coverage.stores_checked = memory.stores_checked;
    coverage.stores_truncated = memory.stores_truncated;

    Ok(AuditData {
        mode: if mode.is_global() { "global" } else { "local" },
        now,
        window: Window {
            days,
            start_ts: w_start,
            end_ts: now + 1,
        },
        baseline: BaselineInfo {
            days: baseline_days,
            start_ts: b_start,
            end_ts: w_start,
            sufficient,
            successful_calls: baseline_metrics.successes,
            active_days: baseline_metrics.active_days.len(),
            clipped,
        },
        coverage,
        window_metrics,
        baseline_metrics,
        recent7,
        repos,
        memory,
    })
}

fn parse_line(line: &str, now: i64, local_root: Option<&Path>) -> LineOutcome {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return LineOutcome::Malformed;
    };
    let Some(tool_raw) = v.get("tool").and_then(|x| x.as_str()) else {
        return LineOutcome::Malformed;
    };
    if tool_raw.is_empty() || tool_raw.len() > 64 || tool_raw.chars().any(char::is_control) {
        return LineOutcome::Malformed;
    }
    let Some(ts) = v.get("ts").and_then(|x| x.as_i64()) else {
        return LineOutcome::Legacy;
    };
    if ts < 0 {
        return LineOutcome::Invalid;
    }
    if ts > now.saturating_add(300) {
        return LineOutcome::Future;
    }
    let Some(exit_code) = v.get("exit_code").and_then(|x| x.as_i64()) else {
        return LineOutcome::Invalid;
    };
    if !(0..=7).contains(&exit_code) {
        return LineOutcome::Invalid;
    }
    let count = |key: &str| -> Option<i64> {
        match v.get(key) {
            None => Some(0),
            Some(x) => x.as_i64().filter(|n| *n >= 0),
        }
    };
    let (Some(result_count), Some(shown)) = (count("result_count"), count("shown")) else {
        return LineOutcome::Invalid;
    };
    let latency_us = match v.get("latency_us") {
        None => None,
        Some(x) => match x.as_i64() {
            Some(n) if n >= 0 => Some(n),
            _ => return LineOutcome::Invalid,
        },
    };
    let Some(repo_raw) = v.get("repo").and_then(|x| x.as_str()) else {
        return LineOutcome::DeadRepo;
    };
    if repo_raw.is_empty() || !crate::journal::liveness::repo_is_live(Some(repo_raw)) {
        return LineOutcome::DeadRepo;
    }
    let repo = PathBuf::from(repo_raw);
    if let Some(root) = local_root {
        if !same_root(&repo, root) {
            return LineOutcome::OutsideScope;
        }
    }
    LineOutcome::Valid(Entry {
        tool: canonical_tool(tool_raw),
        exit_code,
        result_count,
        shown,
        latency_us,
        ts,
        repo,
    })
}

fn same_root(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

fn canonical_tool(tool: &str) -> String {
    const KNOWN: &[&str] = &[
        "build",
        "callers",
        "callees",
        "context",
        "imports",
        "path",
        "impact",
        "unresolved",
        "symbols",
        "reuse",
        "search",
        "stats",
        "doctor",
        "usage",
        "mem_write",
        "mem_touch",
        "mem_query",
        "mem_get",
        "mem_delete",
        "mem_gc",
        "mem_export",
        "mem_import",
    ];
    if KNOWN.contains(&tool) {
        tool.to_string()
    } else {
        "other".to_string()
    }
}

pub fn is_priced(tool: &str) -> bool {
    matches!(
        tool,
        "callers"
            | "callees"
            | "context"
            | "path"
            | "impact"
            | "imports"
            | "search"
            | "reuse"
            | "symbols"
            | "build"
    )
}

pub fn is_navigation(tool: &str) -> bool {
    matches!(
        tool,
        "callers" | "callees" | "context" | "path" | "impact" | "imports"
    )
}
