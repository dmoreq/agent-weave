//! Dashboard touch tests.

use super::*;

#[test]
fn dashboard_api_memory_touch_success_and_error() {
    let repo = super::super::common::temp_repo("dash-touch-api");
    super::super::common::mem_write(&repo, "touch-key", "body content", "gotcha");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let temp_log = repo.join("usage.log");
    let log_clone = temp_log.clone();
    let repo_clone = repo.clone();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let _ = handle_client(stream, &log_clone, Some(&repo_clone), false);
    });

    let store_id = std::fs::canonicalize(repo.join(".engram/memory.db"))
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let mut client = TcpStream::connect(addr).unwrap();
    let body = format!(r#"{{"key":"touch-key","store_id":"{store_id}","rev":1}}"#);
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
    assert!(resp.contains("\"store_id\":"), "{resp}");
    handle.join().unwrap();

    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn dashboard_touch_rejects_bad_content_type_stale_rev_and_unknown_store() {
    let repo = super::super::common::temp_repo("dash-touch-status");
    super::super::common::mem_write(&repo, "touch-key", "body content", "gotcha");
    let store_id = std::fs::canonicalize(repo.join(".engram/memory.db"))
        .unwrap()
        .to_string_lossy()
        .into_owned();

    let send = |body: String, content_type: &str, expect: &str| {
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
        let req = format!(
            "POST /api/memory/touch HTTP/1.1\r\nHost: localhost\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        client.write_all(req.as_bytes()).unwrap();
        let mut resp = String::new();
        client.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with(expect), "{resp}");
        handle.join().unwrap();
    };

    // Missing/incorrect Content-Type -> 415.
    send(
        format!(r#"{{"key":"touch-key","store_id":"{store_id}","rev":1}}"#),
        "text/plain",
        "HTTP/1.1 415",
    );
    // Stale revision -> 409 with the current rev named.
    send(
        format!(r#"{{"key":"touch-key","store_id":"{store_id}","rev":99}}"#),
        "application/json",
        "HTTP/1.1 409",
    );
    // Unknown store -> 404, never a silent cross-store fallback.
    send(
        r#"{"key":"touch-key","store_id":"/nonexistent/store/memory.db","rev":1}"#.to_string(),
        "application/json",
        "HTTP/1.1 404",
    );
    let _ = std::fs::remove_dir_all(&repo);
}
