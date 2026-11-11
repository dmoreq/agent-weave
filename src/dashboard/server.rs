//! Native synchronous HTTP 1.1 and Server-Sent Events server in Rust stdlib.
//! Zero async runtimes, zero external web crates, zero Python dependency.

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub(crate) mod http;
pub(crate) mod touch;

use crate::dashboard::data;
use crate::db;
use crate::error::{EgError, Result};
use http::{host_allowed, read_http_request, send_response, RequestLog};

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
    if !matches!(host, "127.0.0.1" | "localhost") {
        return Err(EgError::Usage(format!(
            "dashboard binds loopback only (got --host {host}); telemetry carries local paths"
        )));
    }
    let bind_addr = format!("{host}:{port}");
    let listener = TcpListener::bind(&bind_addr).map_err(|e| {
        EgError::Internal(format!(
            "could not bind dashboard server to {bind_addr}: {e}"
        ))
    })?;

    let url = format!("http://{host}:{port}/");
    println!("eg: live dashboard running at {url}");
    println!("Press Ctrl+C to stop.");

    if open {
        open_browser(&url);
    }

    // `--local` must isolate one repository: resolve its root once, at
    // startup, instead of letting discovery scan every journaled repo.
    let root_path: Option<PathBuf> = if local && root.is_none() {
        match crate::walk::discover_root(None) {
            Ok(root) => Some(root),
            Err(e) => {
                return Err(EgError::Usage(format!(
                    "--local needs a resolvable repository root: {e}"
                )))
            }
        }
    } else {
        root.map(Path::to_path_buf)
    };

    let log_path = if local {
        let r = root_path
            .as_deref()
            .ok_or_else(|| EgError::Usage("--local needs a repository root".to_string()))?;
        crate::journal::usage_log_path(r)
    } else {
        db::global_usage_log_path().unwrap_or_else(|_| PathBuf::from("usage.log"))
    };

    let _ = crate::journal::prune_stale_entries_all(&log_path);
    let mode = if local { "local" } else { "global" };
    eprintln!(
        "eg dashboard: mode={mode} root={} log={}",
        root_path
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "-".to_string()),
        log_path.display()
    );

    let _ = crate::journal::prune_stale_entries_all(&log_path);

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

/// One parsed HTTP request. Header totals are bounded so a hostile local page
/// cannot hold a thread with an endless header block.
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
    _global: bool,
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
            let payload = data::collect_memories(root, &entries).unwrap_or_else(|e| {
                serde_json::json!({
                    "schema_version": 2, "notes": [], "degraded": true,
                    "errors": [{ "store": "-", "stage": "collect", "message": e.to_string() }]
                })
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
        _ => Ok(false),
    }
}

fn stream_sse_events(stream: &mut TcpStream, log_path: &Path) -> std::io::Result<()> {
    stream.set_read_timeout(None)?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    // Same-origin only: the stream carries repository paths and search terms.
    let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
    stream.write_all(header.as_bytes())?;
    stream.flush()?;

    let mut tail = data::Tail::new(log_path);
    let mut last_keepalive = Instant::now();

    loop {
        let now = Instant::now();
        poll_sse_file(stream, &mut tail, log_path, &mut last_keepalive, now)?;

        if now.duration_since(last_keepalive) >= Duration::from_secs(15) {
            stream.write_all(b": keepalive\n\n")?;
            stream.flush()?;
            last_keepalive = now;
        }

        thread::sleep(Duration::from_millis(500));
    }
}

/// Pump one tail poll to the SSE client. A shrink or rotation emits the named
/// `reset` event and no replayed frames.
pub fn poll_sse_file(
    stream: &mut TcpStream,
    tail: &mut data::Tail,
    log_path: &Path,
    last_keepalive: &mut Instant,
    now: Instant,
) -> std::io::Result<()> {
    let mut frames: Vec<String> = Vec::new();
    let mut reset = false;
    tail.poll(log_path, |event| match event {
        data::TailEvent::Reset => reset = true,
        data::TailEvent::Line(line) => {
            if let Some(val) = data::live_event(line) {
                if let Ok(payload) = serde_json::to_string(&val) {
                    frames.push(format!("data: {payload}\n\n"));
                }
            }
        }
    });
    if reset {
        // Named event: a pre-upgrade tab's default onmessage handler ignores
        // it instead of treating it as a journal row.
        stream.write_all(b"event: reset\ndata: {}\n\n")?;
        *last_keepalive = now;
    }
    for frame in frames {
        stream.write_all(frame.as_bytes())?;
        *last_keepalive = now;
    }
    stream.flush()
}

pub fn handle_client(
    mut stream: TcpStream,
    log_path: &Path,
    root: Option<&Path>,
    global: bool,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let Some(req) = read_http_request(&mut stream)? else {
        return Ok(());
    };
    let method = req.method.as_str();
    let path = req.path.split('?').next().unwrap_or(&req.path);
    // One stderr line per request: the only way to see a stuck or failing
    // route on a console dashboard.
    let _log = RequestLog {
        method: method.to_string(),
        path: path.to_string(),
        started: Instant::now(),
    };
    if !host_allowed(req.host.as_deref()) {
        let resp = "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        stream.write_all(resp.as_bytes())?;
        return Ok(());
    }

    if method == "POST" && path == "/api/memory/touch" {
        return touch::handle_memory_touch(&mut stream, root, log_path, &req);
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
        if SSE_CLIENTS.fetch_add(1, Ordering::SeqCst) >= MAX_SSE_CLIENTS {
            SSE_CLIENTS.fetch_sub(1, Ordering::SeqCst);
            let resp = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream.write_all(resp.as_bytes())?;
            return Ok(());
        }
        let _guard = SseGuard;
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

static SSE_CLIENTS: AtomicUsize = AtomicUsize::new(0);
const MAX_SSE_CLIENTS: usize = 8;

/// Releases the SSE slot when the connection thread ends.
struct SseGuard;

impl Drop for SseGuard {
    fn drop(&mut self) {
        SSE_CLIENTS.fetch_sub(1, Ordering::SeqCst);
    }
}
