//! Usage journal: one JSONL line per journaled invocation appended to
//! `.aw/usage.log`. Best-effort by contract: a failed journal write is
//! silently ignored and can never alter a command's output or exit code.
//! The line is serialized by serde_json (the MCP boundary dependency); the
//! summary reader extracts only the fixed keys we write.

use std::cell::RefCell;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Rotate `usage.log` to `usage.log.1` past this size (one prior generation).
pub(crate) const ROTATE_BYTES: u64 = 5 * 1024 * 1024;

pub fn usage_log_path(root: &Path) -> PathBuf {
    crate::db::aw_dir(root).join("usage.log")
}

/// Session/arm attribution: R1/R2 A/B tags, sanitized.
fn env_tag(key: &str) -> String {
    std::env::var(key)
        .map(|v| v.trim().chars().take(64).collect())
        .unwrap_or_default()
}

/// Session attribution for memory rows (`memories.session`): the same
/// sanitized AW_SESSION tag the journal line carries. Public because the
/// memory write path stamps it at write time.
pub fn session_tag() -> String {
    env_tag("AW_SESSION")
}

/// Journal opt-out contract: env `AW_NO_JOURNAL=1` (checked in `begin`) or
/// the global `--no-journal` flag (checked in `main` — it skips `begin`).
struct Pending {
    surface: &'static str,
    tool: String,
    args: serde_json::Map<String, serde_json::Value>,
    result_count: usize,
    shown: usize,
    total: usize,
    ambiguous: usize,
    stale: bool,
    session: String,
    arm: String,
}

thread_local! {
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
    static DISABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Disable journaling for this process without mutating the environment.
/// Used by the MCP server when `--no-journal` is requested.
pub fn disable() {
    DISABLED.with(|d| d.set(true));
}

/// Key/value arguments one journaled invocation carries: built by the
/// `service` layer's `journal_args` helpers and rendered into the usage
/// line's `args` object by `begin`.
pub type JournalArgs = Vec<(&'static str, String)>;

/// Arm the journal for this invocation. No-op when the env opt-out is set;
/// the `--no-journal` flag is enforced by the caller skipping `begin`.
pub fn begin(surface: &'static str, tool: &str, args: &JournalArgs) {
    if DISABLED.with(|d| d.get()) || std::env::var("AW_NO_JOURNAL").as_deref() == Ok("1") {
        return;
    }
    let args: serde_json::Map<String, serde_json::Value> = args
        .iter()
        .map(|(k, v)| ((*k).to_string(), serde_json::Value::String(v.clone())))
        .collect();
    PENDING.with(|p| {
        *p.borrow_mut() = Some(Pending {
            surface,
            tool: tool.to_string(),
            args,
            result_count: 0,
            shown: 0,
            total: 0,
            ambiguous: 0,
            stale: false,
            session: env_tag("AW_SESSION"),
            arm: env_tag("AW_ARM"),
        });
    });
}

/// The three same-typed outcome counters `record_result` journals. Named
/// fields keep `result_count`/`shown`/`total` from being swapped silently at
/// the call sites.
pub struct OutcomeCounts {
    pub result_count: usize,
    pub shown: usize,
    pub total: usize,
}

impl OutcomeCounts {
    /// One count for all three counters: the non-paginated commands that
    /// journal no truncation and no filtering.
    pub fn uniform(n: usize) -> Self {
        Self {
            result_count: n,
            shown: n,
            total: n,
        }
    }

    /// The paginated pointer queries: the journal count is the displayed set,
    /// so `result_count` equals `shown` and only `total` can differ.
    pub fn of(shown: usize, total: usize) -> Self {
        Self {
            result_count: shown,
            shown,
            total,
        }
    }
}

/// Handlers inject the query outcome before the process exits. No-op when the
/// journal was not armed (disabled, or a non-journaled subcommand).
pub fn record_result(counts: OutcomeCounts, stale: bool, ambiguous: usize) {
    PENDING.with(|p| {
        if let Some(pending) = p.borrow_mut().as_mut() {
            pending.result_count = counts.result_count;
            pending.shown = counts.shown;
            pending.total = counts.total;
            pending.stale = stale;
            pending.ambiguous = ambiguous;
        }
    });
}

/// Serialize and append the usage line. Called exactly once per process,
/// after the outcome is known. All I/O failures are swallowed by contract.
/// A `None` root (repo discovery failed) skips the write — there is no
/// `.aw/` location.
pub fn finalize(root: Option<&Path>, exit_code: i32, latency_us: u64) {
    let Some(pending) = PENDING.with(|p| p.borrow_mut().take()) else {
        return;
    };
    let Some(root) = root else {
        return;
    };
    let ts = crate::db::unix_now();
    let line = serde_json::json!({
        "ts": ts,
        "surface": pending.surface,
        "tool": pending.tool,
        "args": Value::Object(pending.args),
        "result_count": pending.result_count,
        "shown": pending.shown,
        "total": pending.total,
        "ambiguous": pending.ambiguous,
        "exit_code": exit_code,
        "latency_us": latency_us,
        "stale": pending.stale,
        "aw_version": env!("CARGO_PKG_VERSION"),
        "session": pending.session,
        "arm": pending.arm,
    });
    append_line(&usage_log_path(root), &format!("{line}\n"));
}

fn append_line(path: &Path, line: &str) {
    // Write confinement: the journal is derived state and may only grow a
    // regular file under a real `.aw/` directory. A symlinked directory or log
    // would append attacker-chosen content outside the repo (the read path
    // already rejects non-regular files).
    if let Some(dir) = path.parent() {
        if let Ok(meta) = fs::symlink_metadata(dir) {
            if !meta.is_dir() {
                return;
            }
        }
    }
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.is_file() {
            return;
        }
    }
    rotate_if_needed(path, ROTATE_BYTES);
    // Whether `.aw` exists before this append: the journal seeds the gitignore
    // marker only when IT creates the directory. A pre-existing `.aw` is owned
    // by build/doctor (or the user) — appending must not touch its marker.
    let aw_created_here = path
        .parent()
        .is_none_or(|dir| fs::symlink_metadata(dir).is_err());
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let mut file = match OpenOptions::new().append(true).open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            recreate_journal(path, aw_created_here);
            match OpenOptions::new().create_new(true).append(true).open(path) {
                Ok(file) => file,
                Err(_) => return,
            }
        }
        Err(_) => return,
    };
    let _ = file.write_all(line.as_bytes());
}

