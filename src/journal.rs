//! Usage journal (Amendment 6) and result-payload journal (Amendment 10):
//! one JSONL line per journaled invocation appended to `.kg/usage.log`, plus
//! one self-contained pointer-payload line per successful pointer query
//! appended to `.kg/usage-results.jsonl`. Best-effort by contract: a failed
//! journal write is silently ignored and can never alter a command's output
//! or exit code. Zero dependencies — the JSON line is hand-serialized; the
//! summary reader extracts only the fixed keys we write.

use std::cell::RefCell;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Rotate `usage.log` to `usage.log.1` past this size (one prior generation).
pub(crate) const ROTATE_BYTES: u64 = 5 * 1024 * 1024;

pub fn usage_log_path(root: &Path) -> PathBuf {
    crate::db::kg_dir(root).join("usage.log")
}

/// Result-payload journal (Amendment 10): rendered pointers of the displayed
/// set, one line per successful pointer query.
pub fn payload_log_path(root: &Path) -> PathBuf {
    crate::db::kg_dir(root).join("usage-results.jsonl")
}

/// Session/arm attribution (Amendment 10b): R1/R2 A/B tags, sanitized.
fn env_tag(key: &str) -> String {
    std::env::var(key)
        .map(|v| v.trim().chars().take(64).collect())
        .unwrap_or_default()
}

/// Journal opt-out contract: env `KG_NO_JOURNAL=1` (checked in `begin`) or
/// the global `--no-journal` flag (checked in `main` — it skips `begin`).
struct Pending {
    surface: &'static str,
    tool: String,
    args: String, // pre-serialized JSON object body
    result_count: usize,
    shown: usize,
    total: usize,
    ambiguous: usize,
    stale: bool,
    session: String,
    arm: String,
    // Payload fields (Amendment 10): set by record_payload on the success path
    // of a pointer-returning query; empty pointers -> no payload line.
    name: String,
    pointers: Vec<String>,
    built_at: u64,
    source_rev: String,
}

thread_local! {
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
}

/// Arm the journal for this invocation. No-op when the env opt-out is set;
/// the `--no-journal` flag is enforced by the caller skipping `begin`.
pub fn begin(surface: &'static str, tool: &str, args: &[(&'static str, String)]) {
    if std::env::var("KG_NO_JOURNAL").as_deref() == Ok("1") {
        return;
    }
    let body = args
        .iter()
        .map(|(k, v)| format!("\"{}\":\"{}\"", k, json_escape(v)))
        .collect::<Vec<_>>()
        .join(",");
    PENDING.with(|p| {
        *p.borrow_mut() = Some(Pending {
            surface,
            tool: tool.to_string(),
            args: format!("{{{body}}}"),
            result_count: 0,
            shown: 0,
            total: 0,
            ambiguous: 0,
            stale: false,
            session: env_tag("KG_SESSION"),
            arm: env_tag("KG_ARM"),
            name: String::new(),
            pointers: Vec::new(),
            built_at: 0,
            source_rev: String::new(),
        });
    });
}

/// Handlers inject the query outcome before the process exits. No-op when the
/// journal was not armed (disabled, or a non-journaled subcommand).
pub fn record_result(result_count: usize, shown: usize, total: usize, stale: bool, ambiguous: usize) {
    PENDING.with(|p| {
        if let Some(pending) = p.borrow_mut().as_mut() {
            pending.result_count = result_count;
            pending.shown = shown;
            pending.total = total;
            pending.stale = stale;
            pending.ambiguous = ambiguous;
        }
    });
}

/// Capture the payload of a successful pointer query (Amendment 10a):
/// the rendered pointer text of the displayed set — byte-identical to the
/// query's stdout data lines — plus the index meta the pointers came from.
/// Called from the query layer (`query::collect_output`, `query::symbols`);
/// empty `pointers` writes no payload line at finalize time.
pub fn record_payload(
    name: &str,
    pointers: &[String],
    stale: bool,
    shown: usize,
    total: usize,
    built_at: u64,
    source_rev: &str,
) {
    PENDING.with(|p| {
        if let Some(pending) = p.borrow_mut().as_mut() {
            pending.name = name.to_string();
            pending.pointers = pointers.to_vec();
            pending.stale = stale;
            pending.shown = shown;
            pending.total = total;
            pending.built_at = built_at;
            pending.source_rev = source_rev.to_string();
        }
    });
}

