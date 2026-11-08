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
    let ts = crate::db::unix_now();
    let repo_path = root.map(|r| r.to_string_lossy().into_owned());
    let repo_name = root
        .and_then(|r| r.file_name())
        .map(|n| n.to_string_lossy().into_owned());
    let is_worktree = root.map(|r| r.join(".git").is_file()).unwrap_or(false);

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
        "repo": repo_path,
        "repo_name": repo_name,
        "worktree": is_worktree,
    });
    let line_str = format!("{line}\n");

    if let Some(root) = root {
        append_line(&usage_log_path(root), &line_str);
    }
    if let Ok(global_log) = crate::db::global_usage_log_path() {
        if let Some(parent) = global_log.parent() {
            let should_append = if std::env::var("AW_GLOBAL_DIR").is_ok() {
                parent.is_dir()
            } else {
                true
            };
            if should_append {
                append_line(&global_log, &line_str);
            }
        }
    }
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
    if let Ok(global_dir) = crate::db::global_aw_dir() {
        if path.starts_with(&global_dir) {
            return;
        }
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
    ts: Option<i64>,
    repo: Option<String>,
    repo_name: Option<String>,
    worktree: bool,
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
    })
}

mod render;
mod tokens;

pub(crate) use render::*;
pub(crate) use tokens::*;
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