/// Seed the `.aw` gitignore marker on the recreate path, but only when THIS
/// append created the directory: a pre-existing `.aw` is owned by
/// build/doctor (or the user) and appending must not touch its marker.
fn recreate_journal(path: &Path, aw_created_here: bool) {
    if !aw_created_here {
        return;
    }
    let Some(root) = path.parent().and_then(Path::parent) else {
        return;
    };
    let _ = crate::db::ensure_aw_gitignore(root);
}

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

fn rotate_if_needed(path: &Path, max: u64) {
    let oversized = fs::metadata(path).map(|m| m.len() > max).unwrap_or(false);
    if !oversized {
        return;
    }
    let mut rotated = path.as_os_str().to_owned();
    rotated.push(".1");
    let _ = fs::rename(path, PathBuf::from(rotated));
}

// ---------------------------------------------------------------------------
// `aw usage` summary: tolerant parsing of the fixed keys this module writes.
// ---------------------------------------------------------------------------

/// Parsed usage line. Queries and signatures can contain quotes, backslashes,
/// and control characters, so this must go through serde_json rather than
/// slicing the raw text between quote characters.
struct UsageFields {
    tool: String,
    exit_code: i64,
    result_count: i64,
    shown: i64,
    total: i64,
    stale: bool,
    ambiguous: i64,
    latency_us: Option<i64>,
}

fn parse_usage_line(line: &str) -> Option<UsageFields> {
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
    })
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

/// Read both journal generations, newest last (`""` is current, `.1` rotated).
fn load_usage_lines(root: &Path) -> crate::error::Result<String> {
    let mut lines = String::new();
    for gen in [".1", ""] {
        let path = usage_log_path(root);
        let path = if gen.is_empty() {
            path
        } else {
            let mut p = path.into_os_string();
            p.push(gen);
            PathBuf::from(p)
        };
        if let Some(content) = read_log_bounded(&path) {
            lines.push_str(&content);
        }
    }
    if lines.trim().is_empty() {
        return Err(crate::error::AwError::NoResults(
            "no journal entries — usage.log is empty or absent".to_string(),
        ));
    }
    Ok(lines)
}

