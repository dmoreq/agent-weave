//! Integration tests: dashboard section.

use std::fs::File;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Instant;

use engram::dashboard::server::{handle_client, poll_sse_file};

#[test]
fn dashboard_query_string_stripped_and_head_method_suppresses_body() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let temp_log = std::env::temp_dir().join(format!("test_usage_{}.log", std::process::id()));
        let _ = handle_client(stream, &temp_log, None, false);
    });

    let mut client = TcpStream::connect(addr).unwrap();
    // 1. GET with query string: must strip query string for routing and return 200 OK
    client
        .write_all(b"GET /style.css?v=123 HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut resp = String::new();
    client.read_to_string(&mut resp).unwrap();
    assert!(resp.starts_with("HTTP/1.1 200 OK"), "{resp}");
    assert!(resp.contains("Content-Length:"), "{resp}");
    assert!(resp.contains("body {"), "{resp}");
    handle.join().unwrap();

    // 2. HEAD method suppresses response body per RFC 9110
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let temp_log = std::env::temp_dir().join(format!("test_usage_{}.log", std::process::id()));
        let _ = handle_client(stream, &temp_log, None, false);
    });

    let mut client = TcpStream::connect(addr).unwrap();
    client
        .write_all(b"HEAD /style.css HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut resp = Vec::new();
    client.read_to_end(&mut resp).unwrap();
    let resp_str = String::from_utf8_lossy(&resp);
    assert!(resp_str.starts_with("HTTP/1.1 200 OK"), "{resp_str}");
    assert!(resp_str.contains("Content-Length:"), "{resp_str}");
    assert!(
        resp_str.ends_with("\r\n\r\n"),
        "HEAD must have no body: {resp_str}"
    );
    handle.join().unwrap();

    // 3. HEAD /events terminates immediately without hanging
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let temp_log = std::env::temp_dir().join(format!("test_usage_{}.log", std::process::id()));
        let _ = handle_client(stream, &temp_log, None, false);
    });

    let mut client = TcpStream::connect(addr).unwrap();
    client
        .write_all(b"HEAD /events HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut resp = Vec::new();
    client.read_to_end(&mut resp).unwrap();
    let resp_str = String::from_utf8_lossy(&resp);
    assert!(resp_str.starts_with("HTTP/1.1 200 OK"), "{resp_str}");
    assert!(resp_str.contains("text/event-stream"), "{resp_str}");
    assert!(resp_str.ends_with("\r\n\r\n"), "{resp_str}");
    handle.join().unwrap();
}

#[test]
fn dashboard_sse_poll_propagates_write_error_on_disconnected_client() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let client = TcpStream::connect(addr).unwrap();
    let (mut server_stream, _) = listener.accept().unwrap();
    // Drop client to disconnect
    drop(client);

    let temp_log = std::env::temp_dir().join(format!("test_sse_{}.log", std::process::id()));
    let mut content = String::new();
    for i in 0..500 {
        content.push_str(&format!("{{\"ts\":{},\"entry\":\"test\"}}\n", i));
    }
    std::fs::write(&temp_log, content).unwrap();
    let file = File::open(&temp_log).unwrap();

    let mut last_pos = 0;
    let mut last_keepalive = Instant::now();
    let res = poll_sse_file(
        &mut server_stream,
        file,
        &mut last_pos,
        &mut last_keepalive,
        Instant::now(),
    );
    assert!(res.is_err(), "must propagate I/O error on broken stream");
    let _ = std::fs::remove_file(&temp_log);
}

