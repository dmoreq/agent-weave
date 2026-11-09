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
    local: bool,
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

    let log_path = if local {
        if let Some(r) = root {
            crate::journal::usage_log_path(r)
        } else if let Ok(cwd) = std::env::current_dir() {
            crate::journal::usage_log_path(&cwd)
        } else {
            db::global_usage_log_path().unwrap_or_else(|_| PathBuf::from("usage.log"))
        }
    } else {
        db::global_usage_log_path().unwrap_or_else(|_| PathBuf::from("usage.log"))
    };

    let _ = crate::journal::prune_deleted_worktrees_all(&log_path);

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
                let is_global = !local;
                thread::spawn(move || {
                    let _ = handle_client(stream, &log, r_path.as_deref(), is_global);
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

fn read_http_request(stream: &mut TcpStream) -> std::io::Result<Option<(String, String, Vec<u8>)>> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(None);
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(None);
    }
    let method = parts[0].to_string();
    let path = parts[1].to_string();

    let mut content_length = 0usize;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        let lower = line.to_lowercase();
        if lower.starts_with("content-length:") {
            if let Some(val) = line.split(':').nth(1) {
                content_length = val.trim().parse().unwrap_or(0);
            }
        }
    }

    let mut body = Vec::new();
    if content_length > 0 && content_length < 1_000_000 {
        use std::io::Read;
        body.resize(content_length, 0);
        reader.read_exact(&mut body)?;
    }

    Ok(Some((method, path, body)))
}