/// Fold every parsed line into one `ToolAgg` per tool.
fn aggregate_usage(lines: &str) -> std::collections::BTreeMap<String, ToolAgg> {
    let mut aggs: std::collections::BTreeMap<String, ToolAgg> = Default::default();
    for line in lines.lines().filter(|l| !l.trim().is_empty()) {
        let Some(fields) = parse_usage_line(line) else {
            continue;
        };
        let agg = aggs.entry(fields.tool).or_default();
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
    aggs
}

fn render_summary(
    aggs: &std::collections::BTreeMap<String, ToolAgg>,
    quality: bool,
) -> Vec<String> {
    let total: usize = aggs.values().map(|a| a.calls).sum();
    let mut out = vec![format!("aw usage: {total} journaled calls")];

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

    out.push(format!("  ├─ Speed: p50 = {p50} | p95 = {p95}"));
    out.push(format!(
        "  ├─ Reliability: {reliability_pct:.1}% ok ({total_errors} errors, {total_stale} stale reads)"
    ));
    out.push(format!(
        "  ├─ Estimated Token Savings: ~{saved_tokens_formatted} tokens (~${dollars:.2})"
    ));
    out.push(format!(
        "  ├─ Anti-Reinvention: {reuse_hits} duplication warnings caught"
    ));

    let mut rows: Vec<(&String, &ToolAgg)> = aggs.iter().collect();
    rows.sort_by(|a, b| b.1.calls.cmp(&a.1.calls).then(a.0.cmp(b.0)));
    out.extend(
        rows.into_iter()
            .map(|(tool, agg)| tool_row(tool, agg, quality)),
    );
    out
}

/// Informational USD-per-token coefficient for the `~$` savings estimate: a
/// rough blended average of mainstream frontier-model input/output list
/// prices. Display-only — no journal row, exit code, or decision reads it.
const PRICE_PER_TOKEN: f64 = 0.000003;

/// Estimated tokens one rendered graph/context row replaces: a pointer the
/// agent would otherwise re-derive by reading the callee/caller source.
const GRAPH_ROW_TOKENS: usize = 350;
/// Estimated tokens one rendered search/symbol row replaces.
const SEARCH_ROW_TOKENS: usize = 300;
/// Estimated tokens one reuse hit replaces (candidate row plus its trimmed
/// code snippet).
const REUSE_RESULT_TOKENS: usize = 800;
/// Estimated tokens one reuse report replaces beyond its rows (the ranked
/// comparison prose the agent would otherwise write itself).
const REUSE_CALL_TOKENS: usize = 1200;
/// Estimated tokens one rendered symbol row replaces.
const SYMBOL_ROW_TOKENS: usize = 150;
/// Estimated tokens one successful build report replaces (the incremental
/// summary the agent would otherwise re-derive by re-running the build).
const BUILD_CALL_TOKENS: usize = 500;
/// Per-call overhead shared by every priced query arm: the result envelope,
/// staleness header, and pointer identity suffixes.
const QUERY_CALL_TOKENS: usize = 100;

/// Journaled tool names deliberately priced at zero: diagnostics, inventory
/// reports, and memory housekeeping replace no source-file read, so they
/// contribute no token-savings estimate. A new journaled tool must either
/// gain a pricing arm in `estimated_tokens_saved` or be listed here — pinned
/// by `service::tests::estimated_tokens_saved_covers_every_journaled_tool`.
pub const UNPRICED_TOOLS: &[&str] = &[
    "doctor",
    "unresolved",
    "stats",
    "usage",
    "mem_write",
    "mem_query",
    "mem_get",
    "mem_delete",
    "mem_gc",
    "mem_export",
];

/// Estimated tokens one tool call saves the agent versus reading the same
/// result out of source files. The single coefficient table for BOTH telemetry
/// surfaces: `aw usage` aggregates journaled calls with it, and the MCP KPI
/// footer prices the single call it just rendered (subtracting that call's own
/// payload) — so the two surfaces cannot report contradictory numbers.
pub fn estimated_tokens_saved(
    tool: &str,
    shown: usize,
    result_count: usize,
    calls: usize,
) -> usize {
    match tool {
        "context" | "callers" | "callees" | "path" | "impact" | "imports" => {
            shown * GRAPH_ROW_TOKENS + calls * QUERY_CALL_TOKENS
        }
        "search" => shown * SEARCH_ROW_TOKENS + calls * QUERY_CALL_TOKENS,
        "reuse" => result_count * REUSE_RESULT_TOKENS + calls * REUSE_CALL_TOKENS,
        "symbols" => shown * SYMBOL_ROW_TOKENS,
        "build" => calls * BUILD_CALL_TOKENS,
        // Deliberately unpriced journaled tools (see UNPRICED_TOOLS).
        t if UNPRICED_TOOLS.contains(&t) => 0,
        // Legacy names from older journal generations, no longer dispatchable.
        _ => 0,
    }
}

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

fn format_number_with_commas(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let len = s.len();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(c);
    }
    result
}

fn percent(n: usize, d: usize) -> String {
    if d == 0 {
        "-".to_string()
    } else {
        format!("{:.1}%", 100.0 * n as f64 / d as f64)
    }
}

