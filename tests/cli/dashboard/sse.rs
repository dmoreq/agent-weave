//! Dashboard sse tests.

use super::*;

#[test]
fn dashboard_sse_poll_propagates_write_error_on_disconnected_client() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let client = TcpStream::connect(addr).unwrap();
    let (mut server_stream, _) = listener.accept().unwrap();
    // Drop client to disconnect
    drop(client);

    // Frames must carry a live repository to be forwarded at all; the writes
    // then hit the disconnected stream and the error must propagate.
    let repo = std::env::temp_dir().to_string_lossy().into_owned();
    let temp_log = std::env::temp_dir().join(format!("test_sse_{}.log", std::process::id()));
    let mut content = String::new();
    for i in 0..500 {
        content.push_str(&format!(
            "{{\"ts\":{},\"tool\":\"build\",\"repo\":\"{repo}\",\"entry\":\"test\"}}\n",
            i
        ));
    }
    std::fs::write(&temp_log, content).unwrap();
    let mut tail = engram::dashboard::data::Tail::starting_at(&temp_log, 0);
    let mut last_keepalive = Instant::now();
    let res = poll_sse_file(
        &mut server_stream,
        &mut tail,
        &temp_log,
        &mut last_keepalive,
        Instant::now(),
    );
    assert!(res.is_err(), "must propagate I/O error on broken stream");
    let _ = std::fs::remove_file(&temp_log);
}

#[test]
fn dashboard_sse_forwards_only_live_frames() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let client = TcpStream::connect(addr).unwrap();
    let (mut server_stream, _) = listener.accept().unwrap();

    let temp_dir = std::env::temp_dir().join(format!("test_sse_norm_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    // Only the live root is forwarded: dead roots and identityless legacy
    // lines are dropped by the same predicate every reader uses.
    let live = temp_dir.to_string_lossy().into_owned();
    let temp_log = temp_dir.join("usage.log");
    std::fs::write(
        &temp_log,
        format!(
            "{{\"tool\":\"build\",\"repo\":\"{live}\",\"repo_name\":\"live-repo\"}}\n{{\"tool\":\"build\",\"repo\":\"/nonexistent-eg-sse/dead\",\"repo_name\":\"agent-skills\"}}\n{{\"tool\":\"build\",\"repo\":\".tmp/verify/repo1\",\"repo_name\":\"repo1\",\"worktree\":true}}\n"
        ),
    )
    .unwrap();
    let mut tail = engram::dashboard::data::Tail::starting_at(&temp_log, 0);
    let mut last_keepalive = Instant::now();
    poll_sse_file(
        &mut server_stream,
        &mut tail,
        &temp_log,
        &mut last_keepalive,
        Instant::now(),
    )
    .unwrap();
    drop(server_stream);

    let mut resp = String::new();
    let mut client = client;
    client.read_to_string(&mut resp).unwrap();
    let frames: Vec<serde_json::Value> = resp
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .map(|payload| serde_json::from_str::<serde_json::Value>(payload).unwrap())
        .collect();
    assert_eq!(frames.len(), 1, "{resp}");
    assert_eq!(frames[0]["repo"], live, "{resp}");
    assert_eq!(frames[0]["repo_name"], "live-repo", "{resp}");
    assert!(!resp.contains("agent-skills"), "{resp}");
    assert!(!resp.contains("repo1"), "{resp}");
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn dashboard_sse_emits_named_reset_on_shrink() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let client = TcpStream::connect(addr).unwrap();
    let (mut server_stream, _) = listener.accept().unwrap();

    let temp_dir = std::env::temp_dir().join(format!("test_sse_reset_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let log = temp_dir.join("usage.log");
    let repo = temp_dir.to_string_lossy().into_owned();
    std::fs::write(
        &log,
        format!("{{\"tool\":\"build\",\"repo\":\"{repo}\",\"repo_name\":\"r\"}}\n"),
    )
    .unwrap();

    // Consume the backlog, then prune the log in place (same inode, smaller).
    let mut tail = engram::dashboard::data::Tail::starting_at(&log, 0);
    let mut keepalive = Instant::now();
    poll_sse_file(
        &mut server_stream,
        &mut tail,
        &log,
        &mut keepalive,
        Instant::now(),
    )
    .unwrap();
    std::fs::write(&log, "").unwrap();
    poll_sse_file(
        &mut server_stream,
        &mut tail,
        &log,
        &mut keepalive,
        Instant::now(),
    )
    .unwrap();
    drop(server_stream);

    let mut resp = String::new();
    let mut client = client;
    client.read_to_string(&mut resp).unwrap();
    assert_eq!(
        resp.matches("data: {\"repo\"").count(),
        1,
        "one backlog frame, no replay: {resp}"
    );
    assert!(
        resp.contains("event: reset"),
        "shrink must emit the named reset event: {resp}"
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}
