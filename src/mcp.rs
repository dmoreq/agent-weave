//! MCP stdio server (Amendment 8): newline-delimited JSON-RPC 2.0 over
//! stdin/stdout, spawned by the agent host — eg still spawns no daemons.
//! Tools correspond to CLI subcommands and return the CLI's byte-identical
//! stdout text; `--all-tools` controls the exposed set. Tool schemas and the
//! required-argument contract are rendered from the single `service::TOOL_SPECS`
//! table, and every call dispatches through `service::execute`, so the MCP
//! surface cannot drift from the CLI.

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::{json, Value};

use crate::service::{
    from_tool_args, CommandRequest, ParamKind, ParamSpec, Surface, ToolSpec, MAX_STR_ARRAY_ITEMS,
    TOOL_SPECS,
};

mod format;
use format::*;

const PROTOCOL_VERSION: &str = "2025-06-18";

/// Tool registry rendered from the shared specs; `doctor` is deliberately
fn param_schema(param: &ParamSpec) -> (String, Value, bool) {
    let mut schema = match &param.kind {
        ParamKind::Str => json!({"type": "string"}),
        ParamKind::Int { min, max } => {
            let mut s = json!({"type": "integer", "minimum": min});
            if let Some(max) = max {
                s["maximum"] = json!(max);
            }
            s
        }
        ParamKind::Float { min, max } => {
            json!({"type": "number", "minimum": min, "maximum": max})
        }
        ParamKind::Bool => json!({"type": "boolean"}),
        ParamKind::StrArray => {
            json!({
                "type": "array",
                "items": {"type": "string"},
                "maxItems": MAX_STR_ARRAY_ITEMS
            })
        }
    };
    schema["description"] = json!(param.description);
    (param.name.to_string(), schema, param.required)
}

fn tool_schema(spec: &ToolSpec) -> Value {
    let mut props = serde_json::Map::new();
    let mut required: Vec<Value> = Vec::new();
    for param in spec.params {
        let (name, schema, is_required) = param_schema(param);
        props.insert(name, schema);
        if is_required {
            required.push(json!(param.name));
        }
    }
    json!({
        "name": spec.name,
        "description": spec.description,
        "inputSchema": {
            "type": "object",
            "properties": Value::Object(props),
            "required": required,
        },
    })
}

/// absent: `--invalidate` is reserved for the human CLI.
fn tools(all: bool) -> Value {
    let list: Vec<Value> = TOOL_SPECS
        .iter()
        .filter(|spec| all || spec.core)
        .map(tool_schema)
        .collect();
    Value::Array(list)
}

/// Serve until stdin closes. All protocol output goes to stdout; the only
/// stdout bytes ever written are complete JSON-RPC messages. One loop owns
/// the protocol state; tool dispatch is the only extraction boundary.
pub fn serve(
    root_arg: Option<&Path>,
    no_journal: bool,
    all_tools: bool,
) -> crate::error::Result<()> {
    let mut current_root = crate::walk::discover_root(root_arg).ok();
    if no_journal {
        // Amendment 6 opt-out must hold on the MCP surface too (R6d A7).
        crate::journal::disable();
    }
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut buf: Vec<u8> = Vec::new();
    loop {
        match read_frame(&mut reader, &mut out, &mut buf)? {
            Frame::Eof => return Ok(()),
            Frame::Skip => continue,
            Frame::Message(msg) => handle_message(msg, &mut current_root, all_tools, &mut out)?,
        }
    }
}

/// One bounded input frame: a parsed message, a skipped frame (error already
/// written), or end of input.
enum Frame {
    Eof,
    Skip,
    Message(Value),
}

