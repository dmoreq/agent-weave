//! G10 — MCP-vs-CLI parity conformance (Amendment 8): the same fixture must
//! produce byte-identical result text through the MCP server and the CLI, the
//! tool registry must expose exactly the 11 Amendment-8 tools, and every tool
//! call must journal with surface "mcp".

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
    let mut child = Command::new(env!("CARGO_BIN_EXE_kg"))
        .arg("--root")
        .arg(repo)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let reader = BufReader::new(child.stdout.take().unwrap());
    Server { child, stdin, reader }
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

fn result_text(resp: &Value) -> String {
    resp["result"]["content"][0]["text"]
        .as_str()
        .expect("text content")
        .to_string()
}

fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kg-mcp-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let tail = "def tail_block(v):\n    a = v + 1\n    b = v + 2\n    c = v + 3\n    return a + b + c\n";
    fs::write(dir.join("utils.py"), format!("def normalize(text):\n    return text.strip()\n\ndef caller():\n    return normalize('a')\n{}", tail)).unwrap();
    fs::write(dir.join("app.py"), format!("from utils import normalize\n\ndef run():\n    return normalize('x')\n{}", tail)).unwrap();
    dir
}

#[test]
fn g10_initialize_tools_list_and_parity() {
    let repo = fixture("parity");
    // Build the index before capturing CLI references.
    let build = Command::new(env!("CARGO_BIN_EXE_kg"))
        .arg("--root").arg(&repo).arg("build")
        .output().unwrap();
    assert_eq!(build.status.code(), Some(0));
    // CLI reference outputs, captured before the server opens the journal.
    let cli_callers = Command::new(env!("CARGO_BIN_EXE_kg"))
        .arg("--root").arg(&repo).args(["callers", "normalize"])
        .output().unwrap();
    assert_eq!(cli_callers.status.code(), Some(0));
    let cli_symbols = Command::new(env!("CARGO_BIN_EXE_kg"))
        .arg("--root").arg(&repo).args(["symbols", "utils.py"])
        .output().unwrap();
    let cli_symbols_outline = Command::new(env!("CARGO_BIN_EXE_kg"))
        .arg("--root").arg(&repo).args(["symbols", "utils.py", "--outline"])
        .output().unwrap();
    let cli_search = Command::new(env!("CARGO_BIN_EXE_kg"))
        .arg("--root").arg(&repo).args(["search", "normalize"])
        .output().unwrap();
    let cli_dead = Command::new(env!("CARGO_BIN_EXE_kg"))
        .arg("--root").arg(&repo).args(["dead"])
        .output().unwrap();

    let mut srv = spawn(&repo);
    let init = rpc(&mut srv, 1, "initialize", json!({"protocolVersion": "2025-06-18"}));
    assert_eq!(init["result"]["serverInfo"]["name"], "kg");
    assert!(init["result"]["protocolVersion"].is_string());
    notification(&mut srv, "notifications/initialized");

    let list = rpc(&mut srv, 2, "tools/list", json!({}));
    let names: Vec<&str> = list["result"]["tools"]
        .as_array().unwrap()
        .iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        vec!["build", "callers", "callees", "path", "impact", "dead", "unresolved", "symbols", "search", "clones", "stats", "hotspots", "report", "usage"]
    );
    assert!(!names.contains(&"doctor"), "doctor is not exposed (Amendment 8)");

    // Byte parity with the CLI stdout.
    let callers = rpc(&mut srv, 3, "tools/call", json!({"name": "callers", "arguments": {"name": "normalize"}}));
    assert_eq!(callers["result"]["isError"], false);
    assert_eq!(result_text(&callers), String::from_utf8_lossy(&cli_callers.stdout).into_owned());

    let symbols = rpc(&mut srv, 4, "tools/call", json!({"name": "symbols", "arguments": {"file": "utils.py"}}));
    assert_eq!(result_text(&symbols), String::from_utf8_lossy(&cli_symbols.stdout).into_owned());

    let symbols_outline = rpc(&mut srv, 8, "tools/call", json!({"name": "symbols", "arguments": {"file": "utils.py", "outline": true}}));
    assert_eq!(result_text(&symbols_outline), String::from_utf8_lossy(&cli_symbols_outline.stdout).into_owned());

    let search = rpc(&mut srv, 9, "tools/call", json!({"name": "search", "arguments": {"terms": "normalize"}}));
    assert_eq!(result_text(&search), String::from_utf8_lossy(&cli_search.stdout).into_owned());

    // G10 parity extends to clones (Amendment 12d): the fixture's tail block
    // is duplicated across both files, so the pair line must be byte-equal.
    let cli_clones = Command::new(env!("CARGO_BIN_EXE_kg"))
        .arg("--root").arg(&repo).args(["clones"])
        .output().unwrap();
    assert_eq!(cli_clones.status.code(), Some(0));
    let clones = rpc(&mut srv, 10, "tools/call", json!({"name": "clones", "arguments": {}}));
    assert_eq!(result_text(&clones), String::from_utf8_lossy(&cli_clones.stdout).into_owned());
    assert!(
        result_text(&clones).contains("app.py:5-9 utils.py:6-10"),
        "seeded clone pair line present: {}",
        result_text(&clones)
    );

    // min_lines above every span: both surfaces fail with the same stderr
    // text and isError semantics (Amendment 8 error contract).
    let cli_clones_hi = Command::new(env!("CARGO_BIN_EXE_kg"))
        .arg("--root").arg(&repo).args(["clones", "--min-lines", "100"])
        .output().unwrap();
    assert_eq!(cli_clones_hi.status.code(), Some(1));
    let clones_hi = rpc(&mut srv, 11, "tools/call", json!({"name": "clones", "arguments": {"min_lines": 100}}));
    assert_eq!(clones_hi["result"]["isError"], true);
    assert_eq!(
        result_text(&clones_hi).trim_end(),
        String::from_utf8_lossy(&cli_clones_hi.stderr).trim_end()
    );

    // G10 parity extends to path (Amendment 13): hop pointers + chain line
    // byte-equal between the surfaces.
    let cli_path = Command::new(env!("CARGO_BIN_EXE_kg"))
        .arg("--root").arg(&repo).args(["path", "caller", "normalize"])
        .output().unwrap();
    assert_eq!(cli_path.status.code(), Some(0));
    let path_call = rpc(&mut srv, 12, "tools/call", json!({"name": "path", "arguments": {"from": "caller", "to": "normalize"}}));
    assert_eq!(result_text(&path_call), String::from_utf8_lossy(&cli_path.stdout).into_owned());
    assert!(result_text(&path_call).starts_with("kg: path — resolved static edges only;"), "path caveat first: {}", result_text(&path_call));

    let dead = rpc(&mut srv, 5, "tools/call", json!({"name": "dead", "arguments": {}}));
    assert_eq!(result_text(&dead), String::from_utf8_lossy(&cli_dead.stdout).into_owned());
    assert!(result_text(&dead).starts_with("kg: dead-code candidates —"), "caveat preserved: {}", result_text(&dead));

    let stats = rpc(&mut srv, 6, "tools/call", json!({"name": "stats", "arguments": {}}));
    assert!(result_text(&stats).starts_with("kg stats: files=2"), "{}", result_text(&stats));

    let unknown = rpc(&mut srv, 7, "tools/call", json!({"name": "doctor", "arguments": {}}));
    assert_eq!(unknown["error"]["code"], -32602, "unregistered tool rejected");

    // Every dispatched tool call journaled with the mcp surface: callers,
    // symbols (plain), symbols (outline), search, clones, clones (min_lines
    // failure), dead, stats, path. The unknown tool is rejected before
    // dispatch and journals nothing; the CLI reference commands above added
    // their own surface=cli entries.
    let log = fs::read_to_string(repo.join(".kg/usage.log")).unwrap();
    let mcp_lines = log.lines().filter(|l| l.contains("\"surface\":\"mcp\"")).count();
    assert_eq!(mcp_lines, 9, "{log}");

    // Amendment 10a: MCP pointer calls append payload lines with surface=mcp;
    // stats writes none. The callers payload pointers are byte-equal to the
    // CLI reference stdout's data lines (G10 parity extends to the payload).
    let payload = fs::read_to_string(repo.join(".kg/usage-results.jsonl")).unwrap();
    let mcp_payload = payload
        .lines()
        .filter(|l| l.contains("\"surface\":\"mcp\""))
        .count();
    assert_eq!(
        mcp_payload,
        6,
        "callers+symbols(plain)+symbols(outline)+search+dead+path, not stats: {payload}"
    );
    let callers_line = payload
        .lines()
        .find(|l| l.contains("\"tool\":\"callers\"") && l.contains("\"surface\":\"mcp\""))
        .unwrap();
    assert!(callers_line.contains("\"name\":\"normalize\""), "{callers_line}");
    let marker = "\"pointers\":[";
    let s = callers_line.find(marker).unwrap() + marker.len();
    let e = s + callers_line[s..].find("]}").unwrap();
    let expected: String = String::from_utf8_lossy(&cli_callers.stdout)
        .lines()
        .filter(|l| !l.starts_with("kg: "))
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
    let build = Command::new(env!("CARGO_BIN_EXE_kg"))
        .arg("--root").arg(&repo).arg("build")
        .output().unwrap();
    assert_eq!(build.status.code(), Some(0));

    let mut srv = spawn(&repo);
    let callers = rpc(&mut srv, 1, "tools/call", json!({"name": "callers", "arguments": {"name": "normalize"}}));
    assert_eq!(callers["result"]["isError"], false);
    let verify = rpc(&mut srv, 2, "tools/call", json!({"name": "usage", "arguments": {"verify": true, "limit": 10}}));
    assert_eq!(verify["result"]["isError"], false);
    let text = result_text(&verify);
    assert!(text.starts_with("kg usage --verify: 2 pointers sampled from 1 entries"), "{text}");
    assert!(text.contains("changed=0 fail=0"), "{text}");
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn g10_ambiguous_carries_candidates_and_iserror() {
    let repo = fixture("ambiguous");
    fs::write(repo.join("a.py"), "class A:\n    def process(self):\n        return 1\n").unwrap();
    fs::write(repo.join("b.py"), "class B:\n    def process(self):\n        return 2\n").unwrap();
    Command::new(env!("CARGO_BIN_EXE_kg")).arg("--root").arg(&repo).arg("build").output().unwrap();

    let mut srv = spawn(&repo);
    let resp = rpc(&mut srv, 1, "tools/call", json!({"name": "callers", "arguments": {"name": "process"}}));
    assert_eq!(resp["result"]["isError"], true, "ambiguous name is an error result");
    let text = result_text(&resp);
    assert!(text.contains("a.py:2 a.A.process"), "{text}");
    assert!(text.contains("b.py:2 b.B.process"), "{text}");
    assert!(text.contains("ambiguous name 'process'"), "{text}");
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn g10_build_via_mcp_journals_and_returns_stats_line() {
    let repo = fixture("build");
    let mut srv = spawn(&repo);
    let resp = rpc(&mut srv, 1, "tools/call", json!({"name": "build", "arguments": {}}));
    assert_eq!(resp["result"]["isError"], false);
    let text = result_text(&resp);
    assert!(text.starts_with("kg: built index: files=2"), "{text}");
    assert!(text.contains("parse_errors=0"), "{text}");
    let log = fs::read_to_string(repo.join(".kg/usage.log")).unwrap();
    assert!(log.contains("\"tool\":\"build\"") && log.contains("\"surface\":\"mcp\""), "{log}");
    let _ = fs::remove_dir_all(&repo);
}
