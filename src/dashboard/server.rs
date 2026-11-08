//! Native synchronous HTTP 1.1 and Server-Sent Events server in Rust stdlib.
//! Zero async runtimes, zero external web crates, zero Python dependency.

use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::dashboard::data;
use crate::db;
use crate::error::{AwError, Result};

static INDEX_HTML: &str = include_str!("assets/index.html");
static STYLE_CSS: &str = include_str!("assets/style.css");
static APP_JS: &str = include_str!("assets/app.js");

pub fn serve_dashboard(
    root: Option<&Path>,
    host: &str,
    port: u16,
    open: bool,
    global: bool,
) -> Result<()> {
    let bind_addr = format!("{host}:{port}");
    let listener = TcpListener::bind(&bind_addr).map_err(|e| {
        AwError::Internal(format!(
            "could not bind dashboard server to {bind_addr}: {e}"
        ))
    })?;

    let url = format!("http://{host}:{port}/");
    println!("aw: live dashboard running at {url}");
    println!("Press Ctrl+C to stop.");

    if open {
        open_browser(&url);
    }

    let log_path = if global {
        db::global_usage_log_path().unwrap_or_else(|_| PathBuf::from("usage.log"))
    } else if let Some(r) = root {
        crate::journal::usage_log_path(r)
    } else if let Ok(cwd) = std::env::current_dir() {
        crate::journal::usage_log_path(&cwd)
    } else {
        db::global_usage_log_path().unwrap_or_else(|_| PathBuf::from("usage.log"))
    };

    let root_path = root.map(|r| r.to_path_buf());
    let running = Arc::new(AtomicBool::new(true));

    for stream in listener.incoming() {
        if !running.load(Ordering::SeqCst) {
            break;
        }
        match stream {
            Ok(stream) => {
                let log = log_path.clone();
                let r_path = root_path.clone();
                thread::spawn(move || {
                    let _ = handle_client(stream, &log, r_path.as_deref(), global);
                });
            }
            Err(_) => {
                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    Ok(())
}

fn open_browser(url: &str) {
    let (cmd, args) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(target_os = "windows") {
        ("cmd", vec!["/C", "start", url])
    } else {
        ("xdg-open", vec![url])
    };
    let _ = std::process::Command::new(cmd).args(&args).spawn();
}

fn handle_client(
    mut stream: TcpStream,
    log_path: &Path,
    root: Option<&Path>,
    global: bool,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(());
    }
    let method = parts[0];
    let path = parts[1];

    // Read remaining headers until empty line
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
    }

    if method != "GET" && method != "HEAD" {
        let resp = "HTTP/1.1 405 Method Not Allowed\r\nConnection: close\r\n\r\n";
        stream.write_all(resp.as_bytes())?;
        return Ok(());
    }

    match path {
        "/" | "/index.html" => {
            send_response(
                &mut stream,
                200,
                "text/html; charset=utf-8",
                INDEX_HTML.as_bytes(),
            )?;
        }
        "/style.css" => {
            send_response(
                &mut stream,
                200,
                "text/css; charset=utf-8",
                STYLE_CSS.as_bytes(),
            )?;
        }
        "/app.js" => {
            send_response(
                &mut stream,
                200,
                "text/javascript; charset=utf-8",
                APP_JS.as_bytes(),
            )?;
        }
        "/favicon.ico" => {
            let resp = "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n";
            stream.write_all(resp.as_bytes())?;
        }
        "/api/summary" | "/api/metrics" => {
            let entries = data::read_usage_entries(log_path);
            let summary = data::compute_summary(&entries);
            let encoded = serde_json::to_vec_pretty(&summary).unwrap_or_default();
            send_response(
                &mut stream,
                200,
                "application/json; charset=utf-8",
                &encoded,
            )?;
        }
        "/api/reuse" => {
            let entries = data::read_usage_entries(log_path);
            let (candidates, summary, prewrite) = data::collect_reuse_candidates(root, &entries)
                .unwrap_or_else(|_| (Vec::new(), serde_json::json!({}), Vec::new()));
            let payload = serde_json::json!({
                "candidates": candidates,
                "summary": summary,
                "prewrite_searches": prewrite,
            });
            let encoded = serde_json::to_vec_pretty(&payload).unwrap_or_default();
            send_response(
                &mut stream,
                200,
                "application/json; charset=utf-8",
                &encoded,
            )?;
        }
        "/api/memories" => {
            let entries = data::read_usage_entries(log_path);
            let notes = data::collect_memories(root, global, &entries).unwrap_or_default();
            let encoded = serde_json::to_vec_pretty(&notes).unwrap_or_default();
            send_response(
                &mut stream,
                200,
                "application/json; charset=utf-8",
                &encoded,
            )?;
        }
        "/events" => {
            stream.set_read_timeout(None)?;
            stream.set_write_timeout(Some(Duration::from_secs(30)))?;
            let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nAccess-Control-Allow-Origin: *\r\n\r\n";
            stream.write_all(header.as_bytes())?;
            stream.flush()?;

            let mut last_pos = if let Ok(meta) = std::fs::metadata(log_path) {
                meta.len()
            } else {
                0
            };

            let mut last_keepalive = Instant::now();
            loop {
                let now = Instant::now();
                if let Ok(file) = File::open(log_path) {
                    if let Ok(meta) = file.metadata() {
                        let cur_len = meta.len();
                        if cur_len < last_pos {
                            last_pos = 0;
                        }
                        if cur_len > last_pos {
                            let mut buf_reader = BufReader::new(file);
                            if buf_reader.seek(SeekFrom::Start(last_pos)).is_ok() {
                                let mut event_line = String::new();
                                while let Ok(n) = buf_reader.read_line(&mut event_line) {
                                    if n == 0 {
                                        break;
                                    }
                                    let trimmed = event_line.trim();
                                    if !trimmed.is_empty() {
                                        let msg = format!("data: {trimmed}\n\n");
                                        if stream.write_all(msg.as_bytes()).is_err() {
                                            return Ok(());
                                        }
                                        let _ = stream.flush();
                                        last_keepalive = now;
                                    }
                                    event_line.clear();
                                }
                                if let Ok(pos) = buf_reader.stream_position() {
                                    last_pos = pos;
                                }
                            }
                        }
                    }
                }

                if now.duration_since(last_keepalive) >= Duration::from_secs(15) {
                    if stream.write_all(b": keepalive\n\n").is_err() {
                        return Ok(());
                    }
                    let _ = stream.flush();
                    last_keepalive = now;
                }

                thread::sleep(Duration::from_millis(500));
            }
        }
        _ => {
            let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream.write_all(resp.as_bytes())?;
        }
    }

    Ok(())
}

fn send_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let header = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}