/// Read one newline-delimited frame. The MCP input is untrusted: the 8 MiB
/// cap keeps memory bounded, and an oversized frame is drained in chunks so
/// the next request still starts at a clean line.
fn read_frame(
    reader: &mut impl BufRead,
    out: &mut impl Write,
    buf: &mut Vec<u8>,
) -> std::io::Result<Frame> {
    const MAX_FRAME_BYTES: u64 = 8 * 1024 * 1024;
    buf.clear();
    let read = reader
        .by_ref()
        .take(MAX_FRAME_BYTES + 1)
        .read_until(b'\n', buf)?;
    if read == 0 {
        return Ok(Frame::Eof);
    }
    let newline_seen = buf.last() == Some(&b'\n');
    if read as u64 > MAX_FRAME_BYTES {
        // Drain the rest of the oversized frame. If the bounded read already
        // consumed the delimiter, do not drain: the next line is a fresh request.
        if !newline_seen {
            drain_oversized_frame(reader)?;
        }
        write_error(out, Some(Value::Null), -32600, "frame too large")?;
        return Ok(Frame::Skip);
    }
    let frame = match std::str::from_utf8(buf) {
        Ok(text) => text.trim(),
        Err(_) => {
            write_error(out, Some(Value::Null), -32700, "parse error")?;
            return Ok(Frame::Skip);
        }
    };
    if frame.is_empty() {
        return Ok(Frame::Skip);
    }
    match serde_json::from_str(frame) {
        Ok(v) => Ok(Frame::Message(v)),
        Err(_) => {
            write_error(out, Some(Value::Null), -32700, "parse error")?;
            Ok(Frame::Skip)
        }
    }
}

/// Drain an oversized frame chunk by chunk until its newline delimiter (or
/// EOF), so the next request starts at a clean line.
fn drain_oversized_frame(reader: &mut impl BufRead) -> std::io::Result<()> {
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        let nl = chunk.iter().position(|&b| b == b'\n');
        let take = nl.map(|i| i + 1).unwrap_or(chunk.len());
        reader.consume(take);
        if nl.is_some() {
            break;
        }
    }
    Ok(())
}

/// Validate the JSON-RPC envelope guards (object shape, protocol version, a
/// string method) and return the frame id with its method; `None` when the
/// frame was rejected (the error reply is already written) or is a client
/// response without a method (silently ignored). Every early return matches
/// the original `continue`: the loop reads the next frame.
fn validate_envelope(
    msg: &Value,
    out: &mut impl Write,
) -> std::io::Result<Option<(Option<Value>, String)>> {
    if !msg.is_object() {
        write_error(out, Some(Value::Null), -32600, "invalid request")?;
        return Ok(None);
    }
    let id = msg.get("id").cloned();
    if msg.get("jsonrpc").and_then(|v| v.as_str()) != Some("2.0") {
        write_error(out, id, -32600, "invalid request: jsonrpc must be \"2.0\"")?;
        return Ok(None);
    }
    let Some(method_value) = msg.get("method") else {
        // A client response carries result/error and no method; ignore it.
        // Any other envelope without a method is an invalid request.
        if msg.get("result").is_none() && msg.get("error").is_none() {
            write_error(out, id, -32600, "invalid request: missing method")?;
        }
        return Ok(None);
    };
    let Some(method) = method_value.as_str().map(str::to_string) else {
        write_error(out, id, -32600, "invalid request: method must be a string")?;
        return Ok(None);
    };
    Ok(Some((id, method)))
}

/// Dispatch one validated JSON-RPC message; the envelope guards live in
/// `validate_envelope` and already wrote any error reply.
fn handle_message(
    msg: Value,
    root: &mut Option<PathBuf>,
    all_tools: bool,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let Some((id, method)) = validate_envelope(&msg, out)? else {
        return Ok(());
    };
    match method.as_str() {
        "initialize" => {
            handle_initialize(msg.get("params"), root, id, out)?;
        }
        "initialized" | "notifications/initialized" => {}
        "ping" => {
            write_result(out, id, json!({}))?;
        }
        "tools/list" => {
            write_result(out, id, json!({"tools": tools(all_tools)}))?;
        }
        "tools/call" => {
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            handle_tools_call(&params, root, all_tools, id, out)?;
        }
        _ => {
            write_error(out, id, -32601, &format!("method not found: {method}"))?;
        }
    }
    Ok(())
}

