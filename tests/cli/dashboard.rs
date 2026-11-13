//! Integration tests: dashboard section.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Instant;

use engram::dashboard::server::{handle_client, poll_sse_file};

use super::common::*;

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
fn dashboard_api_summary_returns_agent_aggs() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let temp_dir = std::env::temp_dir().join(format!("test_summary_dir_{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let temp_log = temp_dir.join("usage.log");
    // Agent rollups also obey the live-repository rule: the fixture needs a
    // real root for the entries to count.
    let repo = temp_dir.to_string_lossy().into_owned();
    let line1 = format!(
        r#"{{"tool":"symbols","exit_code":0,"agent":"antigravity","repo":"{repo}","latency_us":1000}}"#
    );
    let line2 = format!(
        r#"{{"tool":"build","exit_code":0,"agent":"user","repo":"{repo}","latency_us":2000}}"#
    );
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
fn dashboard_api_summary_returns_reuse_and_memory_aggs() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let temp_dir =
        std::env::temp_dir().join(format!("test_summary_reuse_mem_{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    // Repository rows are a live inventory, so the journaled roots must exist.
    let repo_a_dir = temp_dir.join("repo-a");
    let repo_b_dir = temp_dir.join("repo-b");
    std::fs::create_dir_all(&repo_a_dir).unwrap();
    std::fs::create_dir_all(&repo_b_dir).unwrap();
    let (repo_a, repo_b) = (
        repo_a_dir.to_string_lossy().into_owned(),
        repo_b_dir.to_string_lossy().into_owned(),
    );
    let temp_log = temp_dir.join("usage.log");
    let line1 = format!(
        r#"{{"tool":"reuse","exit_code":0,"agent":"claude","repo":"{repo_a}","repo_name":"repo-a","result_count":3,"latency_us":1000,"worktree":true}}"#
    );
    let line2 = format!(
        r#"{{"tool":"search","exit_code":0,"agent":"claude","repo":"{repo_a}","repo_name":"repo-a","shown":2,"latency_us":1500}}"#
    );
    let line3 = format!(
        r#"{{"tool":"reuse","exit_code":6,"agent":"claude","repo":"{repo_a}","repo_name":"repo-a","latency_us":500}}"#
    );
    let line4 = format!(
        r#"{{"tool":"mem_query","exit_code":0,"agent":"antigravity","repo":"{repo_b}","repo_name":"repo-b","latency_us":800}}"#
    );
    let line5 = format!(
        r#"{{"tool":"mem_write","exit_code":0,"agent":"antigravity","repo":"{repo_b}","repo_name":"repo-b","latency_us":1200}}"#
    );
    let line6 = format!(
        r#"{{"tool":"mem_get","exit_code":6,"agent":"antigravity","repo":"{repo_b}","repo_name":"repo-b","latency_us":400}}"#
    );
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

    let agg_a = &json_val["repo_aggs"][&repo_a];
    assert_eq!(agg_a["display"], "repo-a");
    assert_eq!(agg_a["reuse_calls"], 3);
    assert_eq!(agg_a["reuse_errors"], 1);
    assert_eq!(agg_a["reuse_hits"], 3);
    assert_eq!(agg_a["worktree"], true);

    let agg_b = &json_val["repo_aggs"][&repo_b];
    assert_eq!(agg_b["display"], "repo-b");
    assert_eq!(agg_b["memory_calls"], 3);
    assert_eq!(agg_b["memory_errors"], 1);
    assert_eq!(agg_b["memory_reads"], 2);
    assert_eq!(agg_b["memory_writes"], 1);

    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn dashboard_summary_drops_deleted_repo_rows() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let temp_dir = std::env::temp_dir().join(format!("test_summary_stale_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let temp_log = temp_dir.join("usage.log");
    // Everything without a live repository vanishes: dead absolute roots (a
    // scratch repo, an ordinary deleted repo) and the identityless legacy
    // relative line alike. No unattributed bucket remains.
    let gone = temp_dir.join("eg-reuse-bench-gone");
    let deleted = r#"{"tool":"symbols","exit_code":0,"repo":"/nonexistent-eg-live/archived","repo_name":"archived-repo","latency_us":1000}"#;
    let stale = format!(
        r#"{{"tool":"build","exit_code":0,"repo":"{}","repo_name":"eg-reuse-bench-gone","latency_us":2000}}"#,
        gone.display()
    );
    let legacy = r#"{"tool":"build","exit_code":0,"repo":".tmp/verify/repo1","repo_name":"repo1","latency_us":3000}"#;
    std::fs::write(&temp_log, format!("{deleted}\n{stale}\n{legacy}\n")).unwrap();

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
    assert_eq!(json_val["total_calls"], 0, "{json_val}");
    let aggs = json_val["repo_aggs"].as_object().unwrap();
    assert_eq!(aggs.len(), 0, "{json_val}");
    assert!(
        json_val["repo_aggs"]["/nonexistent-eg-live/archived"].is_null(),
        "{json_val}"
    );
    // The graph window obeys the same rule: nothing without a live repo survives.
    let rows = json_val["graph_recent_queries"].as_array().unwrap();
    assert_eq!(rows.len(), 0, "{json_val}");
    assert!(
        json_val.get("recent_queries").is_none(),
        "dead field removed"
    );
    let body = resp[body_start..].to_string();
    assert!(!body.contains("archived-repo"), "{body}");
    assert!(!body.contains("eg-reuse-bench-gone"), "{body}");
    assert!(!body.contains("repo1"), "{body}");
    assert!(!body.contains("unattributed"), "{body}");
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&temp_dir);
}

mod monitor;
mod security;
mod sse;
mod touch;