#[test]
fn dashboard_api_summary_returns_agent_aggs() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let temp_dir = std::env::temp_dir().join(format!("test_summary_dir_{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let temp_log = temp_dir.join("usage.log");
    let line1 = r#"{"tool":"symbols","exit_code":0,"agent":"antigravity","latency_us":1000}"#;
    let line2 = r#"{"tool":"build","exit_code":0,"agent":"user","latency_us":2000}"#;
    std::fs::write(&temp_log, format!("{line1}\n{line2}\n")).unwrap();

    let log_clone = temp_log.clone();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let _ = handle_client(stream, &log_clone, None, false);
    });

    let mut client = TcpStream::connect(addr).unwrap();
    client
        .write_all(b"GET /api/summary HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut resp = String::new();
    client.read_to_string(&mut resp).unwrap();
    assert!(resp.starts_with("HTTP/1.1 200 OK"), "{resp}");
    assert!(resp.contains("\"agent_aggs\""), "{resp}");
    assert!(resp.contains("\"antigravity\""), "{resp}");
    assert!(resp.contains("\"user\""), "{resp}");
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn dashboard_api_memory_touch_success_and_error() {
    let repo = super::common::temp_repo("dash-touch-api");
    super::common::mem_write(&repo, "touch-key", "body content", "gotcha");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let temp_log = repo.join("usage.log");
    let log_clone = temp_log.clone();
    let repo_clone = repo.clone();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let _ = handle_client(stream, &log_clone, Some(&repo_clone), false);
    });

    let mut client = TcpStream::connect(addr).unwrap();
    let body = r#"{"key":"touch-key","global":false}"#;
    let req = format!(
        "POST /api/memory/touch HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    client.write_all(req.as_bytes()).unwrap();
    let mut resp = String::new();
    client.read_to_string(&mut resp).unwrap();
    assert!(resp.starts_with("HTTP/1.1 200 OK"), "{resp}");
    assert!(resp.contains("\"ok\":true"), "{resp}");
    assert!(resp.contains("\"rev\":1"), "{resp}");
    handle.join().unwrap();

    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn dashboard_api_summary_returns_reuse_and_memory_aggs() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let temp_dir =
        std::env::temp_dir().join(format!("test_summary_reuse_mem_{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let temp_log = temp_dir.join("usage.log");
    let line1 = r#"{"tool":"reuse","exit_code":0,"agent":"claude","repo_name":"repo-a","result_count":3,"latency_us":1000,"worktree":true}"#;
    let line2 = r#"{"tool":"search","exit_code":0,"agent":"claude","repo_name":"repo-a","shown":2,"latency_us":1500}"#;
    let line3 =
        r#"{"tool":"reuse","exit_code":6,"agent":"claude","repo_name":"repo-a","latency_us":500}"#;
    let line4 = r#"{"tool":"mem_query","exit_code":0,"agent":"antigravity","repo_name":"repo-b","latency_us":800}"#;
    let line5 = r#"{"tool":"mem_write","exit_code":0,"agent":"antigravity","repo_name":"repo-b","latency_us":1200}"#;
    let line6 = r#"{"tool":"mem_get","exit_code":6,"agent":"antigravity","repo_name":"repo-b","latency_us":400}"#;
    std::fs::write(
        &temp_log,
        format!("{line1}\n{line2}\n{line3}\n{line4}\n{line5}\n{line6}\n"),
    )
    .unwrap();

    let log_clone = temp_log.clone();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let _ = handle_client(stream, &log_clone, None, false);
    });

    let mut client = TcpStream::connect(addr).unwrap();
    client
        .write_all(b"GET /api/summary HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut resp = String::new();
    client.read_to_string(&mut resp).unwrap();
    assert!(resp.starts_with("HTTP/1.1 200 OK"), "{resp}");
    let body_start = resp.find("\r\n\r\n").unwrap() + 4;
    let json_val: serde_json::Value = serde_json::from_str(&resp[body_start..]).unwrap();
    let claude = &json_val["agent_aggs"]["claude"];
    assert_eq!(claude["reuse_calls"], 3);
    assert_eq!(claude["reuse_errors"], 1);
    assert_eq!(claude["reuse_hits"], 3);
    assert_eq!(claude["reuse_saved_tokens"], 4300);
    let claude_rel = claude["reuse_reliability_pct"].as_f64().unwrap();
    assert!((claude_rel - 66.666).abs() < 0.1);

    let antigravity = &json_val["agent_aggs"]["antigravity"];
    assert_eq!(antigravity["memory_calls"], 3);
    assert_eq!(antigravity["memory_errors"], 1);
    assert_eq!(antigravity["memory_reads"], 2);
    assert_eq!(antigravity["memory_writes"], 1);
    let ag_rel = antigravity["memory_reliability_pct"].as_f64().unwrap();
    assert!((ag_rel - 66.666).abs() < 0.1);

    let repo_a = &json_val["repo_aggs"]["repo-a"];
    assert_eq!(repo_a["reuse_calls"], 3);
    assert_eq!(repo_a["reuse_errors"], 1);
    assert_eq!(repo_a["reuse_hits"], 3);
    assert_eq!(repo_a["worktree"], true);

    let repo_b = &json_val["repo_aggs"]["repo-b"];
    assert_eq!(repo_b["memory_calls"], 3);
    assert_eq!(repo_b["memory_errors"], 1);
    assert_eq!(repo_b["memory_reads"], 2);
    assert_eq!(repo_b["memory_writes"], 1);

    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn monitor_tui_render_all_tabs_coverage() {
    use engram::dashboard::monitor::render::render_screen;
    use engram::dashboard::monitor::state::{MonitorTab, TuiState};

    let temp_dir = std::env::temp_dir().join(format!("test_tui_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let log_path = temp_dir.join("usage.log");
    std::fs::write(
        &log_path,
        r#"{"ts":1794123456,"tool":"callers","repo_name":"test-repo","agent":"test-agent","latency_us":1200,"exit_code":0,"args":{"symbol":"test_fn"}}
{"ts":1794123460,"tool":"search","repo_name":"test-repo","agent":"test-agent","latency_us":3400,"exit_code":0,"args":{"terms":"cache"}}
{"ts":1794123470,"tool":"mem_get","repo_name":"test-repo","agent":"test-agent","latency_us":900,"exit_code":0,"args":{"key":"arch:test"}}
"#,
    )
    .unwrap();

    let mut state = TuiState::new(None, log_path.clone(), None, false);
    assert_eq!(state.total_repos, 1);
    assert_eq!(state.recent_events.len(), 3);

    // Also test with real global log if it exists!
    let global_log =
        std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".engram/usage.log");
    if global_log.is_file() {
        let mut real_state = TuiState::new(None, global_log, None, false);
        real_state.refresh(None);
        for tab in [
            MonitorTab::Graph,
            MonitorTab::Reuse,
            MonitorTab::Memory,
            MonitorTab::Stream,
        ] {
            real_state.active_tab = tab;
            let mut buf = Vec::new();
            render_screen(&mut buf, &real_state, 80, 24).unwrap();
        }
    }

    for tab in [
        MonitorTab::Graph,
        MonitorTab::Reuse,
        MonitorTab::Memory,
        MonitorTab::Stream,
    ] {
        state.active_tab = tab;
        let mut buf = Vec::new();
        render_screen(&mut buf, &state, 80, 24).unwrap();
        let rendered = String::from_utf8_lossy(&buf);
        assert!(
            rendered.contains("eg monitor v0.5.3"),
            "Header missing in tab {tab:?}"
        );
        assert!(rendered.contains("Keys:"), "Footer missing in tab {tab:?}");
        match tab {
            MonitorTab::Graph => {
                assert!(rendered.contains("USP 1: DETERMINISTIC CODE GRAPH"));
                assert!(rendered.contains("callers"));
            }
            MonitorTab::Reuse => {
                assert!(rendered.contains("USP 2: REAL-TIME ANTI-REINVENTION"));
            }
            MonitorTab::Memory => {
                assert!(rendered.contains("USP 3: ZERO-TOKEN PERSISTENT PROJECT MEMORY"));
            }
            MonitorTab::Stream => {
                assert!(rendered.contains("[GRAPH ]"));
                assert!(rendered.contains("[REUSE ]"));
                assert!(rendered.contains("[MEMORY]"));
            }
        }
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn dashboard_summary_drops_deleted_temp_repo_rows() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let temp_dir = std::env::temp_dir().join(format!("test_summary_stale_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let temp_log = temp_dir.join("usage.log");
    // The stale row names a temp-root repository that does not exist. The live
    // row names an ordinary absent path, which keeps its history on purpose.
    let gone = temp_dir.join("eg-reuse-bench-gone");
    let live = r#"{"tool":"symbols","exit_code":0,"repo":"/nonexistent-eg-live/archived","repo_name":"archived-repo","latency_us":1000}"#;
    let stale = format!(
        r#"{{"tool":"build","exit_code":0,"repo":"{}","repo_name":"eg-reuse-bench-gone","latency_us":2000}}"#,
        gone.display()
    );
    std::fs::write(&temp_log, format!("{live}\n{stale}\n")).unwrap();

    let log_clone = temp_log.clone();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let _ = handle_client(stream, &log_clone, None, false);
    });

    let mut client = TcpStream::connect(addr).unwrap();
    client
        .write_all(b"GET /api/summary HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut resp = String::new();
    client.read_to_string(&mut resp).unwrap();
    assert!(resp.starts_with("HTTP/1.1 200 OK"), "{resp}");
    let body_start = resp.find("\r\n\r\n").unwrap() + 4;
    let json_val: serde_json::Value = serde_json::from_str(&resp[body_start..]).unwrap();
    assert_eq!(json_val["total_calls"], 1, "{json_val}");
    // Rows are keyed by the journaled path; the label lives in `display`.
    let archived = &json_val["repo_aggs"]["/nonexistent-eg-live/archived"];
    assert!(archived.is_object(), "{json_val}");
    assert_eq!(archived["display"], "archived-repo", "{json_val}");
    assert!(
        json_val["repo_aggs"]["eg-reuse-bench-gone"].is_null(),
        "{json_val}"
    );
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&temp_dir);
}
