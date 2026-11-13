//! Shared MCP test helpers: server process lifecycle, JSON-RPC framing I/O,
//! and temp-repo fixtures.

use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;

use serde_json::{json, Value};

pub(crate) struct Server {
    pub(crate) child: Child,
    pub(crate) stdin: ChildStdin,
    pub(crate) reader: BufReader<std::process::ChildStdout>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) fn eg_cmd(repo: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_eg"));
    c.arg("--root").arg(repo);
    c.env("ENGRAM_GLOBAL_DIR", repo.join(".test_global"));
    c
}

pub(crate) fn spawn(repo: &Path) -> Server {
    spawn_with(repo, true)
}

pub(crate) fn spawn_with(repo: &Path, all_tools: bool) -> Server {
    spawn_with_env(repo, all_tools, &[])
}

pub(crate) fn spawn_with_env(repo: &Path, all_tools: bool, envs: &[(&str, &Path)]) -> Server {
    let mut command = Command::new(env!("CARGO_BIN_EXE_eg"));
    command
        .arg("--root")
        .arg(repo)
        .env("ENGRAM_GLOBAL_DIR", repo.join(".test_global"))
        .arg("mcp")
        .args(all_tools.then_some("--all-tools"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }
    let mut child = command.spawn().unwrap();
    let stdin = child.stdin.take().unwrap();
    let reader = BufReader::new(child.stdout.take().unwrap());
    Server {
        child,
        stdin,
        reader,
    }
}

pub(crate) fn rpc(server: &mut Server, id: u64, method: &str, params: Value) -> Value {
    let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    writeln!(server.stdin, "{req}").unwrap();
    server.stdin.flush().unwrap();
    let mut line = String::new();
    server.reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).expect("valid JSON-RPC response")
}

pub(crate) fn notification(server: &mut Server, method: &str) {
    let req = json!({"jsonrpc": "2.0", "method": method});
    writeln!(server.stdin, "{req}").unwrap();
    server.stdin.flush().unwrap();
}

/// Initialize the server, send the initialized notification, and return the
/// advertised tool names from `tools/list`.
pub(crate) fn init_and_list_tools(server: &mut Server) -> Vec<String> {
    let init = rpc(
        server,
        1,
        "initialize",
        json!({"protocolVersion": "2025-06-18"}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "engram");
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

pub(crate) fn send_raw(server: &mut Server, frame: &str) {
    writeln!(server.stdin, "{frame}").unwrap();
    server.stdin.flush().unwrap();
}

pub(crate) fn read_response(server: &mut Server) -> Value {
    let mut line = String::new();
    server.reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).expect("valid JSON-RPC response")
}

pub(crate) fn result_text(resp: &Value) -> String {
    resp["result"]["content"][0]["text"]
        .as_str()
        .expect("text content")
        .to_string()
}

pub(crate) fn strip_footer(text: &str) -> &str {
    if let Some(pos) = text.rfind("\n───\n⚡ eg:") {
        &text[..=pos]
    } else if let Some(pos) = text.rfind("⚡ eg:") {
        &text[..pos]
    } else {
        text
    }
}

/// Temporary repository that deletes itself when the last handle drops.
///
/// The helper used to return a bare `PathBuf`, so every MCP test run left eleven `eg-mcp-*`
/// directories behind. Cloning shares one `Arc`, so a test that clones the path for a spawned
/// thread still cleans up exactly once.
#[derive(Clone)]
pub(crate) struct TempRepo(Arc<TempRepoInner>);

struct TempRepoInner(PathBuf);

impl Drop for TempRepoInner {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl std::ops::Deref for TempRepo {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0 .0
    }
}

impl AsRef<Path> for TempRepo {
    fn as_ref(&self) -> &Path {
        &self.0 .0
    }
}

impl AsRef<OsStr> for TempRepo {
    fn as_ref(&self) -> &OsStr {
        self.0 .0.as_os_str()
    }
}

fn make_dir(prefix: &str) -> TempRepo {
    let dir = std::env::temp_dir().join(format!("{prefix}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    TempRepo(Arc::new(TempRepoInner(dir)))
}

pub(crate) fn fixture(name: &str) -> TempRepo {
    let dir = make_dir(&format!("eg-mcp-{name}"));
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

pub(crate) fn mem_fixture(name: &str) -> TempRepo {
    make_dir(&format!("eg-mcp-mem-{name}"))
}

pub(crate) fn cli_mem(repo: &Path, args: &[&str]) -> (i32, String, String) {
    let out = eg_cmd(repo).args(["mem"]).args(args).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

pub(crate) fn spawn_with_global(repo: &Path, global_dir: &Path) -> Server {
    let mut child = Command::new(env!("CARGO_BIN_EXE_eg"))
        .arg("--root")
        .arg(repo)
        .env("ENGRAM_GLOBAL_DIR", global_dir)
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
