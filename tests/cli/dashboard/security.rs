//! Dashboard security tests.

use super::*;

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
