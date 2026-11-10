//! Usage-log parsing, aggregation, and rendering: per-tool and per-repo
//! folds, summary rendering, telemetry totals, and bounded log loading.

use super::*;

/// Bounded reader for one journal generation: reject non-regular files and
/// cap allocation at 16 MiB even when the file is oversized or growing.
const MAX_LOG_READ: u64 = 16 * 1024 * 1024;

fn read_log_bounded(path: &Path) -> Option<String> {
    let file = crate::walk::open_regular_file(path).ok().flatten()?;
    let mut reader = file.take(MAX_LOG_READ);
    let mut out = String::new();
    reader.read_to_string(&mut out).ok()?;
    Some(out)
}

/// Parsed usage line. Queries and signatures can contain quotes, backslashes,
/// and control characters, so this must go through serde_json rather than
/// slicing the raw text between quote characters.
pub(crate) struct UsageFields {
    pub(crate) tool: String,
    pub(crate) exit_code: i64,
    pub(crate) result_count: i64,
    pub(crate) shown: i64,
    pub(crate) total: i64,
    pub(crate) stale: bool,
    pub(crate) ambiguous: i64,
    pub(crate) latency_us: Option<i64>,
    pub(crate) ts: Option<i64>,
    pub(crate) repo: Option<String>,
    pub(crate) repo_name: Option<String>,
    pub(crate) worktree: bool,
    pub(crate) agent: Option<String>,
}

pub(crate) fn parse_usage_line(line: &str) -> Option<UsageFields> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    Some(UsageFields {
        tool: v.get("tool")?.as_str()?.to_string(),
        exit_code: v.get("exit_code").and_then(|x| x.as_i64()).unwrap_or(0),
        result_count: v.get("result_count").and_then(|x| x.as_i64()).unwrap_or(0),
        shown: v.get("shown").and_then(|x| x.as_i64()).unwrap_or(0),
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
    })
}

#[derive(Default)]
struct RepoAgg {
    calls: usize,
    errors: usize,
    worktree: bool,
}

struct ErrorRecord {
    ts: Option<i64>,
    tool: String,
    repo_name: Option<String>,
    exit_code: i64,
    agent: Option<String>,
}

#[derive(Default)]
struct ToolAgg {
    calls: usize,
    errors: usize,
    empty: usize,
    empty_ok: usize,
    empty_error: usize,
    stale: usize,
    ambiguous: usize,
    capped: usize,
    latencies: Vec<u64>,
    total_shown: usize,
    total_results: usize,
}

/// Summarize `usage.log` (+ `.1`): per-tool calls, errors, p50/p95, rates.
/// `quality` adds the empty-result split and the cap count without changing
/// the journal schema or the default text.
pub fn summary(root: &Path, quality: bool) -> crate::error::Result<Vec<String>> {
    let lines = load_usage_lines(root)?;
    let aggs = aggregate_usage(&lines);
    Ok(render_summary(&aggs, quality))
}

/// Summarize machine-wide global journal (~/.engram/usage.log): cross-repo calls,
/// per-repository breakdown, error rate, token savings, and recent errors.
pub fn global_summary(quality: bool) -> crate::error::Result<Vec<String>> {
    let lines = load_global_usage_lines()?;
    let (aggs, repo_aggs, recent_errors) = aggregate_global_usage(&lines);
    Ok(render_global_summary(
        &aggs,
        &repo_aggs,
        &recent_errors,
        quality,
    ))
}

fn load_usage_lines_from_path(base_log: &Path) -> crate::error::Result<String> {
    let mut lines = String::new();
    for gen in [".1", ""] {
        let path = if gen.is_empty() {
            base_log.to_path_buf()
        } else {
            let mut p = base_log.to_path_buf().into_os_string();
            p.push(gen);
            PathBuf::from(p)
        };
        if let Some(content) = read_log_bounded(&path) {
            lines.push_str(&content);
        }
    }
    if lines.trim().is_empty() {
        return Err(crate::error::EgError::NoResults(
            "no journal entries — usage.log is empty or absent".to_string(),
        ));
    }
    Ok(lines)
}

