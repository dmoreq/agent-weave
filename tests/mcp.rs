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
    let cli_dead = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .args(["dead"])
        .output()
        .unwrap();

    let mut srv = spawn(&repo);
    let init = rpc(
        &mut srv,
        1,
        "initialize",
        json!({"protocolVersion": "2025-06-18"}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "agent-weave");
    assert!(init["result"]["protocolVersion"].is_string());
    notification(&mut srv, "notifications/initialized");

    let list = rpc(&mut srv, 2, "tools/list", json!({}));
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "build",
            "callers",
            "callees",
            "path",
            "context",
            "implementers",
            "imports",
            "tests",
            "impact",
            "dead",
            "unresolved",
            "symbols",
            "complexity",
            "files",
            "resolve",
            "search",
            "clones",
            "stats",
            "hotspots",
            "report",
            "usage"
        ]
    );
    assert!(
        !names.contains(&"doctor"),
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
        result_text(&callers),
        String::from_utf8_lossy(&cli_callers.stdout).into_owned()
    );

    let symbols = rpc(
        &mut srv,
        4,
        "tools/call",
        json!({"name": "symbols", "arguments": {"file": "utils.py"}}),
    );
    assert_eq!(
        result_text(&symbols),
        String::from_utf8_lossy(&cli_symbols.stdout).into_owned()
    );

    let symbols_outline = rpc(
        &mut srv,
        8,
        "tools/call",
        json!({"name": "symbols", "arguments": {"file": "utils.py", "outline": true}}),
    );
    assert_eq!(
        result_text(&symbols_outline),
        String::from_utf8_lossy(&cli_symbols_outline.stdout).into_owned()
    );

    let search = rpc(
        &mut srv,
        9,
        "tools/call",
        json!({"name": "search", "arguments": {"terms": "normalize"}}),
    );
    assert_eq!(
        result_text(&search),
        String::from_utf8_lossy(&cli_search.stdout).into_owned()
    );

    // G10 parity extends to clones (Amendment 12d): the fixture's tail block
    // is duplicated across both files, so the pair line must be byte-equal.
    let cli_clones = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .args(["clones"])
        .output()
        .unwrap();
    assert_eq!(cli_clones.status.code(), Some(0));
    let clones = rpc(
        &mut srv,
        10,
        "tools/call",
        json!({"name": "clones", "arguments": {}}),
    );
    assert_eq!(
        result_text(&clones),
        String::from_utf8_lossy(&cli_clones.stdout).into_owned()
    );
    assert!(
        result_text(&clones).contains("app.py:5-9")
            && result_text(&clones).contains("utils.py:6-10"),
        "seeded clone pair line present: {}",
        result_text(&clones)
    );

    // min_lines above every span: both surfaces fail with the same stderr
    // text and isError semantics (Amendment 8 error contract).
    let cli_clones_hi = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .args(["clones", "--min-lines", "100"])
        .output()
        .unwrap();
    assert_eq!(cli_clones_hi.status.code(), Some(1));
    let clones_hi = rpc(
        &mut srv,
        11,
        "tools/call",
        json!({"name": "clones", "arguments": {"min_lines": 100}}),
    );
    assert_eq!(clones_hi["result"]["isError"], true);
    // R6 round-2: the empty path carries the mandatory caveat on stdout, so
    // the MCP text is caveat + the stderr reason line.
    let expected = format!(
        "aw: clone candidates — normalized-text equality, not proof of semantic duplication\n{}",
        String::from_utf8_lossy(&cli_clones_hi.stderr).trim_end()
    );
    assert_eq!(result_text(&clones_hi).trim_end(), expected);

    // R6e: parity covers every pointer/report tool that does not depend on
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
        ("tests", json!({"name": "run"}), vec!["tests", "run"]),
        (
            "complexity",
            json!({"min_cyc": 1}),
            vec!["complexity", "--min-cyc", "1"],
        ),
        (
            "hotspots",
            json!({"limit": 5}),
            vec!["hotspots", "--limit", "5"],
        ),
        ("report", json!({}), vec!["report"]),
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
            result_text(&via_mcp),
            String::from_utf8_lossy(&cli.stdout).into_owned(),
            "{tool} byte parity"
        );
    }
    // unresolved: empty-result parity (channel differs by contract: CLI stderr,
    // MCP result text; same class text).
    let cli_unres = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .args(["unresolved"])
        .output()
        .unwrap();
    let unres = rpc(
        &mut srv,
        14,
        "tools/call",
        json!({"name": "unresolved", "arguments": {}}),
    );
    if cli_unres.status.code() == Some(0) {
        assert_eq!(
            result_text(&unres),
            String::from_utf8_lossy(&cli_unres.stdout).into_owned()
        );
    } else {
        assert_eq!(unres["result"]["isError"], true);
        assert_eq!(
            result_text(&unres).trim_end(),
            String::from_utf8_lossy(&cli_unres.stderr).trim_end()
        );
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
        result_text(&imports_missing).trim_end(),
        String::from_utf8_lossy(&cli_imports_missing.stderr).trim_end()
    );

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
        result_text(&context_call),
        String::from_utf8_lossy(&cli_context.stdout).into_owned()
    );

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
        result_text(&path_call),
        String::from_utf8_lossy(&cli_path.stdout).into_owned()
    );
    assert!(
        result_text(&path_call).starts_with("aw: path — resolved static edges only;"),
        "path caveat first: {}",
        result_text(&path_call)
    );

    let dead = rpc(
        &mut srv,
        5,
        "tools/call",
        json!({"name": "dead", "arguments": {}}),
    );
    assert_eq!(
        result_text(&dead),
        String::from_utf8_lossy(&cli_dead.stdout).into_owned()
    );
    assert!(
        result_text(&dead).starts_with("aw: dead-code candidates —"),
        "caveat preserved: {}",
        result_text(&dead)
    );

    let stats = rpc(
        &mut srv,
        6,
        "tools/call",
        json!({"name": "stats", "arguments": {}}),
    );
    assert!(
        result_text(&stats).starts_with("aw stats: files=2"),
        "{}",
        result_text(&stats)
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

    // Every dispatched tool call journaled with the mcp surface: callers,
    // callers (depth 2), symbols (plain), symbols (outline), search, clones,
    // clones (min_lines failure), dead, stats, path, plus the R6e parity
    // additions (callees, impact, hotspots, report, unresolved, imports,
    // complexity; imports no-result). The unknown tool is rejected
    // before dispatch and journals nothing; CLI reference commands above add
    // their own surface=cli entries.
    let log = fs::read_to_string(repo.join(".aw/usage.log")).unwrap();
    let mcp_lines = log
        .lines()
        .filter(|l| l.contains("\"surface\":\"mcp\""))
        .count();
    assert_eq!(mcp_lines, 20, "{log}");

    // Amendment 10a: MCP pointer calls append payload lines with surface=mcp;
    // stats writes none. The callers payload pointers are byte-equal to the
    // CLI reference stdout's data lines (G10 parity extends to the payload).
    let payload = fs::read_to_string(repo.join(".aw/usage-results.jsonl")).unwrap();
    let mcp_payload = payload
        .lines()
        .filter(|l| l.contains("\"surface\":\"mcp\""))
        .count();
    assert_eq!(
        mcp_payload, 13,
        "callers+callers(depth2)+symbols(plain)+symbols(outline)+search+dead+path+callees+impact+context+imports+tests+complexity, not stats: {payload}"
    );
    let callers_line = payload
        .lines()
        .find(|l| l.contains("\"tool\":\"callers\"") && l.contains("\"surface\":\"mcp\""))
        .unwrap();
    assert!(
        callers_line.contains("\"name\":\"normalize\""),
        "{callers_line}"
    );
    let marker = "\"pointers\":[";
    let s = callers_line.find(marker).unwrap() + marker.len();
    let e = s + callers_line[s..].find("]}").unwrap();
    let expected: String = String::from_utf8_lossy(&cli_callers.stdout)
        .lines()
        .filter(|l| !l.starts_with("aw: "))
        .map(|p| format!("\"{p}\""))
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(&callers_line[s..e], &expected, "payload byte-parity");

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn g10_usage_verify_via_mcp() {
    // Amendment 10c on the MCP surface: usage with verify=true shares the CLI
    // code path and reports the same three-state outcome.
    let repo = fixture("mcp-verify");
    let build = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .arg("build")
        .output()
        .unwrap();
    assert_eq!(build.status.code(), Some(0));

    let mut srv = spawn(&repo);
    let callers = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({"name": "callers", "arguments": {"name": "normalize"}}),
    );
    assert_eq!(callers["result"]["isError"], false);
    let verify = rpc(
        &mut srv,
        2,
        "tools/call",
        json!({"name": "usage", "arguments": {"verify": true, "limit": 10}}),
    );
    assert_eq!(verify["result"]["isError"], false);
    let text = result_text(&verify);
    assert!(
        text.starts_with("aw usage --verify: 2 pointers sampled from 1 entries"),
        "{text}"
    );
    assert!(text.contains("changed=0 fail=0"), "{text}");
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
fn resolve_empty_names_and_non_strings_are_tool_errors() {
    let repo = fixture("mcp-resolve-empty");
    let build = Command::new(env!("CARGO_BIN_EXE_aw"))
        .arg("--root")
        .arg(&repo)
        .arg("build")
        .output()
        .unwrap();
    assert_eq!(build.status.code(), Some(0));
    let mut srv = spawn(&repo);

    let empty = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({"name": "resolve", "arguments": {"names": [""]}}),
    );
    assert_eq!(empty["result"]["isError"], true, "{empty}");
    assert!(result_text(&empty).contains("no names on stdin"), "{empty}");

    let mixed = rpc(
        &mut srv,
        2,
        "tools/call",
        json!({"name": "resolve", "arguments": {"names": ["normalize", 5]}}),
    );
    assert_eq!(mixed["result"]["isError"], true, "{mixed}");
    assert!(
        result_text(&mixed).contains("resolve names must be strings"),
        "{mixed}"
    );

    let ok = rpc(
        &mut srv,
        3,
        "tools/call",
        json!({"name": "resolve", "arguments": {"names": ["normalize"]}}),
    );
    assert_eq!(ok["result"]["isError"], false, "{ok}");
    assert!(result_text(&ok).contains("found normalize"), "{ok}");
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

/// Default MCP profile: only the eight core navigation tools. The full
/// inventory/analysis surface stays available behind `--all-tools`.
#[test]
fn default_mcp_exposes_core_navigation_tools() {
    let repo = fixture("core");
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
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["build", "callers", "callees", "path", "context", "imports", "impact", "symbols",]
    );

    let denied = rpc(
        &mut srv,
        3,
        "tools/call",
        json!({"name": "search", "arguments": {"terms": "normalize"}}),
    );
    assert_eq!(denied["error"]["code"], -32602);
    assert!(denied["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unknown tool 'search'"));
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
