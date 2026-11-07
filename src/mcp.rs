//! MCP stdio server (Amendment 8): newline-delimited JSON-RPC 2.0 over
//! stdin/stdout, spawned by the agent host — aw still spawns no daemons.
//! Tools correspond to CLI subcommands and return the CLI's byte-identical
//! stdout text; `--all-tools` controls the exposed set. Tool schemas and the
//! required-argument contract are rendered from the single `service::TOOL_SPECS`
//! table, and every call dispatches through `service::execute`, so the MCP
//! surface cannot drift from the CLI.

use std::io::{BufRead, Read, Write};
use std::path::Path;
use std::time::Instant;

use serde_json::{json, Value};

use crate::service::{
    from_tool_args, CommandRequest, ParamKind, Surface, MAX_STR_ARRAY_ITEMS, TOOL_SPECS,
};

const PROTOCOL_VERSION: &str = "2025-06-18";

/// Tool registry rendered from the shared specs; `doctor` is deliberately
/// absent: `--invalidate` is reserved for the human CLI.
fn tools(all: bool) -> Value {
    let list: Vec<Value> = TOOL_SPECS
        .iter()
        .filter(|spec| all || spec.core)
        .map(|spec| {
            let mut props = serde_json::Map::new();
            let mut required: Vec<Value> = Vec::new();
            for param in spec.params {
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
                props.insert(param.name.to_string(), schema);
                if param.required {
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
        })
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
    let root = crate::walk::discover_root(root_arg)?;
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
            Frame::Message(msg) => handle_message(msg, &root, all_tools, &mut out)?,
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
        }
        write_error(out, None, -32600, "frame too large")?;
        return Ok(Frame::Skip);
    }
    let frame = match std::str::from_utf8(buf) {
        Ok(text) => text.trim(),
        Err(_) => {
            write_error(out, None, -32700, "parse error")?;
            return Ok(Frame::Skip);
        }
    };
    if frame.is_empty() {
        return Ok(Frame::Skip);
    }
    match serde_json::from_str(frame) {
        Ok(v) => Ok(Frame::Message(v)),
        Err(_) => {
            write_error(out, None, -32700, "parse error")?;
            Ok(Frame::Skip)
        }
    }
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
        write_error(out, None, -32600, "invalid request")?;
        return Ok(None);
    }
    let id = msg.get("id").cloned();
    if msg.get("jsonrpc").and_then(|v| v.as_str()) != Some("2.0") {
        reply_error(out, id, -32600, "invalid request: jsonrpc must be \"2.0\"")?;
        return Ok(None);
    }
    let Some(method_value) = msg.get("method") else {
        // A client response carries result/error and no method; ignore it.
        // Any other envelope without a method is an invalid request.
        if msg.get("result").is_none() && msg.get("error").is_none() {
            reply_error(out, id, -32600, "invalid request: missing method")?;
        }
        return Ok(None);
    };
    let Some(method) = method_value.as_str().map(str::to_string) else {
        reply_error(out, id, -32600, "invalid request: method must be a string")?;
        return Ok(None);
    };
    Ok(Some((id, method)))
}

/// Dispatch one validated JSON-RPC message; the envelope guards live in
/// `validate_envelope` and already wrote any error reply.
fn handle_message(
    msg: Value,
    root: &Path,
    all_tools: bool,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let Some((id, method)) = validate_envelope(&msg, out)? else {
        return Ok(());
    };
    match method.as_str() {
        "initialize" => {
            let requested = msg
                .get("params")
                .and_then(|p| p.get("protocolVersion"))
                .and_then(|v| v.as_str())
                .unwrap_or(PROTOCOL_VERSION)
                .to_string();
            let result = json!({
                "protocolVersion": requested,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "agent-weave", "version": env!("CARGO_PKG_VERSION")},
            });
            reply_result(out, id, result)?;
        }
        "notifications/initialized" => {}
        "ping" => {
            reply_result(out, id, json!({}))?;
        }
        "tools/list" => {
            reply_result(out, id, json!({"tools": tools(all_tools)}))?;
        }
        "tools/call" => {
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            handle_tools_call(&params, root, all_tools, id, out)?;
        }
        _ => {
            reply_error(out, id, -32601, &format!("method not found: {method}"))?;
        }
    }
    Ok(())
}