/// Read both journal generations, newest last (`""` is current, `.1` rotated).
fn load_usage_lines(root: &Path) -> crate::error::Result<String> {
    load_usage_lines_from_path(&usage_log_path(root))
}

/// Read both global journal generations from ~/.engram/usage.log.
fn load_global_usage_lines() -> crate::error::Result<String> {
    let path = crate::db::global_usage_log_path()?;
    load_usage_lines_from_path(&path)
}

/// Fold one parsed journal line into its per-tool aggregate. The single copy
/// of the counter rules shared by the per-repo and global summaries, so their
/// error/empty/capped/stale semantics cannot drift apart.
fn fold_tool(fields: &UsageFields, agg: &mut ToolAgg) {
    agg.calls += 1;
    if fields.exit_code != 0 {
        agg.errors += 1;
    }
    if fields.result_count == 0 {
        agg.empty += 1;
        if fields.exit_code == 0 {
            agg.empty_ok += 1;
        } else {
            agg.empty_error += 1;
        }
    }
    if fields.total > fields.shown {
        agg.capped += 1;
    }
    if fields.stale {
        agg.stale += 1;
    }
    agg.ambiguous += fields.ambiguous.max(0) as usize;
    agg.total_shown += fields.shown.max(0) as usize;
    agg.total_results += fields.result_count.max(0) as usize;
    if let Some(us) = fields.latency_us {
        agg.latencies.push(us.max(0) as u64);
    }
}

/// Fold every parsed line into one `ToolAgg` per tool.
fn aggregate_usage(lines: &str) -> std::collections::BTreeMap<String, ToolAgg> {
    let mut aggs: std::collections::BTreeMap<String, ToolAgg> = Default::default();
    for line in lines.lines().filter(|l| !l.trim().is_empty()) {
        let Some(fields) = parse_usage_line(line) else {
            continue;
        };
        if fields.is_stale() {
            continue;
        }
        fold_tool(&fields, aggs.entry(fields.tool.clone()).or_default());
    }
    aggs
}

/// Shared KPI section for both summaries: overall latency percentiles,
/// reliability, and the token-savings block. `errors_note` carries the global
/// summary's " across all repos" wording into the otherwise-identical line.
fn kpi_section(
    aggs: &std::collections::BTreeMap<String, ToolAgg>,
    errors_note: &str,
) -> Vec<String> {
    let total: usize = aggs.values().map(|a| a.calls).sum();
    let mut all_lat: Vec<u64> = aggs
        .values()
        .flat_map(|a| a.latencies.iter().copied())
        .collect();
    all_lat.sort_unstable();
    let p50 = percentile(&all_lat, 0.5);
    let p95 = percentile(&all_lat, 0.95);

    let total_errors: usize = aggs.values().map(|a| a.errors).sum();
    let total_stale: usize = aggs.values().map(|a| a.stale).sum();
    let reliability_pct = if total > 0 {
        100.0 * (total.saturating_sub(total_errors) as f64) / (total as f64)
    } else {
        100.0
    };

    let (saved_tokens, reuse_hits) = calculate_telemetry_totals(aggs);
    let dollars = (saved_tokens as f64) * PRICE_PER_TOKEN;
    let saved_tokens_formatted = format_number_with_commas(saved_tokens);
    let basis = savings_basis(aggs);

    let mut out = vec![
        format!("  ├─ Speed: p50 = {p50} | p95 = {p95}"),
        format!("  ├─ Reliability: {reliability_pct:.1}% ok ({total_errors} errors{errors_note}, {total_stale} stale reads)"),
        format!(
            "  ├─ Estimated Token Savings: ~{saved_tokens_formatted} tokens (~${dollars:.2})"
        ),
    ];
    if !basis.is_empty() {
        out.push(format!("  ├─ Savings basis: {basis}"));
    }
    out.push(format!(
        "  ├─ Anti-Reinvention: {reuse_hits} duplication warnings caught"
    ));
    out
}

