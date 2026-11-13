//! Dashboard security tests.

use super::*;

fn local_request(repo: &Path, log: &Path) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let repo = repo.to_path_buf();
    let log = log.to_path_buf();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let _ = handle_client(stream, &log, Some(&repo), false);
    });
    let mut client = TcpStream::connect(addr).unwrap();
    client
        .write_all(b"GET /api/summary HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut resp = String::new();
    client.read_to_string(&mut resp).unwrap();
    handle.join().unwrap();
    resp
}

#[test]
fn local_dashboard_rejects_tracked_journal_at_request_time() {
    let repo = temp_repo("dashboard-tracked-request");
    write(
        &repo,
        ".engram/usage.log",
        "{\"tool\":\"build\",\"exit_code\":0,\"result_count\":1,\"shown\":1,\"repo\":\"/x\",\"ts\":1}\n",
    );
    let clean = local_request(&repo, &repo.join(".engram/usage.log"));
    assert!(clean.starts_with("HTTP/1.1 200"), "{clean}");
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "-f", ".engram/usage.log"]);
    let resp = local_request(&repo, &repo.join(".engram/usage.log"));
    assert!(resp.starts_with("HTTP/1.1 503"), "{resp}");
    assert!(resp.contains("git-tracked"), "{resp}");
}

#[cfg(unix)]
#[test]
fn local_dashboard_rejects_symlinked_journal_file() {
    let repo = temp_repo("dashboard-journal-symlink");
    let outside = temp_repo("dashboard-journal-symlink-outside");
    write(
        &outside,
        "usage.log",
        "{\"tool\":\"build\",\"exit_code\":0,\"result_count\":1,\"shown\":1,\"repo\":\"/x\",\"ts\":1}\n",
    );
    std::fs::create_dir_all(repo.join(".engram")).unwrap();
    std::os::unix::fs::symlink(outside.join("usage.log"), repo.join(".engram/usage.log")).unwrap();
    let resp = local_request(&repo, &repo.join(".engram/usage.log"));
    assert!(resp.starts_with("HTTP/1.1 503"), "{resp}");
    assert!(resp.contains("refusing unsafe journal"), "{resp}");
}

#[test]
fn dashboard_rejects_foreign_host_and_sends_security_headers() {
    // DNS rebinding: a public Host that resolves to loopback must be refused.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let _ = handle_client(
            stream,
            &std::path::PathBuf::from("/nonexistent/usage.log"),
            None,
            false,
        );
    });
    let mut client = TcpStream::connect(addr).unwrap();
    client
        .write_all(b"GET /api/summary HTTP/1.1\r\nHost: evil.example\r\n\r\n")
        .unwrap();
    let mut resp = String::new();
    client.read_to_string(&mut resp).unwrap();
    assert!(resp.starts_with("HTTP/1.1 403"), "{resp}");
    handle.join().unwrap();

    // Loopback Host passes and carries the defensive headers.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let _ = handle_client(
            stream,
            &std::path::PathBuf::from("/nonexistent/usage.log"),
            None,
            false,
        );
    });
    let mut client = TcpStream::connect(addr).unwrap();
    client
        .write_all(b"GET /api/summary HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut resp = String::new();
    client.read_to_string(&mut resp).unwrap();
    assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
    assert!(resp.contains("Content-Security-Policy:"), "{resp}");
    assert!(resp.contains("X-Content-Type-Options: nosniff"), "{resp}");
    assert!(resp.contains("frame-ancestors 'none'"), "{resp}");
    handle.join().unwrap();
}