/// Serialize and append the usage line, then the payload line when pointers
/// were captured. Called exactly once per process, after the outcome is known.
/// All I/O failures are swallowed by contract. A `None` root (repo discovery
/// failed) skips the write — there is no `.kg/` location.
pub fn finalize(root: Option<&Path>, exit_code: i32, latency_us: u64) {
    let Some(pending) = PENDING.with(|p| p.borrow_mut().take()) else {
        return;
    };
    let Some(root) = root else {
        return;
    };
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!(
        "{{\"ts\":{ts},\"surface\":\"{}\",\"tool\":\"{}\",\"args\":{},\"result_count\":{},\"shown\":{},\"total\":{},\"ambiguous\":{},\"exit_code\":{exit_code},\"latency_us\":{latency_us},\"stale\":{},\"kg_version\":\"{}\",\"session\":\"{}\",\"arm\":\"{}\"}}\n",
        pending.surface,
        pending.tool,
        pending.args,
        pending.result_count,
        pending.shown,
        pending.total,
        pending.ambiguous,
        pending.stale,
        env!("CARGO_PKG_VERSION"),
        json_escape(&pending.session),
        json_escape(&pending.arm),
    );
    append_line(&usage_log_path(root), &line);
    if !pending.pointers.is_empty() {
        append_line(&payload_log_path(root), &payload_line(&pending, ts));
    }
}