/// Which tool family the estimate comes from. The coefficient table prices a
/// reuse hit far above a graph row, so one line naming the mix keeps the total
/// from reading as measured savings.
fn savings_basis(aggs: &std::collections::BTreeMap<String, ToolAgg>) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut add = |label: &str, tools: &[&str]| {
        let sum: usize = tools
            .iter()
            .map(|tool| {
                aggs.get(*tool).map_or(0, |agg| {
                    estimated_tokens_saved(
                        tool,
                        agg.total_shown,
                        agg.total_results,
                        agg.calls.saturating_sub(agg.errors),
                    )
                })
            })
            .sum();
        if sum > 0 {
            parts.push(format!("{label} ~{}", format_number_with_commas(sum)));
        }
    };
    add(
        "graph",
        &["context", "callers", "callees", "path", "impact", "imports"],
    );
    add("search", &["search"]);
    add("reuse", &["reuse"]);
    add("symbols", &["symbols"]);
    add("build", &["build"]);
    parts.join(" · ")
}

fn render_summary(
    aggs: &std::collections::BTreeMap<String, ToolAgg>,
    quality: bool,
) -> Vec<String> {
    let total: usize = aggs.values().map(|a| a.calls).sum();
    let mut out = vec![format!("eg usage: {total} journaled calls")];
    out.extend(kpi_section(aggs, ""));

    let mut rows: Vec<(&String, &ToolAgg)> = aggs.iter().collect();
    rows.sort_by(|a, b| b.1.calls.cmp(&a.1.calls).then(a.0.cmp(b.0)));
    out.extend(
        rows.into_iter()
            .map(|(tool, agg)| tool_row(tool, agg, quality)),
    );
    out
}

fn aggregate_global_usage(
    lines: &str,
) -> (
    std::collections::BTreeMap<String, ToolAgg>,
    std::collections::BTreeMap<String, RepoAgg>,
    Vec<ErrorRecord>,
) {
    let mut aggs: std::collections::BTreeMap<String, ToolAgg> = Default::default();
    let mut repo_aggs: std::collections::BTreeMap<String, RepoAgg> = Default::default();
    let mut errors: Vec<ErrorRecord> = Vec::new();

    for line in lines.lines().filter(|l| !l.trim().is_empty()) {
        let Some(fields) = parse_usage_line(line) else {
            continue;
        };
        if fields.is_stale() {
            continue;
        }
        fold_tool(&fields, aggs.entry(fields.tool.clone()).or_default());

        let repo_key = fields
            .repo_name
            .clone()
            .or_else(|| fields.repo.clone())
            .unwrap_or_else(|| "global".to_string());
        let r_agg = repo_aggs.entry(repo_key).or_default();
        r_agg.calls += 1;
        if fields.exit_code != 0 {
            r_agg.errors += 1;
            errors.push(ErrorRecord {
                ts: fields.ts,
                tool: fields.tool,
                repo_name: fields.repo_name,
                exit_code: fields.exit_code,
                agent: fields.agent,
            });
        }
        if fields.worktree {
            r_agg.worktree = true;
        }
    }
    errors.reverse();
    (aggs, repo_aggs, errors)
}

fn render_global_summary(
    aggs: &std::collections::BTreeMap<String, ToolAgg>,
    repo_aggs: &std::collections::BTreeMap<String, RepoAgg>,
    recent_errors: &[ErrorRecord],
    quality: bool,
) -> Vec<String> {
    let total: usize = aggs.values().map(|a| a.calls).sum();
    let repo_count = repo_aggs.len();
    let mut out = vec![format!(
        "eg global usage: {total} journaled calls across {repo_count} repositories"
    )];
    out.extend(kpi_section(aggs, " across all repos"));

    if !repo_aggs.is_empty() {
        out.extend(active_repos_section(repo_aggs));
    }

    out.push("  ├─ Tool Breakdown:".to_string());
    let mut rows: Vec<(&String, &ToolAgg)> = aggs.iter().collect();
    rows.sort_by(|a, b| b.1.calls.cmp(&a.1.calls).then(a.0.cmp(b.0)));
    out.extend(
        rows.into_iter()
            .map(|(tool, agg)| tool_row(tool, agg, quality)),
    );

    out.extend(recent_errors_section(recent_errors));
    out
}