/// One `tools/call`: validate the tool name and required args, run the tool
/// behind a panic boundary, and write the result.
fn handle_tools_call(
    params: &Value,
    root: &Path,
    all_tools: bool,
    id: Option<Value>,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let tool_name = params
        .get("name")
        .and_then(|n| n.as_str())
        .map(str::to_string);
    let Some(tool_name) = tool_name else {
        return reply_error(out, id, -32602, "missing tool name");
    };
    if !TOOL_SPECS
        .iter()
        .any(|t| (all_tools || t.core) && t.name == tool_name)
    {
        return reply_error(out, id, -32602, &format!("unknown tool '{tool_name}'"));
    }
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
    // Protocol-level invalid params (R7 L4): a missing required string must
    // be -32602, not a downstream "symbol '' not in index" tool result that
    // masks client bugs.
    if let Some(reason) = missing_required_arg(&tool_name, &arguments) {
        return reply_error(out, id, -32602, &reason);
    }
    let started = Instant::now();
    // Panic isolation (R6d A7): one tool must not kill the server.
    let (text, is_error, exit_code) =
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            call_tool(root, &tool_name, &arguments)
        })) {
            Ok(v) => v,
            Err(_) => (
                "aw: internal: tool panicked — server still running\n".to_string(),
                true,
                6,
            ),
        };
    let latency_us = started.elapsed().as_micros() as u64;
    let latency_ms = latency_us as f64 / 1000.0;
    // Finalize covers the panic path (the pending journal line is still
    // armed); non-panic paths were finalized in the service.
    crate::journal::finalize(Some(root), exit_code, latency_us);
    let text = format_mcp_output(&tool_name, text, is_error, latency_ms);
    reply_result(
        out,
        id,
        json!({"content": [{"type": "text", "text": text}], "isError": is_error}),
    )
}

fn format_mcp_output(tool: &str, text: String, is_error: bool, latency_ms: f64) -> String {
    // Project memory notes (mem_*) and telemetry (usage) are not
    // code-indexing / navigation queries; they maintain clean report format
    // without appending navigation KPI footers.
    if tool.starts_with("mem_") || tool == "usage" {
        return text;
    }

    if is_error || text.trim().is_empty() {
        if text.trim().is_empty() {
            return format!("⚡ aw: {latency_ms:.1}ms | 0 items\n");
        }
        let mut out = text.trim_end().to_string();
        out.push_str(&format!("\n───\n⚡ aw: {latency_ms:.1}ms | 0 items\n"));
        return out;
    }

    let (shown, file_count) = count_items_and_files(tool, &text);
    let output_tokens = text.len() / 4;

    let saved_tokens = if shown == 0 {
        0
    } else {
        match tool {
            "context" | "callers" | "callees" | "path" | "impact" | "imports" | "symbols" => {
                (file_count * 400).saturating_sub(output_tokens)
            }
            "search" => (file_count.max(2) * 350).saturating_sub(output_tokens),
            "reuse" => (shown * 800 + 1200).saturating_sub(output_tokens),
            "build" => (file_count * 300).saturating_sub(output_tokens),
            _ => 0,
        }
    };

    let mut out = text.trim_end().to_string();
    if saved_tokens > 0 {
        let saved_str = format_token_count(saved_tokens);
        out.push_str(&format!(
            "\n───\n⚡ aw: {latency_ms:.1}ms | {shown} items | ~{saved_str} tokens saved vs file reads\n"
        ));
    } else {
        out.push_str(&format!(
            "\n───\n⚡ aw: {latency_ms:.1}ms | {shown} items\n"
        ));
    }
    out
}

