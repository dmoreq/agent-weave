//! Terminal ANSI query monitor for `aw`.
//! Zero curses, zero Python, zero external dependencies.

use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use crate::db;
use crate::error::Result;

pub fn stream_monitor(
    root: Option<&Path>,
    repo_filter: Option<&str>,
    errors_only: bool,
) -> Result<()> {
    let log_path = if let Some(r) = root {
        crate::journal::usage_log_path(r)
    } else if let Ok(cwd) = std::env::current_dir() {
        let local_p = crate::journal::usage_log_path(&cwd);
        if local_p.is_file() {
            local_p
        } else {
            db::global_usage_log_path().unwrap_or_else(|_| PathBuf::from("usage.log"))
        }
    } else {
        db::global_usage_log_path().unwrap_or_else(|_| PathBuf::from("usage.log"))
    };

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
        "{:<8} {:<14} {:<12} {:<10} {:<9} ARGUMENTS",
        "TIME", "REPO", "TOOL", "LATENCY", "STATUS"
    );
    println!("{}", "─".repeat(88));

    let mut last_pos = if let Ok(meta) = std::fs::metadata(&log_path) {
        meta.len()
    } else {
        0
    };

    loop {
        if let Ok(file) = File::open(&log_path) {
            if let Ok(meta) = file.metadata() {
                let cur_len = meta.len();
                if cur_len < last_pos {
                    last_pos = 0;
                }
                if cur_len > last_pos {
                    let mut reader = BufReader::new(file);
                    if reader.seek(SeekFrom::Start(last_pos)).is_ok() {
                        let mut line = String::new();
                        while let Ok(n) = reader.read_line(&mut line) {
                            if n == 0 {
                                break;
                            }
                            let trimmed = line.trim();
                            if !trimmed.is_empty() {
                                if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
                                    render_event_line(&val, repo_filter, errors_only);
                                }
                            }
                            line.clear();
                        }
                        if let Ok(pos) = reader.stream_position() {
                            last_pos = pos;
                        }
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn render_event_line(val: &Value, repo_filter: Option<&str>, errors_only: bool) {
    let exit_code = val.get("exit_code").and_then(Value::as_i64).unwrap_or(0);
    if errors_only && exit_code < 3 {
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
    let time_str = if ts > 0 {
        let secs = (ts % 86400) as u32;
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        let s = secs % 60;
        format!("{:02}:{:02}:{:02}", hours, mins, s)
    } else {
        "--:--:--".to_string()
    };

    let tool = val.get("tool").and_then(Value::as_str).unwrap_or("-");
    let lat_us = val.get("latency_us").and_then(Value::as_f64);
    let lat_str = lat_us
        .map(|us| format!("{:.1}ms", us / 1000.0))
        .unwrap_or_else(|| "-".to_string());

    let (status_str, status_color) = if exit_code == 0 {
        ("OK", "\x1b[32m") // Green
    } else if exit_code == 1 {
        ("EMPTY", "\x1b[33m") // Yellow
    } else if exit_code == 2 {
        ("AMBIG", "\x1b[35m") // Magenta
    } else {
        ("ERR", "\x1b[31;1m") // Bold Red
    };

    let args_val = val.get("args").cloned().unwrap_or(Value::Null);
    let args_str = serde_json::to_string(&args_val).unwrap_or_default();
    let max_args_len = 36;
    let short_args = if args_str.len() > max_args_len {
        format!("{}...", &args_str[..max_args_len - 3])
    } else {
        args_str
    };

    let r_truncated = if repo_name.len() > 13 {
        &repo_name[..13]
    } else {
        repo_name
    };
    let t_truncated = if tool.len() > 11 { &tool[..11] } else { tool };

    println!(
        "{:<8} {:<14} \x1b[36m{:<12}\x1b[0m {:<10} {}{:<9}\x1b[0m {}",
        time_str, r_truncated, t_truncated, lat_str, status_color, status_str, short_args
    );
}