fn active_repos_section(repo_aggs: &std::collections::BTreeMap<String, RepoAgg>) -> Vec<String> {
    let mut out = vec!["  ├─ Active Repositories:".to_string()];
    let mut repos: Vec<(&String, &RepoAgg)> = repo_aggs.iter().collect();
    repos.sort_by(|a, b| b.1.calls.cmp(&a.1.calls).then(a.0.cmp(b.0)));
    for (repo_name, r_agg) in repos {
        let rel = if r_agg.calls > 0 {
            100.0 * (r_agg.calls.saturating_sub(r_agg.errors) as f64) / (r_agg.calls as f64)
        } else {
            100.0
        };
        let wt_tag = if r_agg.worktree { " [worktree]" } else { "" };
        out.push(format!(
            "  │   ├─ {repo_name}{wt_tag}: {} calls ({rel:.1}% ok, {} errors)",
            r_agg.calls, r_agg.errors
        ));
    }
    out
}

fn recent_errors_section(recent_errors: &[ErrorRecord]) -> Vec<String> {
    if recent_errors.is_empty() {
        return vec!["  └─ Recent Errors: None (100% clean)".to_string()];
    }
    let now = crate::db::unix_now();
    let mut out = vec!["  └─ Recent Errors (latest first):".to_string()];
    for err in recent_errors.iter().take(5) {
        let repo_label = err.repo_name.as_deref().unwrap_or("unknown");
        let agent_label = err.agent.as_deref().unwrap_or("user");
        let age_str = err
            .ts
            .map(|t| format_age_secs(now.saturating_sub(t)))
            .unwrap_or_else(|| "-".to_string());
        out.push(format!(
            "      • [{age_str}] {} in {repo_label} [{agent_label}] -> exit {}",
            err.tool, err.exit_code
        ));
    }
    out
}

fn format_age_secs(diff: i64) -> String {
    if diff < 60 {
        format!("{diff}s ago")
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

/// Informational USD-per-token coefficient for the `~$` savings estimate: a
fn calculate_telemetry_totals(
    aggs: &std::collections::BTreeMap<String, ToolAgg>,
) -> (usize, usize) {
    let mut saved_tokens = 0;
    let mut reuse_hits = 0;

    for (tool, agg) in aggs {
        let successful_calls = agg.calls.saturating_sub(agg.errors);
        saved_tokens +=
            estimated_tokens_saved(tool, agg.total_shown, agg.total_results, successful_calls);
        if tool == "reuse" {
            reuse_hits += agg.total_results.max(agg.total_shown);
        }
    }

    (saved_tokens, reuse_hits)
}

fn tool_row(tool: &str, agg: &ToolAgg, quality: bool) -> String {
    let mut lat = agg.latencies.clone();
    lat.sort_unstable();
    let p50 = percentile(&lat, 0.5);
    let p95 = percentile(&lat, 0.95);
    if quality {
        format!(
            "  {tool}: {} calls, errors={} empty_ok={} ({}) empty_error={} stale={} ambiguous={} capped={} p50={p50} p95={p95}",
            agg.calls,
            agg.errors,
            agg.empty_ok,
            percent(agg.empty_ok, agg.calls),
            agg.empty_error,
            agg.stale,
            agg.ambiguous,
            agg.capped,
        )
    } else {
        format!(
            "  {tool}: {} calls, errors={} empty={} stale={} ambiguous={} p50={p50} p95={p95}",
            agg.calls, agg.errors, agg.empty, agg.stale, agg.ambiguous,
        )
    }
}