fn format_token_count(tokens: usize) -> String {
    if tokens >= 1000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        tokens.to_string()
    }
}

fn count_items_and_files(tool: &str, text: &str) -> (usize, usize) {
    let mut files = std::collections::HashSet::new();

    if tool == "reuse" {
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("aw: reuse — ") {
                if let Some(num_str) = rest.split_whitespace().next() {
                    if let Ok(n) = num_str.parse::<usize>() {
                        return (n, n.max(1));
                    }
                }
            }
            if line.contains("0 reuse candidates found") {
                return (0, 0);
            }
        }
    }

    if tool == "build" {
        for line in text.lines() {
            if let Some(pos) = line.find("files=") {
                let rest = &line[pos + 6..];
                let count_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(n) = count_str.parse::<usize>() {
                    return (n, n);
                }
            }
        }
    }

    let mut item_count = 0;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("───")
            || trimmed.starts_with("aw:")
            || trimmed.starts_with("[aw]")
            || trimmed.starts_with("showing ")
            || trimmed.starts_with("top unresolved")
            || trimmed == "callers:"
            || trimmed == "callees:"
            || trimmed.starts_with("callers (")
            || trimmed.starts_with("callees (")
        {
            continue;
        }

        item_count += 1;

        let token =
            trimmed.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ' ');
        if let Some(pos) = token.find(':') {
            let candidate = &token[..pos];
            if candidate.contains('.') && !candidate.contains(' ') {
                files.insert(candidate.to_string());
            }
        } else if let Some(first_word) = token.split_whitespace().next() {
            if first_word.contains('.')
                && (first_word.ends_with(".rs")
                    || first_word.ends_with(".ts")
                    || first_word.ends_with(".js")
                    || first_word.ends_with(".py"))
            {
                files.insert(first_word.to_string());
            }
        }
    }

    let file_count = files.len().max(if item_count > 0 { 1 } else { 0 });
    (item_count, file_count)
}

fn write_result(out: &mut impl Write, id: Option<Value>, result: Value) -> std::io::Result<()> {
    let msg = json!({"jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result});
    writeln!(out, "{msg}")
}

fn write_error(
    out: &mut impl Write,
    id: Option<Value>,
    code: i64,
    message: &str,
) -> std::io::Result<()> {
    let msg = json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": {"code": code, "message": message},
    });
    writeln!(out, "{msg}")
}

/// Reply to a request; a notification (no id) gets no response frame.
fn reply_result(out: &mut impl Write, id: Option<Value>, result: Value) -> std::io::Result<()> {
    if id.is_some() {
        write_result(out, id, result)?;
    }
    Ok(())
}

fn reply_error(
    out: &mut impl Write,
    id: Option<Value>,
    code: i64,
    message: &str,
) -> std::io::Result<()> {
    if id.is_some() {
        write_error(out, id, code, message)?;
    }
    Ok(())
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

/// Render one tool result: (text, is_error, exit_code). Success text is the
/// CLI's stdout; error text is the CLI's stdout (caveat/candidates) followed
/// by its stderr diagnostic — Amendment 8's error contract.
fn call_tool(root: &Path, tool: &str, args: &Value) -> (String, bool, i32) {
    let outcome = from_tool_args(tool, args).map(|req: CommandRequest| {
        let exec = crate::service::execute(&req, Some(root), Surface::Mcp, true);
        (exec.stdout, exec.stderr, exec.exit_code)
    });
    match outcome {
        Ok((stdout, _stderr, 0)) => (stdout, false, 0),
        Ok((stdout, stderr, code)) => {
            let mut text = stdout;
            text.push_str(&stderr);
            (text, true, code)
        }
        Err(e) => (format!("{e}\n"), true, e.exit_code()),
    }
}
