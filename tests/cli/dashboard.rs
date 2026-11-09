//! Integration tests: dashboard section.

use std::fs::File;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Instant;

use agent_weave::dashboard::server::{handle_client, poll_sse_file};

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