fn percentile(lat: &[u64], q: f64) -> String {
    if lat.is_empty() {
        return "-".to_string();
    }
    let idx = ((lat.len() as f64 * q).ceil() as usize)
        .saturating_sub(1)
        .min(lat.len() - 1);
    format!("{:.1}ms", lat[idx] as f64 / 1000.0)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_moves_log_aside_past_threshold() {
        let dir = crate::db::temp_root("journal");
        let path = dir.join("usage.log");
        fs::write(&path, b"1234567").unwrap();
        rotate_if_needed(&path, 5);
        assert!(!path.exists());
        assert_eq!(fs::read(dir.join("usage.log.1")).unwrap(), b"1234567");
        rotate_if_needed(&path, 5); // absent -> no-op
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_fields_round_trip() {
        let line = "{\"ts\":1,\"surface\":\"cli\",\"tool\":\"callers\",\"args\":{\"name\":\"a\"},\"result_count\":3,\"shown\":3,\"total\":3,\"ambiguous\":1,\"exit_code\":0,\"latency_us\":9800,\"stale\":false,\"aw_version\":\"0.1.0\"}";
        let f = parse_usage_line(line).unwrap();
        assert_eq!(f.tool, "callers");
        assert_eq!(f.result_count, 3);
        assert_eq!(f.ambiguous, 1);
        assert_eq!(f.latency_us, Some(9800));
        assert!(!f.stale);
        // Lines without optional keys keep working.
        let legacy = parse_usage_line(
            "{\"ts\":1,\"surface\":\"cli\",\"tool\":\"callers\",\"result_count\":1,\"exit_code\":0}",
        )
        .unwrap();
        assert_eq!(legacy.tool, "callers");
        assert_eq!(legacy.latency_us, None);
    }

    #[test]
    fn usage_parsing_unescapes_quoted_text() {
        let line = r#"{"tool":"search","args":{"terms":"say \"run\" \\ now"},"result_count":2,"exit_code":0,"latency_us":1200,"stale":true,"ambiguous":3}"#;
        let f = parse_usage_line(line).unwrap();
        assert_eq!(f.tool, "search");
        assert_eq!(f.result_count, 2);
        assert_eq!(f.latency_us, Some(1200));
        assert!(f.stale);
        assert_eq!(f.ambiguous, 3);
    }

    #[test]
    fn summary_quality_splits_empty_and_cap() {
        let root = crate::db::temp_root("journal-quality");
        fs::create_dir_all(root.join(".aw")).unwrap();
        let lines = [
            r#"{"tool":"callers","result_count":2,"shown":2,"total":2,"exit_code":0,"latency_us":1000}"#,
            r#"{"tool":"callers","result_count":0,"shown":0,"total":0,"exit_code":0,"latency_us":2000}"#,
            r#"{"tool":"callers","result_count":0,"shown":0,"total":0,"exit_code":2,"latency_us":3000}"#,
            r#"{"tool":"callers","result_count":50,"shown":50,"total":60,"exit_code":0,"latency_us":4000}"#,
        ];
        fs::write(usage_log_path(&root), lines.join("\n")).unwrap();

        let default = summary(&root, false).unwrap().join("\n");
        assert!(
            default.contains("callers: 4 calls, errors=1 empty=2 stale=0 ambiguous=0"),
            "{default}"
        );
        assert!(!default.contains("empty_ok"), "{default}");

        let quality = summary(&root, true).unwrap().join("\n");
        assert!(quality.contains("empty_ok=1 (25.0%)"), "{quality}");
        assert!(quality.contains("empty_error=1"), "{quality}");
        assert!(quality.contains("capped=1"), "{quality}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn env_tag_trims_and_caps_at_64() {
        // Safety: test-only env mutation, single-threaded test binary section.
        std::env::set_var("AW_TEST_TAG", "  r1-task-1  ");
        assert_eq!(env_tag("AW_TEST_TAG"), "r1-task-1");
        std::env::set_var("AW_TEST_TAG", "x".repeat(100));
        assert_eq!(env_tag("AW_TEST_TAG").len(), 64);
        std::env::remove_var("AW_TEST_TAG");
        assert_eq!(env_tag("AW_TEST_TAG"), "");
    }

    #[cfg(unix)]
    #[test]
    fn append_refuses_symlinked_log_and_directory() {
        let dir = crate::db::temp_root("journal-link");
        let target = dir.join("target.txt");
        fs::write(&target, b"keep").unwrap();
        let log = dir.join("usage.log");
        std::os::unix::fs::symlink(&target, &log).unwrap();
        append_line(&log, "line\n");
        assert_eq!(fs::read(&target).unwrap(), b"keep");

        // A symlinked `.aw` directory must not redirect the write either.
        let outside = dir.join("outside");
        fs::create_dir_all(&outside).unwrap();
        let linked_root = dir.join("repo");
        fs::create_dir_all(&linked_root).unwrap();
        std::os::unix::fs::symlink(&outside, linked_root.join(".aw")).unwrap();
        append_line(&linked_root.join(".aw/usage.log"), "line\n");
        assert!(!outside.join("usage.log").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