fn handle_initialize(
    params: Option<&Value>,
    root: &mut Option<PathBuf>,
    id: Option<Value>,
    out: &mut impl Write,
) -> std::io::Result<()> {
    if let Some(p) = params {
        if let Some(client_name) = p
            .get("clientInfo")
            .and_then(|c| c.get("name"))
            .and_then(|n| n.as_str())
        {
            crate::journal::set_agent_context(client_name);
        }
    }
    if root.is_none() {
        if let Some(p) = params {
            let root_candidate = p
                .get("rootUri")
                .and_then(|u| u.as_str())
                .or_else(|| p.get("rootPath").and_then(|u| u.as_str()))
                .or_else(|| {
                    p.get("workspaceFolders")
                        .and_then(|wf| wf.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|f| f.get("uri"))
                        .and_then(|u| u.as_str())
                });
            if let Some(uri) = root_candidate {
                let path_str = uri.strip_prefix("file://").unwrap_or(uri);
                let path = Path::new(path_str);
                if let Some(r) = crate::walk::find_git_root(path) {
                    *root = Some(r);
                } else if let Ok(r) = crate::walk::discover_root(Some(path)) {
                    *root = Some(r);
                }
            }
        }
    }
    let requested = params
        .and_then(|p| p.get("protocolVersion"))
        .and_then(|v| v.as_str())
        .unwrap_or(PROTOCOL_VERSION)
        .to_string();
    let result = json!({
        "protocolVersion": requested,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "engram", "version": env!("CARGO_PKG_VERSION")},
    });
    write_result(out, id, result)
}

/// One `tools/call`: validate the tool name and required args, run the tool
/// behind a panic boundary, and write the result.
fn handle_tools_call(
    params: &Value,
    root: &mut Option<PathBuf>,
    all_tools: bool,
    id: Option<Value>,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let tool_name = params
        .get("name")
        .and_then(|n| n.as_str())
        .map(str::to_string);
    let Some(tool_name) = tool_name else {
        return write_error(out, id, -32602, "missing tool name");
    };
    if !TOOL_SPECS
        .iter()
        .any(|t| (all_tools || t.core) && t.name == tool_name)
    {
        return write_error(out, id, -32602, &format!("unknown tool '{tool_name}'"));
    }
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
    // Protocol-level invalid params (R7 L4): a missing required string must
    // be -32602, not a downstream "symbol '' not in index" tool result that
    // masks client bugs.
    if let Some(reason) = missing_required_arg(&tool_name, &arguments) {
        return write_error(out, id, -32602, &reason);
    }
    infer_active_root(root, &arguments);
    let Some(active_root) = root.as_deref() else {
        return write_result(
            out,
            id,
            json!({
                "content": [{"type": "text", "text": "eg: usage: no git root ancestor found — initialize MCP with rootUri or run inside a git repository\n"}],
                "isError": true
            }),
        );
    };
    let (text, is_error, latency_ms, shown) =
        run_tool_journaled(active_root, &tool_name, &arguments);
    let text = format_mcp_output(&tool_name, text, is_error, shown, latency_ms);
    write_result(
        out,
        id,
        json!({"content": [{"type": "text", "text": text}], "isError": is_error}),
    )
}