fn serve_static_asset(stream: &mut TcpStream, path: &str, is_head: bool) -> std::io::Result<bool> {
    match path {
        "/" | "/index.html" => {
            send_response(
                stream,
                200,
                "text/html; charset=utf-8",
                INDEX_HTML.as_bytes(),
                is_head,
            )?;
            Ok(true)
        }
        "/style.css" => {
            send_response(
                stream,
                200,
                "text/css; charset=utf-8",
                STYLE_CSS.as_bytes(),
                is_head,
            )?;
            Ok(true)
        }
        "/app.js" => {
            send_response(
                stream,
                200,
                "text/javascript; charset=utf-8",
                APP_JS.as_bytes(),
                is_head,
            )?;
            Ok(true)
        }
        "/favicon.ico" => {
            let resp = "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n";
            stream.write_all(resp.as_bytes())?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn serve_api_route(
    stream: &mut TcpStream,
    path: &str,
    log_path: &Path,
    root: Option<&Path>,
    global: bool,
    is_head: bool,
) -> std::io::Result<bool> {
    match path {
        "/api/summary" | "/api/metrics" => {
            let entries = data::read_usage_entries(log_path);
            let summary = data::compute_summary(&entries);
            let encoded = serde_json::to_vec_pretty(&summary).unwrap_or_default();
            send_response(
                stream,
                200,
                "application/json; charset=utf-8",
                &encoded,
                is_head,
            )?;
            Ok(true)
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
                stream,
                200,
                "application/json; charset=utf-8",
                &encoded,
                is_head,
            )?;
            Ok(true)
        }
        "/api/memories" => {
            let entries = data::read_usage_entries(log_path);
            let notes = data::collect_memories(root, global, &entries).unwrap_or_default();
            let encoded = serde_json::to_vec_pretty(&notes).unwrap_or_default();
            send_response(
                stream,
                200,
                "application/json; charset=utf-8",
                &encoded,
                is_head,
            )?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn stream_sse_events(stream: &mut TcpStream, log_path: &Path) -> std::io::Result<()> {
    stream.set_read_timeout(None)?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nAccess-Control-Allow-Origin: *\r\n\r\n";
    stream.write_all(header.as_bytes())?;
    stream.flush()?;

    let mut last_pos = std::fs::metadata(log_path).map(|m| m.len()).unwrap_or(0);
    let mut last_keepalive = Instant::now();

    loop {
        let now = Instant::now();
        if let Ok(file) = File::open(log_path) {
            poll_sse_file(stream, file, &mut last_pos, &mut last_keepalive, now)?;
        }

        if now.duration_since(last_keepalive) >= Duration::from_secs(15) {
            stream.write_all(b": keepalive\n\n")?;
            stream.flush()?;
            last_keepalive = now;
        }

        thread::sleep(Duration::from_millis(500));
    }
}

pub fn poll_sse_file(
    stream: &mut TcpStream,
    file: File,
    last_pos: &mut u64,
    last_keepalive: &mut Instant,
    now: Instant,
) -> std::io::Result<()> {
    let Ok(meta) = file.metadata() else {
        return Ok(());
    };
    let cur_len = meta.len();
    if cur_len < *last_pos {
        *last_pos = 0;
    }
    if cur_len <= *last_pos {
        return Ok(());
    }

    let mut buf_reader = BufReader::new(file);
    if buf_reader.seek(SeekFrom::Start(*last_pos)).is_err() {
        return Ok(());
    }

    let mut event_line = String::new();
    while let Ok(n) = buf_reader.read_line(&mut event_line) {
        if n == 0 {
            break;
        }
        let trimmed = event_line.trim();
        if !trimmed.is_empty() {
            let msg = format!("data: {trimmed}\n\n");
            stream.write_all(msg.as_bytes())?;
            stream.flush()?;
            *last_keepalive = now;
        }
        event_line.clear();
    }
    if let Ok(pos) = buf_reader.stream_position() {
        *last_pos = pos;
    }
    Ok(())
}

pub fn handle_client(
    mut stream: TcpStream,
    log_path: &Path,
    root: Option<&Path>,
    global: bool,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let Some((method, raw_path, body)) = read_http_request(&mut stream)? else {
        return Ok(());
    };

    let path = raw_path.split('?').next().unwrap_or(&raw_path);

    if method == "POST" && path == "/api/memory/touch" {
        return handle_memory_touch(&mut stream, root, &body);
    }

    if method != "GET" && method != "HEAD" {
        let resp = "HTTP/1.1 405 Method Not Allowed\r\nConnection: close\r\n\r\n";
        stream.write_all(resp.as_bytes())?;
        return Ok(());
    }

    let is_head = method == "HEAD";

    if serve_static_asset(&mut stream, path, is_head)? {
        return Ok(());
    }

    if serve_api_route(&mut stream, path, log_path, root, global, is_head)? {
        return Ok(());
    }

    if path == "/events" {
        if is_head {
            let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n";
            stream.write_all(header.as_bytes())?;
            return stream.flush();
        }
        return stream_sse_events(&mut stream, log_path);
    }

    let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    stream.write_all(resp.as_bytes())
}

fn handle_memory_touch(
    stream: &mut TcpStream,
    root: Option<&Path>,
    body: &[u8],
) -> std::io::Result<()> {
    let val: serde_json::Value = serde_json::from_slice(body).unwrap_or(serde_json::Value::Null);
    let key = val
        .get("key")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let is_global = val
        .get("global")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if key.is_empty() {
        let resp_body = serde_json::json!({ "ok": false, "error": "missing key" });
        let encoded = serde_json::to_vec(&resp_body).unwrap_or_default();
        return send_response(
            stream,
            400,
            "application/json; charset=utf-8",
            &encoded,
            false,
        );
    }

    let now = crate::db::unix_now();
    let mut touched_rev = None;

    if is_global {
        if let Ok(mut conn) = crate::db::open_global_memory_mutation() {
            if let Ok(rev) = crate::db::memory_touch(&mut conn, key, now) {
                touched_rev = rev;
            }
        }
    } else {
        if let Some(r) = root {
            if let Ok(mut conn) = crate::db::open_memory_mutation(r) {
                if let Ok(rev) = crate::db::memory_touch(&mut conn, key, now) {
                    touched_rev = rev;
                }
            }
        }
        if touched_rev.is_none() {
            if let Ok(mut conn) = crate::db::open_global_memory_mutation() {
                if let Ok(rev) = crate::db::memory_touch(&mut conn, key, now) {
                    touched_rev = rev;
                }
            }
        }
    }

    if let Some(rev) = touched_rev {
        let resp_body = serde_json::json!({ "ok": true, "key": key, "rev": rev });
        let encoded = serde_json::to_vec(&resp_body).unwrap_or_default();
        send_response(
            stream,
            200,
            "application/json; charset=utf-8",
            &encoded,
            false,
        )
    } else {
        let resp_body = serde_json::json!({ "ok": false, "error": format!("unknown key '{key}'") });
        let encoded = serde_json::to_vec(&resp_body).unwrap_or_default();
        send_response(
            stream,
            404,
            "application/json; charset=utf-8",
            &encoded,
            false,
        )
    }
}

fn send_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    is_head: bool,
) -> std::io::Result<()> {
    let status_line = match status {
        200 => "200 OK",
        400 => "400 Bad Request",
        404 => "404 Not Found",
        405 => "405 Method Not Allowed",
        _ => "200 OK",
    };
    let header = format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    if !is_head {
        stream.write_all(body)?;
    }
    stream.flush()
}
