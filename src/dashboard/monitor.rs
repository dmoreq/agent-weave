//! Terminal ANSI query monitor for `aw`.
//! Zero curses, zero Python, zero external dependencies.

use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use crate::db;
use crate::error::ExitClass;
use crate::error::Result;

pub fn stream_monitor(
    root: Option<&Path>,
    repo_filter: Option<&str>,
    errors_only: bool,
    local: bool,
) -> Result<()> {
    let log_path = resolve_monitor_log_path(root, local);
    print_monitor_header(&log_path, repo_filter, errors_only);

    let mut last_pos = preload_backlog_events(&log_path, repo_filter, errors_only, 15);

    loop {
        if let Ok(file) = File::open(&log_path) {
            poll_log_events(file, &mut last_pos, repo_filter, errors_only);
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn resolve_monitor_log_path(root: Option<&Path>, local: bool) -> PathBuf {
    if local {
        if let Some(r) = root {
            return crate::journal::usage_log_path(r);
        }
        if let Ok(cwd) = std::env::current_dir() {
            let local_p = crate::journal::usage_log_path(&cwd);
            if local_p.is_file() {
                return local_p;
            }
        }
    }
    db::global_usage_log_path().unwrap_or_else(|_| PathBuf::from("usage.log"))
}

fn preload_backlog_events(
    log_path: &Path,
    repo_filter: Option<&str>,
    errors_only: bool,
    max_lines: usize,
) -> u64 {
    let Ok(file) = File::open(log_path) else {
        return 0;
    };
    let meta_len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let reader = BufReader::new(file);
    let mut recent_lines = Vec::new();
    for line in reader.lines().map_while(std::result::Result::ok) {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            recent_lines.push(trimmed.to_string());
            if recent_lines.len() > max_lines * 4 {
                recent_lines.drain(0..max_lines * 2);
            }
        }
    }

    let start_idx = recent_lines.len().saturating_sub(max_lines);
    for line in &recent_lines[start_idx..] {
        if let Ok(val) = serde_json::from_str::<Value>(line) {
            let is_worktree = val
                .get("worktree")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if is_worktree {
                if let Some(repo) = val.get("repo").and_then(Value::as_str) {
                    if !Path::new(repo).exists() {
                        continue;
                    }
                }
            }
            render_event_line(&val, repo_filter, errors_only);
        }
    }

    meta_len
}

fn print_monitor_header(log_path: &Path, repo_filter: Option<&str>, errors_only: bool) {
    println!(
        "aw: live query monitor streaming from {}",
        log_path.display()
    );
    let filter_repo_str = repo_filter.unwrap_or("ALL");
    let mode_str = if errors_only {
        "CRITICAL ERRORS ONLY"
    } else {
        "ALL CALLS"
    };
    println!("Filter: Repo=[{filter_repo_str}] | Mode=[{mode_str}] | Ctrl+C to stop\n");
    println!(
        "{:<8} {:<14} {:<12} {:<12} {:<10} {:<9} ARGUMENTS",
        "TIME", "REPO", "AGENT", "TOOL", "LATENCY", "STATUS"
    );
    println!("{}", "─".repeat(96));
}

fn poll_log_events(file: File, last_pos: &mut u64, repo_filter: Option<&str>, errors_only: bool) {
    let Ok(meta) = file.metadata() else {
        return;
    };
    let cur_len = meta.len();
    if cur_len < *last_pos {
        *last_pos = 0;
    }
    if cur_len <= *last_pos {
        return;
    }

    let mut reader = BufReader::new(file);
    if reader.seek(SeekFrom::Start(*last_pos)).is_err() {
        return;
    }

    let mut line = String::new();
    while let Ok(n) = reader.read_line(&mut line) {
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
                let is_worktree = val
                    .get("worktree")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if is_worktree {
                    if let Some(repo) = val.get("repo").and_then(Value::as_str) {
                        if !Path::new(repo).exists() {
                            line.clear();
                            continue;
                        }
                    }
                }
                render_event_line(&val, repo_filter, errors_only);
            }
        }
        line.clear();
    }
    if let Ok(pos) = reader.stream_position() {
        *last_pos = pos;
    }
}

fn format_timestamp(ts: i64) -> String {
    if ts <= 0 {
        return "--:--:--".to_string();
    }
    let secs = (ts % 86400) as u32;
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{hours:02}:{mins:02}:{s:02}")
}

fn render_event_line(val: &Value, repo_filter: Option<&str>, errors_only: bool) {
    let exit_code = val.get("exit_code").and_then(Value::as_i64).unwrap_or(0);
    let exit_class = ExitClass::of(exit_code);
    if errors_only && exit_class != ExitClass::Error {
        return;
    }

    let repo_name = val
        .get("repo_name")
        .and_then(Value::as_str)
        .unwrap_or("workspace");
    if let Some(rf) = repo_filter {
        if rf != "ALL" && rf != repo_name {
            return;
        }
    }

    let ts = val.get("ts").and_then(Value::as_i64).unwrap_or(0);
    let time_str = format_timestamp(ts);
    let tool = val.get("tool").and_then(Value::as_str).unwrap_or("-");
    let lat_str = val
        .get("latency_us")
        .and_then(Value::as_f64)
        .map(|us| format!("{:.1}ms", us / 1000.0))
        .unwrap_or_else(|| "-".to_string());

    let (status_str, status_color) = match exit_class {
        ExitClass::Ok => ("OK", "\x1b[32m"),           // Green
        ExitClass::Empty => ("EMPTY", "\x1b[33m"),     // Yellow
        ExitClass::Ambiguous => ("AMBIG", "\x1b[35m"), // Magenta
        ExitClass::Error => ("ERR", "\x1b[31;1m"),     // Bold Red
    };

    let args_val = val.get("args").cloned().unwrap_or(Value::Null);
    let args_str = serde_json::to_string(&args_val).unwrap_or_default();
    let max_args_len = 36;
    let short_args = if args_str.len() > max_args_len {
        format!("{}...", truncate_chars(&args_str, max_args_len - 3))
    } else {
        args_str
    };

    let r_truncated = truncate_chars(repo_name, 13);
    let agent_name = val
        .get("agent")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("user");
    let a_truncated = truncate_chars(agent_name, 11);
    println!(
        "{time_str:<8} {r_truncated:<14} {a_truncated:<12} {tool:<12} {lat_str:<10} {status_color}{status_str:<9}\x1b[0m {short_args}"
    );
}

/// Truncate `s` to at most `max_chars` UTF-8 scalar values safely without
/// panicking on multibyte character boundaries.
fn truncate_chars(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((byte_offset, _)) => &s[..byte_offset],
        None => s,
    }
}
