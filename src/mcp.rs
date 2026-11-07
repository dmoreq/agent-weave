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

use crate::service::{from_tool_args, CommandRequest, ParamKind, Surface, CORE_TOOLS, TOOL_SPECS};

const PROTOCOL_VERSION: &str = "2025-06-18";

/// Tool registry rendered from the shared specs; `doctor` is deliberately
/// absent: `--invalidate` is reserved for the human CLI.
fn tools(all: bool) -> Value {
    let list: Vec<Value> = TOOL_SPECS
        .iter()
        .filter(|spec| all || CORE_TOOLS.contains(&spec.name))
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
                        json!({"type": "array", "items": {"type": "string"}, "maxItems": 1000})
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
/// stdout bytes ever written are complete JSON-RPC messages.
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
    // Bounded frame size (R6d A7): the MCP input is untrusted; without a cap
    // a single oversized frame forces memory linear in its size.
    const MAX_FRAME_BYTES: u64 = 8 * 1024 * 1024;
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        let read = reader
            .by_ref()
            .take(MAX_FRAME_BYTES + 1)
            .read_until(b'\n', &mut buf)?;
        if read == 0 {
            return Ok(());
        }
        let newline_seen = buf.last() == Some(&b'\n');
        if read as u64 > MAX_FRAME_BYTES {
            // Drain the rest of the oversized frame in fixed-size chunks so
            // memory stays bounded. If the bounded read already consumed the
            // delimiter, do not drain: the next line is a fresh request.
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
            write_error(&mut out, None, -32600, "frame too large")?;
            continue;
        }
        let frame = match std::str::from_utf8(&buf) {
            Ok(text) => text.trim(),
            Err(_) => {
                write_error(&mut out, None, -32700, "parse error")?;
                continue;
            }
        };
        if frame.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(frame) {
            Ok(v) => v,
            Err(_) => {
                write_error(&mut out, None, -32700, "parse error")?;
                continue;
            }
        };
        let id = msg.get("id").cloned();
        let is_notification = id.is_none();
        if !msg.is_object() {
            write_error(&mut out, None, -32600, "invalid request")?;
            continue;
        }
        if msg.get("jsonrpc").and_then(|v| v.as_str()) != Some("2.0") {
            if !is_notification {
                write_error(
                    &mut out,
                    id,
                    -32600,
                    "invalid request: jsonrpc must be \"2.0\"",
                )?;
            }
            continue;
        }
        let Some(method_value) = msg.get("method") else {
            // A client response carries result/error and no method; ignore it.
            // Any other envelope without a method is an invalid request.
            if !is_notification && msg.get("result").is_none() && msg.get("error").is_none() {
                write_error(&mut out, id, -32600, "invalid request: missing method")?;
            }
            continue;
        };
        let Some(method) = method_value.as_str().map(str::to_string) else {
            if !is_notification {
                write_error(
                    &mut out,
                    id,
                    -32600,
                    "invalid request: method must be a string",
                )?;
            }
            continue;
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
                if !is_notification {
                    write_result(&mut out, id, result)?;
                }
            }
            "notifications/initialized" => {}
            "ping" => {
                if !is_notification {
                    write_result(&mut out, id, json!({}))?;
                }
            }
            "tools/list" => {
                if !is_notification {
                    write_result(&mut out, id, json!({"tools": tools(all_tools)}))?;
                }
            }
            "tools/call" => {
                let params = msg.get("params").cloned().unwrap_or(Value::Null);
                let tool_name = params
                    .get("name")
                    .and_then(|n| n.as_str())
                    .map(str::to_string);
                let Some(tool_name) = tool_name else {
                    if !is_notification {
                        write_error(&mut out, id, -32602, "missing tool name")?;
                    }
                    continue;
                };
                if !TOOL_SPECS
                    .iter()
                    .any(|t| (all_tools || CORE_TOOLS.contains(&t.name)) && t.name == tool_name)
                {
                    if !is_notification {
                        write_error(
                            &mut out,
                            id,
                            -32602,
                            format!("unknown tool '{tool_name}'").as_str(),
                        )?;
                    }
                    continue;
                }
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                // Protocol-level invalid params (R7 L4): a missing required
                // string must be -32602, not a downstream "symbol '' not in
                // index" tool result that masks client bugs.
                if let Some(reason) = missing_required_arg(&tool_name, &arguments) {
                    if !is_notification {
                        write_error(&mut out, id, -32602, &reason)?;
                    }
                    continue;
                }
                let started = Instant::now();
                // Panic isolation (R6d A7): one tool must not kill the server.
                let (text, is_error, exit_code) =
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        call_tool(&root, &tool_name, &arguments)
                    })) {
                        Ok(v) => v,
                        Err(_) => (
                            "aw: internal: tool panicked — server still running\n".to_string(),
                            true,
                            6,
                        ),
                    };
                // Finalize covers the panic path (the pending journal line is
                // still armed); non-panic paths were finalized in the service.
                crate::journal::finalize(
                    Some(&root),
                    exit_code,
                    started.elapsed().as_micros() as u64,
                );
                if !is_notification {
                    write_result(
                        &mut out,
                        id,
                        json!({"content": [{"type": "text", "text": text}], "isError": is_error}),
                    )?;
                }
            }
            _ => {
                if !is_notification {
                    write_error(
                        &mut out,
                        id,
                        -32601,
                        format!("method not found: {method}").as_str(),
                    )?;
                }
            }
        }
    }
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
