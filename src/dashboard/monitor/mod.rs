//! Interactive terminal monitor for `aw`.
//! Zero external curses dependencies, zero unsafe code.

pub mod render;
pub mod state;

use std::fs::File;
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::Value;

use self::render::render_screen;
use self::state::{MonitorTab, TuiState};
use crate::db;
use crate::error::Result;

enum InputEvent {
    Tab(MonitorTab),
    Refresh,
    Quit,
}

pub fn stream_monitor(
    root: Option<&Path>,
    repo_filter: Option<&str>,
    errors_only: bool,
    local: bool,
) -> Result<()> {
    let log_path = resolve_monitor_log_path(root, local);

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return stream_linear(&log_path, repo_filter, errors_only);
    }

    let Some(guard) = RawModeGuard::enter() else {
        return stream_linear(&log_path, repo_filter, errors_only);
    };

    run_interactive(root, log_path, repo_filter, errors_only, guard)
}

struct RawModeGuard {
    saved: Option<String>,
}

impl RawModeGuard {
    fn enter() -> Option<Self> {
        let saved = Command::new("stty").arg("-g").output().ok().and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        });

        let status = Command::new("stty")
            .arg("-echo")
            .arg("cbreak")
            .status()
            .ok()?;
        if status.success() {
            let mut out = io::stdout();
            let _ = write!(out, "\x1b[?1049h\x1b[?25l");
            let _ = out.flush();
            Some(Self { saved })
        } else {
            None
        }
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let mut out = io::stdout();
        let _ = write!(out, "\x1b[?25h\x1b[?1049l");
        let _ = out.flush();
        if let Some(ref s) = self.saved {
            let _ = Command::new("stty").arg(s).status();
        } else {
            let _ = Command::new("stty").arg("echo").arg("-cbreak").status();
        }
    }
}

fn run_interactive(
    root: Option<&Path>,
    log_path: PathBuf,
    repo_filter: Option<&str>,
    errors_only: bool,
    _guard: RawModeGuard,
) -> Result<()> {
    let repo_filter_owned = repo_filter.map(str::to_string);
    let mut state = TuiState::new(root, log_path.clone(), repo_filter_owned, errors_only);

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut stdin = io::stdin();
        let mut buf = [0u8; 1];
        while stdin.read_exact(&mut buf).is_ok() {
            let evt = match buf[0] {
                b'1' => InputEvent::Tab(MonitorTab::Graph),
                b'2' => InputEvent::Tab(MonitorTab::Reuse),
                b'3' => InputEvent::Tab(MonitorTab::Memory),
                b'4' => InputEvent::Tab(MonitorTab::Stream),
                b'r' | b'R' => InputEvent::Refresh,
                b'q' | b'Q' | 3 | 4 | 27 => InputEvent::Quit,
                _ => continue,
            };
            if tx.send(evt).is_err() {
                break;
            }
        }
    });

    let mut last_pos = file_len(&log_path);
    let mut last_refresh = std::time::Instant::now();
    let root_path = root.map(Path::to_path_buf);

    let (mut cols, mut rows) = get_terminal_size();
    let mut stdout = io::stdout();
    let _ = render_screen(&mut stdout, &state, cols, rows);

    loop {
        let mut need_render = false;

        while let Ok(evt) = rx.try_recv() {
            match evt {
                InputEvent::Quit => return Ok(()),
                InputEvent::Tab(tab) => {
                    if state.active_tab != tab {
                        state.active_tab = tab;
                        need_render = true;
                    }
                }
                InputEvent::Refresh => {
                    state.refresh(root_path.as_deref());
                    need_render = true;
                }
            }
        }

        if let Ok(file) = File::open(&log_path) {
            if poll_log_new_events(file, &mut last_pos, &mut state) {
                need_render = true;
            }
        }

        if last_refresh.elapsed() >= Duration::from_secs(5) {
            state.refresh(root_path.as_deref());
            last_refresh = std::time::Instant::now();
            need_render = true;
        }

        let (new_cols, new_rows) = get_terminal_size();
        if new_cols != cols || new_rows != rows {
            cols = new_cols;
            rows = new_rows;
            need_render = true;
        }

        if need_render {
            let _ = render_screen(&mut stdout, &state, cols, rows);
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn file_len(path: &Path) -> u64 {
    File::open(path)
        .and_then(|f| f.metadata())
        .map(|m| m.len())
        .unwrap_or(0)
}

fn poll_log_new_events(file: File, last_pos: &mut u64, state: &mut TuiState) -> bool {
    let Ok(meta) = file.metadata() else {
        return false;
    };
    let cur_len = meta.len();
    if cur_len < *last_pos {
        *last_pos = 0;
    }
    if cur_len <= *last_pos {
        return false;
    }

    let mut reader = BufReader::new(file);
    if reader.seek(SeekFrom::Start(*last_pos)).is_err() {
        return false;
    }

    let mut new_event_found = false;
    let mut line = String::new();
    while let Ok(n) = reader.read_line(&mut line) {
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
                state.push_event(val);
                new_event_found = true;
            }
        }
        line.clear();
    }

    if let Ok(pos) = reader.stream_position() {
        *last_pos = pos;
    }

    new_event_found
}

fn get_terminal_size() -> (usize, usize) {
    if let Ok(out) = Command::new("stty").arg("size").output() {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            let mut parts = text.split_whitespace();
            if let (Some(r), Some(c)) = (parts.next(), parts.next()) {
                if let (Ok(rows), Ok(cols)) = (r.parse::<usize>(), c.parse::<usize>()) {
                    return (cols.max(40), rows.max(10));
                }
            }
        }
    }
    (80, 24)
}

fn stream_linear(log_path: &Path, repo_filter: Option<&str>, errors_only: bool) -> Result<()> {
    println!(
        "aw: query monitor (linear streaming) from {}",
        log_path.display()
    );
    let mut last_pos = file_len(log_path).saturating_sub(4096);
    loop {
        if let Ok(file) = File::open(log_path) {
            poll_linear_events(file, &mut last_pos, repo_filter, errors_only);
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn poll_linear_events(
    file: File,
    last_pos: &mut u64,
    repo_filter: Option<&str>,
    errors_only: bool,
) {
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

    let mut stdout = io::stdout();
    let mut line = String::new();
    while let Ok(n) = reader.read_line(&mut line) {
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
                if is_event_matched(&val, repo_filter, errors_only) {
                    let _ = render::render_event_row(&mut stdout, &val, 78);
                }
            }
        }
        line.clear();
    }

    if let Ok(pos) = reader.stream_position() {
        *last_pos = pos;
    }
}

fn is_event_matched(val: &Value, repo_filter: Option<&str>, errors_only: bool) -> bool {
    if errors_only {
        let code = val.get("exit_code").and_then(Value::as_i64).unwrap_or(0);
        if code < 3 {
            return false;
        }
    }
    if let Some(rf) = repo_filter {
        if rf != "ALL" {
            let r_name = val
                .get("repo_name")
                .and_then(Value::as_str)
                .unwrap_or("workspace");
            if r_name != rf {
                return false;
            }
        }
    }
    true
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