fn append_line(path: &Path, line: &str) {
    rotate_if_needed(path, ROTATE_BYTES);
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Payload journal line (Amendment 10a): self-contained — replay verification
/// needs no join with usage.log. Same `ts` as the usage line of the invocation.
fn payload_line(p: &Pending, ts: u64) -> String {
    let pointers = p
        .pointers
        .iter()
        .map(|ptr| format!("\"{}\"", json_escape(ptr)))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"ts\":{ts},\"surface\":\"{}\",\"tool\":\"{}\",\"name\":\"{}\",\"session\":\"{}\",\"arm\":\"{}\",\"built_at\":{},\"source_rev\":\"{}\",\"stale\":{},\"shown\":{},\"total\":{},\"pointers\":[{pointers}]}}\n",
        p.surface,
        p.tool,
        json_escape(&p.name),
        json_escape(&p.session),
        json_escape(&p.arm),
        p.built_at,
        json_escape(&p.source_rev),
        p.stale,
        p.shown,
        p.total,
    )
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

/// Minimal JSON string escaping for hand-serialized values (symbols, paths).
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// `kg usage` summary: tolerant extraction of the fixed keys this module writes.
// ---------------------------------------------------------------------------

/// Extract `"key":"value"` from a JSONL line written by this module.
fn extract_str(line: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":\"");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Extract `"key":<number|bool>` from a JSONL line written by this module.
/// Booleans map to 1/0 so the `stale` flag aggregates like a counter.
fn extract_num(line: &str, key: &str) -> Option<i64> {
    let needle = format!("\"{key}\":");
    let start = line.find(&needle)? + needle.len();
    let rest = line[start..].trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if let Ok(n) = digits.parse() {
        return Some(n);
    }
    if rest.starts_with("true") {
        return Some(1);
    }
    if rest.starts_with("false") {
        return Some(0);
    }
    None
}

#[derive(Default)]
struct ToolAgg {
    calls: usize,
    errors: usize,
    empty: usize,
    stale: usize,
    ambiguous: usize,
    latencies: Vec<u64>,
}

/// Summarize `usage.log` (+ `.1`): per-tool calls, errors, p50/p95, rates.
pub fn summary(root: &Path) -> crate::error::Result<Vec<String>> {
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
        if let Ok(content) = fs::read_to_string(&path) {
            lines.push_str(&content);
        }
    }
    if lines.trim().is_empty() {
        return Err(crate::error::KgError::NoResults(
            "no journal entries — usage.log is empty or absent".to_string(),
        ));
    }
    let mut aggs: std::collections::BTreeMap<String, ToolAgg> = Default::default();
    for line in lines.lines().filter(|l| !l.trim().is_empty()) {
        let Some(tool) = extract_str(line, "tool") else {
            continue;
        };
        let agg = aggs.entry(tool).or_default();
        agg.calls += 1;
        if extract_num(line, "exit_code").unwrap_or(0) != 0 {
            agg.errors += 1;
        }
        if extract_num(line, "result_count").unwrap_or(0) == 0 {
            agg.empty += 1;
        }
        if extract_num(line, "stale").unwrap_or(0) == 1 {
            agg.stale += 1;
        }
        agg.ambiguous += extract_num(line, "ambiguous").unwrap_or(0).max(0) as usize;
        if let Some(us) = extract_num(line, "latency_us") {
            agg.latencies.push(us.max(0) as u64);
        }
    }
    let total: usize = aggs.values().map(|a| a.calls).sum();
    let mut out = vec![format!("kg usage: {total} journaled calls")];
    let mut rows: Vec<(&String, &ToolAgg)> = aggs.iter().collect();
    rows.sort_by(|a, b| b.1.calls.cmp(&a.1.calls).then(a.0.cmp(b.0)));
    for (tool, agg) in rows {
        let mut lat = agg.latencies.clone();
        lat.sort_unstable();
        let p = |q: f64| -> String {
            if lat.is_empty() {
                "-".to_string()
            } else {
                let idx = ((lat.len() as f64 * q).ceil() as usize)
                    .saturating_sub(1)
                    .min(lat.len() - 1);
                format!("{:.1}ms", lat[idx] as f64 / 1000.0)
            }
        };
        out.push(format!(
            "  {tool}: {} calls, errors={} empty={} stale={} ambiguous={} p50={} p95={}",
            agg.calls,
            agg.errors,
            agg.empty,
            agg.stale,
            agg.ambiguous,
            p(0.5),
            p(0.95),
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_escapes_quotes_backslashes_and_controls() {
        assert_eq!(json_escape("a\"b"), "a\\\"b");
        assert_eq!(json_escape("a\\b"), "a\\\\b");
        assert_eq!(json_escape("a\nb"), "a\\nb");
        assert_eq!(json_escape("a\u{1}b"), "a\\u0001b");
        assert_eq!(json_escape("normal.text"), "normal.text");
    }

    #[test]
    fn rotate_moves_log_aside_past_threshold() {
        let dir = std::env::temp_dir().join(format!("kg-journal-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
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
        let line = "{\"ts\":1,\"surface\":\"cli\",\"tool\":\"callers\",\"args\":{\"name\":\"a\"},\"result_count\":3,\"shown\":3,\"total\":3,\"ambiguous\":1,\"exit_code\":0,\"latency_us\":9800,\"stale\":false,\"kg_version\":\"0.1.0\"}";
        assert_eq!(extract_str(line, "tool").as_deref(), Some("callers"));
        assert_eq!(extract_num(line, "result_count"), Some(3));
        assert_eq!(extract_num(line, "ambiguous"), Some(1));
        assert_eq!(extract_num(line, "latency_us"), Some(9800));
        assert_eq!(extract_num(line, "stale"), Some(0));
        // Amendment 10 additive keys stay extractable.
        assert_eq!(
            extract_str(line, "session"),
            None,
            "pre-Amendment-10 lines have no session key"
        );
    }

    #[test]
    fn payload_line_is_self_contained_and_escaped() {
        let p = Pending {
            surface: "mcp",
            tool: "callers".into(),
            args: "{}".into(),
            result_count: 2,
            shown: 2,
            total: 2,
            ambiguous: 0,
            stale: true,
            session: "r1-t1".into(),
            arm: "kg".into(),
            name: "a.b".into(),
            pointers: vec!["src/a.rs:7".into(), "src/b.rs:12 [ambiguous]".into()],
            built_at: 42,
            source_rev: "abc".into(),
        };
        let line = payload_line(&p, 7);
        assert_eq!(
            line,
            "{\"ts\":7,\"surface\":\"mcp\",\"tool\":\"callers\",\"name\":\"a.b\",\
             \"session\":\"r1-t1\",\"arm\":\"kg\",\"built_at\":42,\
             \"source_rev\":\"abc\",\"stale\":true,\"shown\":2,\"total\":2,\
             \"pointers\":[\"src/a.rs:7\",\"src/b.rs:12 [ambiguous]\"]}\n"
        );
        assert_eq!(extract_str(&line, "session").as_deref(), Some("r1-t1"));
        assert_eq!(extract_num(&line, "built_at"), Some(42));
        assert_eq!(extract_num(&line, "stale"), Some(1));
    }

    #[test]
    fn env_tag_trims_and_caps_at_64() {
        // Safety: test-only env mutation, single-threaded test binary section.
        std::env::set_var("KG_TEST_TAG", "  r1-task-1  ");
        assert_eq!(env_tag("KG_TEST_TAG"), "r1-task-1");
        std::env::set_var("KG_TEST_TAG", "x".repeat(100));
        assert_eq!(env_tag("KG_TEST_TAG").len(), 64);
        std::env::remove_var("KG_TEST_TAG");
        assert_eq!(env_tag("KG_TEST_TAG"), "");
    }
}