fn infer_active_root(root: &mut Option<PathBuf>, arguments: &Value) {
    if root.is_some() {
        return;
    }
    if let Some(file_val) = arguments
        .get("file")
        .or_else(|| arguments.get("path"))
        .and_then(|v| v.as_str())
    {
        let p = Path::new(file_val);
        let dir = if p.is_dir() {
            p
        } else {
            p.parent().unwrap_or(p)
        };
        let start = if dir.is_absolute() {
            dir.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|c| c.join(dir))
                .unwrap_or_else(|_| dir.to_path_buf())
        };
        if let Some(r) = crate::walk::find_git_root(&start) {
            *root = Some(r);
            return;
        } else if let Ok(r) = crate::walk::discover_root(Some(&start)) {
            *root = Some(r);
            return;
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(r) = crate::walk::find_git_root(&cwd) {
            *root = Some(r);
        } else if let Ok(r) = crate::walk::discover_root(Some(&cwd)) {
            *root = Some(r);
        }
    }
}

/// Panic boundary + journal tail of one `tools/call`: one tool must not kill
/// the server (R6d A7), and the journal line is finalized on the panic path
/// too (the pending line is still armed; non-panic paths were finalized in
/// the service). Returns the tool text, the error flag, the footer latency,
/// and the service-arm shown count.
fn run_tool_journaled(
    root: &Path,
    tool_name: &str,
    arguments: &Value,
) -> (String, bool, f64, Option<usize>) {
    let started = Instant::now();
    // Panic isolation (R6d A7): one tool must not kill the server.
    let (text, is_error, exit_code, shown) =
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            call_tool(root, tool_name, arguments)
        })) {
            Ok(v) => v,
            Err(_) => (
                "eg: internal: tool panicked — server still running\n".to_string(),
                true,
                6,
                None,
            ),
        };
    let latency_us = started.elapsed().as_micros() as u64;
    let latency_ms = latency_us as f64 / 1000.0;
    // Finalize covers the panic path (the pending journal line is still
    // armed); non-panic paths were finalized in the service.
    crate::journal::finalize(Some(root), exit_code, latency_us);
    (text, is_error, latency_ms, shown)
}

/// Required-argument check rendered from the shared specs (protocol-level
/// validation mirrors inputSchema `required`).
fn missing_required_arg(tool: &str, args: &Value) -> Option<String> {
    let spec = TOOL_SPECS.iter().find(|t| t.name == tool)?;
    for param in spec.params {
        if !param.required {
            continue;
        }
        let absent = match &param.kind {
            ParamKind::StrArray => args
                .get(param.name)
                .and_then(|v| v.as_array())
                .map(|a| a.is_empty())
                .unwrap_or(true),
            ParamKind::Str => args
                .get(param.name)
                .and_then(|v| v.as_str())
                .map(|v| v.trim().is_empty())
                .unwrap_or(true),
            _ => args.get(param.name).is_none(),
        };
        if absent {
            return Some(format!(
                "missing required argument '{}' for tool '{tool}'",
                param.name
            ));
        }
    }
    None
}

/// Render one tool result: (text, is_error, exit_code, shown items). Success
/// text is the CLI's stdout; error text is the CLI's stdout (caveat/candidates)
/// followed by its stderr diagnostic — Amendment 8's error contract.
fn call_tool(root: &Path, tool: &str, args: &Value) -> (String, bool, i32, Option<usize>) {
    let outcome = from_tool_args(tool, args).map(|req: CommandRequest| {
        let journal = !matches!(
            req,
            CommandRequest::Usage { .. } | CommandRequest::Doctor { .. }
        );
        let exec = crate::service::execute(&req, Some(root), Surface::Mcp, journal);
        (exec.stdout, exec.stderr, exec.exit_code, exec.shown)
    });
    match outcome {
        Ok((mut stdout, stderr, 0, shown)) => {
            if tool == "build" && !stderr.is_empty() {
                if !stdout.is_empty() && !stdout.ends_with('\n') {
                    stdout.push('\n');
                }
                stdout.push_str(&stderr);
            }
            (stdout, false, 0, shown)
        }
        Ok((stdout, stderr, code, _shown)) => {
            let mut text = stdout;
            text.push_str(&stderr);
            (text, true, code, None)
        }
        Err(e) => (format!("{e}\n"), true, e.exit_code(), None),
    }
}
