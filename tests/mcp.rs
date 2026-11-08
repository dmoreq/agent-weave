//! MCP-vs-CLI parity conformance: the same fixture must produce byte-identical
//! result text through the MCP server and the CLI. The default registry is the
//! eight core navigation tools; `--all-tools` exposes all 21. Every tool call
//! must journal with surface "mcp".

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

use serde_json::{json, Value};

struct Server {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn(repo: &Path) -> Server {
    spawn_with(repo, true)
}

fn spawn_with(repo: &Path, all_tools: bool) -> Server {
    let mut child = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(repo)
        .env("AW_GLOBAL_DIR", repo.join(".test_global"))
        .arg("mcp")
        .args(all_tools.then_some("--all-tools"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let reader = BufReader::new(child.stdout.take().unwrap());
    Server {
        child,
        stdin,
        reader,
    }
}

fn rpc(server: &mut Server, id: u64, method: &str, params: Value) -> Value {
    let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    writeln!(server.stdin, "{req}").unwrap();
    server.stdin.flush().unwrap();
    let mut line = String::new();
    server.reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).expect("valid JSON-RPC response")
}

fn notification(server: &mut Server, method: &str) {
    let req = json!({"jsonrpc": "2.0", "method": method});
    writeln!(server.stdin, "{req}").unwrap();
    server.stdin.flush().unwrap();
}

/// Initialize the server, send the initialized notification, and return the
/// advertised tool names from `tools/list`.
fn init_and_list_tools(server: &mut Server) -> Vec<String> {
    let init = rpc(
        server,
        1,
        "initialize",
        json!({"protocolVersion": "2025-06-18"}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "agent-weave");
    assert!(init["result"]["protocolVersion"].is_string());
    notification(server, "notifications/initialized");
    let list = rpc(server, 2, "tools/list", json!({}));
    list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

fn send_raw(server: &mut Server, frame: &str) {
    writeln!(server.stdin, "{frame}").unwrap();
    server.stdin.flush().unwrap();
}

fn read_response(server: &mut Server) -> Value {
    let mut line = String::new();
    server.reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).expect("valid JSON-RPC response")
}

fn result_text(resp: &Value) -> String {
    resp["result"]["content"][0]["text"]
        .as_str()
        .expect("text content")
        .to_string()
}

fn strip_footer(text: &str) -> &str {
    if let Some(pos) = text.rfind("\n───\n⚡ aw:") {
        &text[..=pos]
    } else if let Some(pos) = text.rfind("⚡ aw:") {
        &text[..pos]
    } else {
        text
    }
}

fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aw-mcp-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let tail =
        "def tail_block(v):\n    a = v + 1\n    b = v + 2\n    c = v + 3\n    return a + b + c\n";
    fs::write(dir.join("utils.py"), format!("def normalize(text):\n    return text.strip()\n\ndef caller():\n    return normalize('a')\n{}", tail)).unwrap();
    fs::write(
        dir.join("app.py"),
        format!(
            "from utils import normalize\n\ndef run():\n    return normalize('x')\n{}\ndef test_run():\n    return run()\n",
            tail
        ),
    )
    .unwrap();
    dir
}

#[test]
fn g10_initialize_tools_list_and_parity() {
    let repo = fixture("parity");
    // Build the index before capturing CLI references.
    let build = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .arg("build")
        .output()
        .unwrap();
    assert_eq!(build.status.code(), Some(0));
    // CLI reference outputs, captured before the server opens the journal.
    let cli_callers = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .args(["callers", "normalize"])
        .output()
        .unwrap();
    assert_eq!(cli_callers.status.code(), Some(0));
    let cli_symbols = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .args(["symbols", "utils.py"])
        .output()
        .unwrap();
    let cli_symbols_outline = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .args(["symbols", "utils.py", "--outline"])
        .output()
        .unwrap();
    let cli_search = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .args(["search", "normalize"])
        .output()
        .unwrap();
    let mut srv = spawn(&repo);
    let names = init_and_list_tools(&mut srv);
    assert_eq!(
        names,
        [
            "build",
            "callers",
            "callees",
            "path",
            "context",
            "imports",
            "impact",
            "symbols",
            "reuse",
            "search",
            "usage",
            "mem_write",
            "mem_query",
            "mem_get",
            "mem_delete",
            "mem_gc",
            "mem_export"
        ]
        .map(str::to_string)
    );
    assert!(
        !names.iter().any(|name| name == "doctor"),
        "doctor is not exposed (Amendment 8)"
    );

    // Byte parity with the CLI stdout.
    let callers = rpc(
        &mut srv,
        3,
        "tools/call",
        json!({"name": "callers", "arguments": {"name": "normalize"}}),
    );
    assert_eq!(callers["result"]["isError"], false);
    assert_eq!(
        strip_footer(&result_text(&callers)),
        String::from_utf8_lossy(&cli_callers.stdout).into_owned()
    );
    assert!(result_text(&callers).contains("⚡ aw:"));

    let symbols = rpc(
        &mut srv,
        4,
        "tools/call",
        json!({"name": "symbols", "arguments": {"file": "utils.py"}}),
    );
    assert_eq!(
        strip_footer(&result_text(&symbols)),
        String::from_utf8_lossy(&cli_symbols.stdout).into_owned()
    );
    assert!(result_text(&symbols).contains("⚡ aw:"));

    let symbols_outline = rpc(
        &mut srv,
        8,
        "tools/call",
        json!({"name": "symbols", "arguments": {"file": "utils.py", "outline": true}}),
    );
    assert_eq!(
        strip_footer(&result_text(&symbols_outline)),
        String::from_utf8_lossy(&cli_symbols_outline.stdout).into_owned()
    );
    assert!(result_text(&symbols_outline).contains("⚡ aw:"));

    let search = rpc(
        &mut srv,
        9,
        "tools/call",
        json!({"name": "search", "arguments": {"terms": "normalize"}}),
    );
    assert_eq!(
        strip_footer(&result_text(&search)),
        String::from_utf8_lossy(&cli_search.stdout).into_owned()
    );
    assert!(result_text(&search).contains("⚡ aw:"));

    // R6e: parity covers every pointer tool that does not depend on
    // wall-clock timings or journal state.
    for (tool, args, cli_args) in [
        (
            "callers",
            json!({"name": "normalize", "depth": 2}),
            vec!["callers", "normalize", "--depth", "2"],
        ),
        (
            "callees",
            json!({"name": "caller"}),
            vec!["callees", "caller"],
        ),
        (
            "impact",
            json!({"file": "utils.py"}),
            vec!["impact", "utils.py"],
        ),
        (
            "imports",
            json!({"file": "app.py"}),
            vec!["imports", "app.py"],
        ),
        ("reuse", json!({}), vec!["reuse"]),
    ] {
        let cli = Command::new(env!("CARGO_BIN_EXE_aw"))
            .arg("--root")
            .arg(&repo)
            .args(&cli_args)
            .output()
            .unwrap();
        assert_eq!(cli.status.code(), Some(0), "{tool} CLI failed");
        let via_mcp = rpc(
            &mut srv,
            13,
            "tools/call",
            json!({"name": tool, "arguments": args}),
        );
        assert_eq!(
            strip_footer(&result_text(&via_mcp)),
            String::from_utf8_lossy(&cli.stdout).into_owned(),
            "{tool} byte parity"
        );
        assert!(result_text(&via_mcp).contains("⚡ aw:"));
    }

    // P8 no-result parity: imports on an unknown file (CLI stderr, MCP text).
    let cli_imports_missing = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .args(["imports", "nope.py"])
        .output()
        .unwrap();
    assert_eq!(cli_imports_missing.status.code(), Some(1));
    let imports_missing = rpc(
        &mut srv,
        15,
        "tools/call",
        json!({"name": "imports", "arguments": {"file": "nope.py"}}),
    );
    assert_eq!(imports_missing["result"]["isError"], true);
    assert_eq!(
        strip_footer(&result_text(&imports_missing)).trim_end(),
        String::from_utf8_lossy(&cli_imports_missing.stderr).trim_end()
    );
    assert!(result_text(&imports_missing).contains("⚡ aw:"));

    // G10 parity extends to context (Amendment 17).
    let cli_context = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .args(["context", "normalize"])
        .output()
        .unwrap();
    assert_eq!(cli_context.status.code(), Some(0));
    let context_call = rpc(
        &mut srv,
        15,
        "tools/call",
        json!({"name": "context", "arguments": {"name": "normalize"}}),
    );
    assert_eq!(
        strip_footer(&result_text(&context_call)),
        String::from_utf8_lossy(&cli_context.stdout).into_owned()
    );
    assert!(result_text(&context_call).contains("⚡ aw:"));

    // G10 parity extends to path (Amendment 13): hop pointers + chain line
    // byte-equal between the surfaces.
    let cli_path = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .args(["path", "caller", "normalize"])
        .output()
        .unwrap();
    assert_eq!(cli_path.status.code(), Some(0));
    let path_call = rpc(
        &mut srv,
        12,
        "tools/call",
        json!({"name": "path", "arguments": {"from": "caller", "to": "normalize"}}),
    );
    assert_eq!(
        strip_footer(&result_text(&path_call)),
        String::from_utf8_lossy(&cli_path.stdout).into_owned()
    );
    assert!(result_text(&path_call).contains("⚡ aw:"));
    assert!(
        result_text(&path_call).starts_with("aw: path — resolved static edges only;"),
        "path caveat first: {}",
        result_text(&path_call)
    );

    let unknown = rpc(
        &mut srv,
        7,
        "tools/call",
        json!({"name": "doctor", "arguments": {}}),
    );
    assert_eq!(
        unknown["error"]["code"], -32602,
        "unregistered tool rejected"
    );

    let log = fs::read_to_string(repo.join(".aw/usage.log")).unwrap();
    let mcp_lines = log
        .lines()
        .filter(|l| l.contains("\"surface\":\"mcp\""))
        .count();
    assert_eq!(mcp_lines, 12, "{log}");

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn g10_ambiguous_carries_candidates_and_iserror() {
    let repo = fixture("ambiguous");
    fs::write(
        repo.join("a.py"),
        "class A:\n    def process(self):\n        return 1\n",
    )
    .unwrap();
    fs::write(
        repo.join("b.py"),
        "class B:\n    def process(self):\n        return 2\n",
    )
    .unwrap();
    Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .arg("build")
        .output()
        .unwrap();

    let mut srv = spawn(&repo);
    let resp = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({"name": "callers", "arguments": {"name": "process"}}),
    );
    assert_eq!(
        resp["result"]["isError"], true,
        "ambiguous name is an error result"
    );
    let text = result_text(&resp);
    assert!(text.contains("a.py:2 a.A.process"), "{text}");
    assert!(text.contains("b.py:2 b.B.process"), "{text}");
    assert!(text.contains("ambiguous name 'process'"), "{text}");
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn reuse_arguments_via_mcp() {
    let repo = fixture("mcp-reuse");
    let build = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .arg("build")
        .output()
        .unwrap();
    assert_eq!(build.status.code(), Some(0));
    let mut srv = spawn(&repo);

    let bad_paths = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({"name": "reuse", "arguments": {"paths": "not_an_array"}}),
    );
    assert_eq!(bad_paths["result"]["isError"], true, "{bad_paths}");

    let ok = rpc(
        &mut srv,
        2,
        "tools/call",
        json!({"name": "reuse", "arguments": {"paths": ["utils.py"]}}),
    );
    assert_eq!(ok["result"]["isError"], false, "{ok}");
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn g10_build_via_mcp_journals_and_returns_stats_line() {
    let repo = fixture("build");
    let mut srv = spawn(&repo);
    let resp = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({"name": "build", "arguments": {}}),
    );
    assert_eq!(resp["result"]["isError"], false);
    let text = result_text(&resp);
    assert!(text.starts_with("aw: built index: files=2"), "{text}");
    assert!(text.contains("parse_errors=0"), "{text}");
    let log = fs::read_to_string(repo.join(".aw/usage.log")).unwrap();
    assert!(
        log.contains("\"tool\":\"build\"") && log.contains("\"surface\":\"mcp\""),
        "{log}"
    );
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn malformed_json_frame_returns_parse_error() {
    let repo = fixture("mcp-badjson");
    let build = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .arg("build")
        .output()
        .unwrap();
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
    let build = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .arg("build")
        .output()
        .unwrap();
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

/// Default MCP profile: the eight core navigation tools plus the memory
/// write/query loop (OQ1). The full inventory/analysis surface stays
/// available behind `--all-tools`.
#[test]
fn default_mcp_exposes_core_navigation_tools() {
    let repo = fixture("core");
    let mut srv = spawn_with(&repo, false);
    let names = init_and_list_tools(&mut srv);
    assert_eq!(
        names,
        [
            "build",
            "callers",
            "callees",
            "path",
            "context",
            "imports",
            "impact",
            "symbols",
            "reuse",
            "search",
            "usage",
            "mem_write",
            "mem_query",
        ]
        .map(str::to_string)
    );

    let denied = rpc(
        &mut srv,
        3,
        "tools/call",
        json!({"name": "mem_get", "arguments": {"key": "k"}}),
    );
    assert_eq!(denied["error"]["code"], -32602);
    assert!(denied["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unknown tool 'mem_get'"));
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

#[cfg(unix)]
#[test]
fn mcp_build_rejects_symlinked_aw_directory() {
    use std::os::unix::fs::symlink;
    let repo = fixture("mcp-symlink-aw");
    let ext = std::env::temp_dir().join(format!("aw-ext-mcp-aw-{}", std::process::id()));
    let _ = fs::remove_dir_all(&ext);
    fs::create_dir_all(&ext).unwrap();
    let sentinel = ext.join("sentinel.txt");
    fs::write(&sentinel, "keep").unwrap();
    let _ = fs::remove_dir_all(repo.join(".aw"));
    symlink(&ext, repo.join(".aw")).unwrap();

    let mut srv = spawn(&repo);
    let build = rpc(
        &mut srv,
        2,
        "tools/call",
        json!({"name": "build", "arguments": {}}),
    );
    assert_eq!(build["result"]["isError"], true, "{build}");
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "keep");
    assert!(!ext.join("index.db").exists());
    let _ = fs::remove_dir_all(&ext);
}

// ---------------------------------------------------------------------------
// Project memory over MCP (companion design, Phase 1): OQ1 puts mem_write +
// mem_query in the default `aw mcp` tool set (ToolSpec::core); the other four
// stay behind --all-tools. Every call dispatches through service::execute, so
// the result text is the CLI's stdout byte for byte.
// ---------------------------------------------------------------------------

fn mem_fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aw-mcp-mem-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cli_mem(repo: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(repo)
        .args(["mem"])
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn mem_core_tools_default_surface_and_byte_parity() {
    let repo = mem_fixture("core");
    // CLI reference output first (the server would journal its own calls).
    let (code, cli_write, cli_stderr) = cli_mem(
        &repo,
        &[
            "write",
            "parity-note",
            "--body",
            "the resolver keeps confidence tiers over resolved edges",
            "--kind",
            "decision",
            "--tags",
            "resolver confidence",
            "--refs",
            "src/query/callgraph.rs",
        ],
    );
    assert_eq!(code, 0, "{cli_write}{cli_stderr}");

    // spawn() defaults to --all-tools; the default surface needs spawn_with(false).
    let mut srv = spawn_with(&repo, false);
    let init = rpc(
        &mut srv,
        1,
        "initialize",
        json!({"protocolVersion": "2025-06-18"}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "agent-weave");
    notification(&mut srv, "notifications/initialized");
    let list = rpc(&mut srv, 2, "tools/list", json!({}));
    let names: Vec<String> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"mem_write".to_string()), "{names:?}");
    assert!(names.contains(&"mem_query".to_string()), "{names:?}");
    for tool in ["mem_get", "mem_delete", "mem_gc", "mem_export"] {
        assert!(
            !names.contains(&tool.to_string()),
            "{tool} stays behind --all-tools: {names:?}"
        );
    }

    // A second note via MCP (duplicate content under a new key shape).
    let call = rpc(
        &mut srv,
        3,
        "tools/call",
        json!({
            "name": "mem_write",
            "arguments": {
                "key": "parity-note-2",
                "body": "memory notes are candidates, never proof",
                "kind": "fact"
            }
        }),
    );
    assert_eq!(call["result"]["isError"], false, "{}", result_text(&call));

    // mem_query over MCP is byte-identical to the CLI stdout.
    let (code, cli_query, _stderr) = cli_mem(&repo, &["query", "confidence", "tiers"]);
    assert_eq!(code, 0);
    let query_call = rpc(
        &mut srv,
        4,
        "tools/call",
        json!({"name": "mem_query", "arguments": {"terms": "confidence tiers"}}),
    );
    assert_eq!(query_call["result"]["isError"], false);
    assert_eq!(result_text(&query_call), cli_query, "mem_query byte parity");

    // Journal: MCP surface tag lands on mem tool rows.
    drop(srv);
    let log = fs::read_to_string(repo.join(".aw/usage.log")).unwrap();
    assert!(
        log.lines()
            .any(|l| l.contains("\"tool\":\"mem_write\"") && l.contains("\"surface\":\"mcp\"")),
        "{log}"
    );
}

#[test]
fn mem_all_tools_exposes_full_family_with_parity() {
    let repo = mem_fixture("all");
    let (code, _stdout, stderr) = cli_mem(
        &repo,
        &[
            "write",
            "export-me",
            "--body",
            "export parity body",
            "--kind",
            "fact",
        ],
    );
    assert_eq!(code, 0, "{stderr}");

    let mut srv = spawn_with(&repo, true);
    notification(&mut srv, "notifications/initialized");
    let list = rpc(&mut srv, 1, "tools/list", json!({}));
    let names: Vec<String> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    for tool in [
        "mem_write",
        "mem_query",
        "mem_get",
        "mem_delete",
        "mem_gc",
        "mem_export",
    ] {
        assert!(
            names.contains(&tool.to_string()),
            "{tool} missing: {names:?}"
        );
    }

    // mem_get parity.
    let (code, cli_get, _stderr) = cli_mem(&repo, &["get", "export-me"]);
    assert_eq!(code, 0);
    let get = rpc(
        &mut srv,
        2,
        "tools/call",
        json!({"name": "mem_get", "arguments": {"key": "export-me"}}),
    );
    assert_eq!(result_text(&get), cli_get, "mem_get byte parity");

    // mem_gc parity (Phase 1 report-only).
    let (code, cli_gc, _stderr) = cli_mem(&repo, &["gc"]);
    assert_eq!(code, 0);
    let gc = rpc(
        &mut srv,
        3,
        "tools/call",
        json!({"name": "mem_gc", "arguments": {}}),
    );
    assert_eq!(result_text(&gc), cli_gc, "mem_gc byte parity");

    // mem_export parity: pure JSONL text, caveat absent from the text.
    let (code, cli_export, _stderr) = cli_mem(&repo, &["export"]);
    assert_eq!(code, 0);
    let export = rpc(
        &mut srv,
        4,
        "tools/call",
        json!({"name": "mem_export", "arguments": {}}),
    );
    assert_eq!(result_text(&export), cli_export, "mem_export byte parity");
    assert!(
        !result_text(&export).contains("aw: memory"),
        "{:?}",
        result_text(&export)
    );

    // Phase 2 lifecycle over MCP: session scope + ttl + cross-key supersedes.
    let session = rpc(
        &mut srv,
        6,
        "tools/call",
        json!({"name": "mem_write", "arguments": {
            "key": "mcp-session-note", "body": "transient mcp scratch", "kind": "fact",
            "scope": "session", "ttl": 3600
        }}),
    );
    assert_eq!(
        session["result"]["isError"],
        false,
        "{}",
        result_text(&session)
    );
    let supersedes = rpc(
        &mut srv,
        7,
        "tools/call",
        json!({"name": "mem_write", "arguments": {
            "key": "mcp-replacement", "body": "replaces the export note", "kind": "fact",
            "supersedes": "export-me"
        }}),
    );
    assert_eq!(
        supersedes["result"]["isError"],
        false,
        "{}",
        result_text(&supersedes)
    );
    let (code, cli_all, _stderr) = cli_mem(&repo, &["query", "--all"]);
    assert_eq!(code, 0);
    assert!(
        cli_all.contains("mcp-session-note") && !cli_all.contains("export-me"),
        "{cli_all}"
    );
    let all = rpc(
        &mut srv,
        8,
        "tools/call",
        json!({"name": "mem_query", "arguments": {"all": true}}),
    );
    assert_eq!(result_text(&all), cli_all, "mem_query --all byte parity");
    let with_superseded = rpc(
        &mut srv,
        9,
        "tools/call",
        json!({"name": "mem_query", "arguments": {"all": true, "include_superseded": true}}),
    );
    let (code, cli_sup, _stderr) = cli_mem(&repo, &["query", "--all", "--include-superseded"]);
    assert_eq!(code, 0);
    assert!(
        result_text(&with_superseded).contains("[superseded]"),
        "{}",
        result_text(&with_superseded)
    );
    assert_eq!(
        result_text(&with_superseded),
        cli_sup,
        "include-superseded byte parity"
    );

    // mem_gc parity: expire the session note via the store, then compare.
    {
        let conn = rusqlite::Connection::open(repo.join(".aw/memory.db")).unwrap();
        conn.execute(
            "UPDATE memories SET expires_at = 1 WHERE key = 'mcp-session-note'",
            [],
        )
        .unwrap();
    }
    let (code, cli_gc_after, _stderr) = cli_mem(&repo, &["gc"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(cli_gc_after.contains("expired=1 removed"), "{cli_gc_after}");

    // mem_delete parity: MCP delete text equals the CLI delete text for the
    // same input (recreated between the two calls — delete is destructive).
    let write = rpc(
        &mut srv,
        10,
        "tools/call",
        json!({"name": "mem_write", "arguments": {"key": "doomed", "body": "delete parity body", "kind": "fact"}}),
    );
    assert_eq!(write["result"]["isError"], false);
    let delete = rpc(
        &mut srv,
        11,
        "tools/call",
        json!({"name": "mem_delete", "arguments": {"key": "doomed"}}),
    );
    assert_eq!(delete["result"]["isError"], false);
    // Recreate via CLI so the CLI delete sees the same input shape.
    let (code, _stdout, stderr) = cli_mem(
        &repo,
        &[
            "write",
            "doomed",
            "--body",
            "delete parity body",
            "--kind",
            "fact",
        ],
    );
    assert_eq!(code, 0, "{stderr}");
    let (code, cli_delete, _stderr) = cli_mem(&repo, &["delete", "doomed"]);
    assert_eq!(code, 0);
    assert_eq!(result_text(&delete), cli_delete, "mem_delete byte parity");

    // Error path: unknown key reaches the caller with the CLI's stderr text.
    let (code, _stdout, cli_err) = cli_mem(&repo, &["get", "missing"]);
    assert_eq!(code, 1);
    let missing = rpc(
        &mut srv,
        5,
        "tools/call",
        json!({"name": "mem_get", "arguments": {"key": "missing"}}),
    );
    assert_eq!(missing["result"]["isError"], true);
    assert!(
        result_text(&missing).contains("unknown memory key 'missing'"),
        "{}",
        result_text(&missing)
    );
    assert!(
        result_text(&missing).contains(cli_err.trim()),
        "{}",
        result_text(&missing)
    );
}

#[test]
fn mcp_response_includes_kpi_footer() {
    let repo = fixture("kpi-footer");
    let build = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .arg("build")
        .output()
        .unwrap();
    assert_eq!(build.status.code(), Some(0));

    let mut srv = spawn(&repo);
    init_and_list_tools(&mut srv);

    // 1. context
    let ctx = rpc(
        &mut srv,
        10,
        "tools/call",
        json!({"name": "context", "arguments": {"name": "normalize"}}),
    );
    assert_eq!(ctx["result"]["isError"], false);
    let text = result_text(&ctx);
    assert!(text.contains("\n───\n⚡ aw:"), "context footer: {text}");
    assert!(
        text.contains("tokens saved vs file reads"),
        "context tokens: {text}"
    );

    // 2. search
    let search = rpc(
        &mut srv,
        11,
        "tools/call",
        json!({"name": "search", "arguments": {"terms": "normalize"}}),
    );
    assert_eq!(search["result"]["isError"], false);
    let text = result_text(&search);
    assert!(text.contains("\n───\n⚡ aw:"), "search footer: {text}");
    assert!(
        text.contains("tokens saved vs file reads"),
        "search tokens: {text}"
    );

    // 3. reuse
    let reuse = rpc(
        &mut srv,
        12,
        "tools/call",
        json!({"name": "reuse", "arguments": {}}),
    );
    assert_eq!(reuse["result"]["isError"], false);
    let text = result_text(&reuse);
    assert!(text.contains("\n───\n⚡ aw:"), "reuse footer: {text}");
    assert!(
        text.contains("tokens saved vs file reads"),
        "reuse tokens: {text}"
    );

    // 4. callees on a leaf function (0 items -> clean footer without tokens saved)
    let leaf = rpc(
        &mut srv,
        13,
        "tools/call",
        json!({"name": "callees", "arguments": {"name": "normalize"}}),
    );
    assert_eq!(leaf["result"]["isError"], true);
    let leaf_text = result_text(&leaf);
    assert!(
        leaf_text.contains("\n───\n⚡ aw:"),
        "leaf footer: {leaf_text}"
    );
    assert!(
        leaf_text.contains("| 0 items"),
        "leaf zero items: {leaf_text}"
    );
    assert!(
        !leaf_text.contains("tokens saved"),
        "leaf no savings: {leaf_text}"
    );

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn mcp_usage_tool_returns_telemetry() {
    let repo = fixture("mcp-usage");
    let build = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .arg("build")
        .output()
        .unwrap();
    assert_eq!(build.status.code(), Some(0));

    let mut srv = spawn(&repo);
    init_and_list_tools(&mut srv);

    // Call a couple of tools first to populate the journal
    let _ = rpc(
        &mut srv,
        10,
        "tools/call",
        json!({"name": "callers", "arguments": {"name": "normalize"}}),
    );
    let _ = rpc(
        &mut srv,
        11,
        "tools/call",
        json!({"name": "search", "arguments": {"terms": "normalize"}}),
    );

    // Now call usage tool via MCP
    let usage = rpc(
        &mut srv,
        12,
        "tools/call",
        json!({"name": "usage", "arguments": {}}),
    );
    assert_eq!(usage["result"]["isError"], false, "{usage}");
    let text = result_text(&usage);
    assert!(text.contains("aw usage:"), "header: {text}");
    assert!(text.contains("Speed: p50 ="), "speed: {text}");
    assert!(text.contains("Reliability:"), "reliability: {text}");
    assert!(text.contains("Estimated Token Savings:"), "savings: {text}");
    assert!(
        text.contains("Anti-Reinvention:"),
        "anti-reinvention: {text}"
    );
    assert!(text.contains("callers:"), "callers: {text}");
    // usage is an on-demand telemetry card; it must not append a redundant footer
    assert!(
        !text.contains("⚡ aw:"),
        "usage has no redundant footer: {text}"
    );

    let _ = fs::remove_dir_all(&repo);
}

fn spawn_with_global(repo: &Path, global_dir: &Path) -> Server {
    let mut child = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(repo)
        .env("AW_GLOBAL_DIR", global_dir)
        .arg("mcp")
        .arg("--all-tools")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let reader = BufReader::new(child.stdout.take().unwrap());
    Server {
        child,
        stdin,
        reader,
    }
}

#[test]
fn global_memory_mcp_protocol_parity() {
    let repo = mem_fixture("mcp-global-memory");
    let global_dir = mem_fixture("mcp-global-aw");
    let mut srv = spawn_with_global(&repo, &global_dir);
    init_and_list_tools(&mut srv);

    // 1. mem_write with global: true
    let write_res = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({
            "name": "mem_write",
            "arguments": {
                "key": "user_pref",
                "body": "Language: Vietnamese. Style: Concise.",
                "kind": "decision",
                "global": true
            }
        }),
    );
    assert_eq!(write_res["result"]["isError"], false, "{write_res}");
    let text = result_text(&write_res);
    assert!(text.contains("ok: 'user_pref' rev 1 written"), "{text}");

    // Global file exists, local file does not
    assert!(global_dir.join("memory.db").exists());
    assert!(!repo.join(".aw/memory.db").exists());

    // 2. mem_query without global flag (layered query) returns the global note tagged [global]
    let query_res = rpc(
        &mut srv,
        2,
        "tools/call",
        json!({
            "name": "mem_query",
            "arguments": {
                "terms": "Vietnamese"
            }
        }),
    );
    assert_eq!(query_res["result"]["isError"], false, "{query_res}");
    let text = result_text(&query_res);
    assert!(text.contains("user_pref r1 decision durable "));
    assert!(text.contains(" [global]"), "{text}");
    assert!(text.contains("Language: Vietnamese"), "{text}");

    // 3. mem_get with global: true
    let get_res = rpc(
        &mut srv,
        3,
        "tools/call",
        json!({
            "name": "mem_get",
            "arguments": {
                "key": "user_pref",
                "global": true
            }
        }),
    );
    assert_eq!(get_res["result"]["isError"], false, "{get_res}");
    let text = result_text(&get_res);
    assert!(text.contains("user_pref r1 decision durable "));
    assert!(text.contains(" [global]"), "{text}");
    assert!(text.contains("revs: 1"), "{text}");

    // 4. mem_export with global: true
    let export_res = rpc(
        &mut srv,
        4,
        "tools/call",
        json!({
            "name": "mem_export",
            "arguments": {
                "global": true
            }
        }),
    );
    assert_eq!(export_res["result"]["isError"], false, "{export_res}");
    let text = result_text(&export_res);
    assert!(text.contains("\"key\":\"user_pref\""), "{text}");

    // 5. mem_gc with global: true
    let gc_res = rpc(
        &mut srv,
        5,
        "tools/call",
        json!({
            "name": "mem_gc",
            "arguments": {
                "global": true
            }
        }),
    );
    assert_eq!(gc_res["result"]["isError"], false, "{gc_res}");
    let text = result_text(&gc_res);
    assert!(
        text.contains("expired=0 removed, superseded=0 removed, durable=1 rows"),
        "{text}"
    );

    // 6. mem_delete with global: true
    let del_res = rpc(
        &mut srv,
        6,
        "tools/call",
        json!({
            "name": "mem_delete",
            "arguments": {
                "key": "user_pref",
                "global": true
            }
        }),
    );
    assert_eq!(del_res["result"]["isError"], false, "{del_res}");
    let text = result_text(&del_res);
    assert!(text.contains("deleted 'user_pref' revs=1"), "{text}");

    let _ = fs::remove_dir_all(&repo);
    let _ = fs::remove_dir_all(&global_dir);
}

#[test]
fn global_usage_mcp_returns_cross_repo_summary() {
    let repo = fixture("mcp-global-usage");
    let global_dir = mem_fixture("mcp-global-usage-aw");

    let build = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .env("AW_GLOBAL_DIR", &global_dir)
        .arg("build")
        .output()
        .unwrap();
    assert_eq!(build.status.code(), Some(0));

    let mut srv = spawn_with_global(&repo, &global_dir);
    init_and_list_tools(&mut srv);

    // Call search tool to generate telemetry
    let _ = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({"name": "search", "arguments": {"terms": "normalize"}}),
    );

    // Call usage tool with global: true via MCP
    let usage = rpc(
        &mut srv,
        2,
        "tools/call",
        json!({"name": "usage", "arguments": {"global": true}}),
    );
    assert_eq!(usage["result"]["isError"], false, "{usage}");
    let text = result_text(&usage);
    assert!(text.contains("aw global usage:"), "header: {text}");
    assert!(text.contains("Speed: p50 ="), "speed: {text}");
    assert!(
        text.contains("Active Repositories:"),
        "active repos: {text}"
    );

    let _ = fs::remove_dir_all(&repo);
    let _ = fs::remove_dir_all(&global_dir);
}
