//! JSON-RPC protocol conformance: framing, parse errors, envelope validation,
//! and dynamic root binding.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

use super::common::{aw_cmd, fixture, read_response, rpc, send_raw, spawn};

#[test]
fn malformed_json_frame_returns_parse_error() {
    let repo = fixture("mcp-badjson");
    let build = aw_cmd(&repo).arg("build").output().unwrap();
    assert_eq!(build.status.code(), Some(0));
    let mut srv = spawn(&repo);
    srv.stdin.write_all(b"not json\n").unwrap();
    srv.stdin.flush().unwrap();
    let mut line = String::new();
    srv.reader.read_line(&mut line).unwrap();
    let resp: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(resp["error"]["code"], -32700, "{resp}");
}

#[test]
fn missing_required_argument_returns_invalid_params() {
    // R7 L4: a schema-required string must fail at the protocol layer, not as
    // a downstream "symbol '' not in index" tool result.
    let repo = fixture("mcp-required");
    let build = aw_cmd(&repo).arg("build").output().unwrap();
    assert_eq!(build.status.code(), Some(0));
    let mut srv = spawn(&repo);
    let resp = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({"name": "callers", "arguments": {}}),
    );
    assert_eq!(resp["error"]["code"], -32602, "{resp}");
    assert!(
        resp["error"]["message"]
            .as_str()
            .unwrap()
            .contains("'name'"),
        "{resp}"
    );
    // A blank string is equally invalid.
    let resp = rpc(
        &mut srv,
        2,
        "tools/call",
        json!({"name": "search", "arguments": {"terms": "   "}}),
    );
    assert_eq!(resp["error"]["code"], -32602, "{resp}");
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn mcp_framing_survives_invalid_utf8_and_oversized_frames() {
    let repo = fixture("mcp-framing");
    let mut srv = spawn(&repo);

    // Invalid UTF-8 must produce a parse error, not terminate the session.
    srv.stdin.write_all(&[0xFF, b'\n']).unwrap();
    srv.stdin.flush().unwrap();
    let mut invalid = String::new();
    srv.reader.read_line(&mut invalid).unwrap();
    let err: Value = serde_json::from_str(&invalid).unwrap();
    assert_eq!(err["error"]["code"], -32700, "{invalid}");

    let init = rpc(
        &mut srv,
        2,
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}),
    );
    assert!(init["result"].is_object(), "{init}");

    // Reject an oversized frame without consuming the following request.
    for id in [3u64, 4] {
        let pad = if id == 3 { 0 } else { 16 };
        let mut frame = vec![b' '; 8 * 1024 * 1024 + pad];
        frame.push(b'\n');
        let req = serde_json::to_vec(
            &json!({"jsonrpc": "2.0", "id": id, "method": "tools/list", "params": {}}),
        )
        .unwrap();
        frame.extend_from_slice(&req);
        frame.push(b'\n');
        srv.stdin.write_all(&frame).unwrap();
        srv.stdin.flush().unwrap();

        let mut first = String::new();
        srv.reader.read_line(&mut first).unwrap();
        let err: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(err["error"]["code"], -32600, "{first}");

        let mut second = String::new();
        srv.reader.read_line(&mut second).unwrap();
        let resp: Value = serde_json::from_str(&second).unwrap();
        assert_eq!(resp["id"], id, "{second}");
        assert!(resp["result"]["tools"].is_array(), "{second}");
    }
}

#[test]
fn notification_ping_has_no_response() {
    let repo = fixture("notify-ping");
    let mut srv = spawn(&repo);
    send_raw(&mut srv, r#"{"jsonrpc":"2.0","method":"ping"}"#);
    let resp = rpc(&mut srv, 1, "ping", json!({}));
    assert_eq!(resp["id"], 1, "notification received a response: {resp}");
}

#[test]
fn invalid_request_envelopes_return_minus_32600() {
    let repo = fixture("invalid-envelope");
    let mut srv = spawn(&repo);
    send_raw(&mut srv, r#"{"jsonrpc":"2.0","id":2,"method":3}"#);
    let resp = read_response(&mut srv);
    assert_eq!(resp["id"], 2, "{resp}");
    assert_eq!(resp["error"]["code"], -32600, "{resp}");
    send_raw(&mut srv, r#"{"id":3,"method":"ping"}"#);
    let resp = read_response(&mut srv);
    assert_eq!(resp["id"], 3, "{resp}");
    assert_eq!(resp["error"]["code"], -32600, "{resp}");
    let resp = rpc(&mut srv, 4, "ping", json!({}));
    assert_eq!(
        resp["id"], 4,
        "server sequencing broke after invalid frames: {resp}"
    );
}

#[test]
fn client_response_envelope_is_ignored() {
    let repo = fixture("client-response");
    let mut srv = spawn(&repo);
    send_raw(&mut srv, r#"{"jsonrpc":"2.0","id":5,"result":{}}"#);
    let resp = rpc(&mut srv, 6, "ping", json!({}));
    assert_eq!(resp["id"], 6, "client response was answered: {resp}");
}

#[test]
fn mcp_dynamic_root_initialization_from_non_git_cwd() {
    let repo = fixture("dynamic-root");
    aw_cmd(&repo).arg("build").output().unwrap();

    let empty_dir = std::env::temp_dir().join(format!("aw-mcp-empty-{}", std::process::id()));
    let _ = fs::create_dir_all(&empty_dir);

    let mut child = Command::new(env!("CARGO_BIN_EXE_aw"))
        .current_dir(&empty_dir)
        .env("AW_GLOBAL_DIR", repo.join(".test_global"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    // 1. Initialize with rootUri
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "rootUri": format!("file://{}", repo.display()),
            "capabilities": {}
        }
    });
    writeln!(stdin, "{init_req}").unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let init_resp: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(init_resp["id"], 1);
    assert_eq!(init_resp["result"]["serverInfo"]["name"], "agent-weave");

    // 2. Call search tool on the dynamically bound repo
    let call_req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "search",
            "arguments": {"terms": "normalize"}
        }
    });
    writeln!(stdin, "{call_req}").unwrap();
    stdin.flush().unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    let call_resp: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(call_resp["id"], 2);
    assert_eq!(call_resp["result"]["isError"], false);

    let _ = child.kill();
    let _ = child.wait();
    let _ = fs::remove_dir_all(&empty_dir);
}
