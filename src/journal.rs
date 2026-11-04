//! Usage journal: one JSONL line per journaled invocation appended to
//! `.aw/usage.log`. Best-effort by contract: a failed journal write is
//! silently ignored and can never alter a command's output or exit code.
//! Zero dependencies — the JSON line is hand-serialized; the summary reader
//! extracts only the fixed keys we write.

use std::cell::RefCell;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Journal opt-out contract: env `AW_NO_JOURNAL=1` (checked in `begin`) or
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

/// Arm the journal for this invocation. No-op when the env opt-out is set;
/// the `--no-journal` flag is enforced by the caller skipping `begin`.
pub fn begin(surface: &'static str, tool: &str, args: &[(&'static str, String)]) {
    if DISABLED.with(|d| d.get()) || std::env::var("AW_NO_JOURNAL").as_deref() == Ok("1") {
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
            session: env_tag("AW_SESSION"),
            arm: env_tag("AW_ARM"),
        });
    });
}

/// Replace the armed argument object when the argument set depends on
/// parsed input (resolve count). No-op when the journal is not armed.
pub fn set_args(args: &[(&'static str, String)]) {
    let body = args
        .iter()
        .map(|(k, v)| format!("\"{}\":\"{}\"", k, json_escape(v)))
        .collect::<Vec<_>>()
        .join(",");
    PENDING.with(|p| {
        if let Some(pending) = p.borrow_mut().as_mut() {
            pending.args = format!("{{{body}}}");
        }
    });
}

/// Handlers inject the query outcome before the process exits. No-op when the
/// journal was not armed (disabled, or a non-journaled subcommand).
pub fn record_result(
    result_count: usize,
    shown: usize,
    total: usize,
    stale: bool,
    ambiguous: usize,
) {
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
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!(
        "{{\"ts\":{ts},\"surface\":\"{}\",\"tool\":\"{}\",\"args\":{},\"result_count\":{},\"shown\":{},\"total\":{},\"ambiguous\":{},\"exit_code\":{exit_code},\"latency_us\":{latency_us},\"stale\":{},\"aw_version\":\"{}\",\"session\":\"{}\",\"arm\":\"{}\"}}\n",
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
}

fn append_line(path: &Path, line: &str) {
    if is_symlink(path) || path.parent().is_some_and(is_symlink) {
        return;
    }
    // Reject FIFOs/devices and hardlinks before any open: append must never
    // block or modify an external inode through a link.
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.is_file() || has_multiple_links(&meta) {
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
    let mut file = match fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_file() || has_multiple_links(&meta) => return,
        Ok(_) => match OpenOptions::new().append(true).open(path) {
            Ok(file) => file,
            Err(_) => return,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // The journal self-created `.aw` (create_dir_all above): seed the
            // tool-owned gitignore marker so the directory never leaks into
            // `git status`. Best-effort — a non-git repo or unreadable marker
            // must not fail the journal write.
            if aw_created_here {
                if let Some(aw) = path.parent() {
                    if let Some(root) = aw.parent() {
                        let _ = crate::db::ensure_aw_gitignore(root);
                    }
                }
            }
            match OpenOptions::new().create_new(true).append(true).open(path) {
                Ok(file) => file,
                Err(_) => return,
            }
        }
        Err(_) => return,
    };
    // Re-check after open: a path swapped to a symlink, FIFO, or hardlink
    // must not receive the append through the opened target.
    if let Ok(meta) = file.metadata() {
        if !meta.is_file() || has_multiple_links(&meta) {
            return;
        }
    }
    if is_symlink(path) {
        return;
    }
    let _ = file.write_all(line.as_bytes());
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
}

#[cfg(unix)]
fn has_multiple_links(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.nlink() > 1
}

#[cfg(not(unix))]
fn has_multiple_links(_: &fs::Metadata) -> bool {
    false
}

/// Bounded reader for one journal generation: reject non-regular files and
/// cap allocation at 16 MiB even when the file is oversized or growing.
const MAX_LOG_READ: u64 = 16 * 1024 * 1024;

fn read_log_bounded(path: &Path) -> Option<String> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let file = fs::File::open(path).ok()?;
    let mut reader = file.take(MAX_LOG_READ);
    let mut out = String::new();
    reader.read_to_string(&mut out).ok()?;
    Some(out)
}

fn rotate_if_needed(path: &Path, max: u64) {
    if is_symlink(path) || path.parent().is_some_and(is_symlink) {
        return;
    }
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
// `aw usage` summary: tolerant parsing of the fixed keys this module writes.
// ---------------------------------------------------------------------------

/// Parsed usage line. Queries and signatures can contain quotes, backslashes,
/// and control characters, so this must go through serde_json rather than
/// slicing the raw text between quote characters.
struct UsageFields {
    tool: String,
    exit_code: i64,
    result_count: i64,
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
        if let Some(content) = read_log_bounded(&path) {
            lines.push_str(&content);
        }
    }
    if lines.trim().is_empty() {
        return Err(crate::error::AwError::NoResults(
            "no journal entries — usage.log is empty or absent".to_string(),
        ));
    }
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
        }
        if fields.stale {
            agg.stale += 1;
        }
        agg.ambiguous += fields.ambiguous.max(0) as usize;
        if let Some(us) = fields.latency_us {
            agg.latencies.push(us.max(0) as u64);
        }
    }
    let total: usize = aggs.values().map(|a| a.calls).sum();
    let mut out = vec![format!("aw usage: {total} journaled calls")];
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
        let dir = std::env::temp_dir().join(format!("aw-journal-{}", std::process::id()));
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
    fn env_tag_trims_and_caps_at_64() {
        // Safety: test-only env mutation, single-threaded test binary section.
        std::env::set_var("AW_TEST_TAG", "  r1-task-1  ");
        assert_eq!(env_tag("AW_TEST_TAG"), "r1-task-1");
        std::env::set_var("AW_TEST_TAG", "x".repeat(100));
        assert_eq!(env_tag("AW_TEST_TAG").len(), 64);
        std::env::remove_var("AW_TEST_TAG");
        assert_eq!(env_tag("AW_TEST_TAG"), "");
    }
}
