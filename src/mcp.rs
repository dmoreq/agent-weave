//! MCP stdio server (Amendment 8): newline-delimited JSON-RPC 2.0 over
//! stdin/stdout, spawned by the agent host — kg still spawns no daemons.
//! Tools map 1:1 to CLI subcommands and return the CLI's byte-identical
//! stdout text. Every tool call journals with `surface:"mcp"` (J2). The
//! process itself is not journaled.

use std::io::{BufRead, Read, Write};
use std::path::Path;
use std::time::Instant;

use serde_json::{json, Value};

use crate::error::KgError;

const PROTOCOL_VERSION: &str = "2025-06-18";

/// Tool registry: (name, description, inputSchema). `doctor` is deliberately
/// absent (Amendment 8): `--invalidate` is reserved for the human CLI.
fn tools() -> Value {
    let schema_obj = |props: Value, required: &[&str]| {
        json!({
            "type": "object",
            "properties": props,
            "required": required,
        })
    };
    let limit = json!({"type": "integer", "minimum": 1, "maximum": 10000});
    json!([
        {"name": "build", "description": "Index the repository into .kg/index.db (full atomic replacement)",
         "inputSchema": schema_obj(json!({
             "force": {"type": "boolean", "description": "Re-extract everything (disable the fast path)"},
             "max_parse_error_rate": {"type": "number", "description": "Abort above this fraction of parse-error files (default 0.05)"}
         }), &[])},
        {"name": "callers", "description": "Who calls this symbol — pointer list path:line",
         "inputSchema": schema_obj(json!({"name": {"type": "string"}, "limit": limit}), &["name"])},
        {"name": "callees", "description": "What does this symbol call — transitive with depth",
         "inputSchema": schema_obj(json!({"name": {"type": "string"}, "depth": {"type": "integer", "minimum": 1, "maximum": 10}, "limit": limit}), &["name"])},
        {"name": "path", "description": "Shortest call path between two symbols — hop call-site pointers path:line over resolved edges; dynamic dispatch may hide paths",
         "inputSchema": schema_obj(json!({"from": {"type": "string"}, "to": {"type": "string"}, "depth": {"type": "integer", "minimum": 1, "maximum": 10, "description": "Maximum hops (default 10)"}}), &["from", "to"])},
        {"name": "context", "description": "One-call symbol context: definition + signature, direct callers, direct callees over resolved edges",
         "inputSchema": schema_obj(json!({"name": {"type": "string"}, "limit": {"type": "integer", "minimum": 1, "maximum": 50, "description": "Callers/callees shown per section (default 10)"}}), &["name"])},
        {"name": "impact", "description": "Files that call into <file>",
         "inputSchema": schema_obj(json!({"file": {"type": "string"}, "limit": limit}), &["file"])},
        {"name": "dead", "description": "Dead-code candidates (zero inbound call edges) — candidates, never proof",
         "inputSchema": schema_obj(json!({"limit": limit}), &[])},
        {"name": "unresolved", "description": "Per-file unresolved-callee counts — where kg could not resolve",
         "inputSchema": schema_obj(json!({"limit": limit}), &[])},
        {"name": "symbols", "description": "Symbols of one indexed file, ordered by line; optional outline adds the one-line signature column",
         "inputSchema": schema_obj(json!({"file": {"type": "string"}, "outline": {"type": "boolean"}, "limit": limit}), &["file"])},
        {"name": "search", "description": "Symbol-level search: bm25-ranked FTS5 over split identifier terms",
         "inputSchema": schema_obj(json!({"terms": {"type": "string"}, "limit": limit}), &["terms"])},
        {"name": "clones", "description": "Duplicate-block candidates: merged 5-line shingle windows, paired",
         "inputSchema": schema_obj(json!({"min_lines": {"type": "integer", "minimum": 5}, "limit": limit}), &[])},

        {"name": "stats", "description": "Index totals: files, symbols, edges, parse errors, sizes",
         "inputSchema": schema_obj(json!({}), &[])},
        {"name": "hotspots", "description": "Pain-ranking of files: cyclomatic + LOC + churn-90d + fix-density",
         "inputSchema": schema_obj(json!({"limit": {"type": "integer", "minimum": 1, "maximum": 100}}), &[])},
        {"name": "report", "description": "Four-section architecture report",
         "inputSchema": schema_obj(json!({}), &[])},
        {"name": "usage", "description": "Usage-journal summary: calls, errors, latency, empty/stale rates; verify=true replays sampled payload pointers (Amendment 10)",
         "inputSchema": schema_obj(json!({"verify": {"type": "boolean"}, "limit": {"type": "integer", "minimum": 1, "maximum": 10000}}), &[])},
    ])
}

/// Serve until stdin closes. All protocol output goes to stdout; the only
/// stdout bytes ever written are complete JSON-RPC messages.
pub fn serve(root_arg: Option<&Path>, no_journal: bool) -> crate::error::Result<()> {
    let root = crate::walk::discover_root(root_arg)?;
    if no_journal {
        // Amendment 6 opt-out must hold on the MCP surface too (R6d A7).
        std::env::set_var("KG_NO_JOURNAL", "1");
    }
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    // Bounded frame size (R6d A7): the MCP input is untrusted; without a cap
    // a single oversized frame forces memory linear in its size.
    const MAX_FRAME_BYTES: u64 = 8 * 1024 * 1024;
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader
            .by_ref()
            .take(MAX_FRAME_BYTES + 1)
            .read_line(&mut line)?;
        if read == 0 {
            return Ok(());
        }
        if read as u64 > MAX_FRAME_BYTES {
            // Drain the rest of the oversized frame in fixed-size chunks so
            // memory stays bounded regardless of frame size (R6 finding).
            loop {
                let buf = reader.fill_buf()?;
                if buf.is_empty() {
                    break;
                }
                let nl = buf.iter().position(|&b| b == b'\n');
                let take = nl.map(|i| i + 1).unwrap_or(buf.len());
                reader.consume(take);
                if nl.is_some() {
                    break;
                }
            }
            write_error(&mut out, None, -32600, "frame too large")?;
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => {
                write_error(&mut out, None, -32700, "parse error")?;
                continue;
            }
        };
        let id = msg.get("id").cloned();
        let Some(method) = msg
            .get("method")
            .and_then(|m| m.as_str())
            .map(str::to_string)
        else {
            continue; // a response or malformed frame — no reply
        };
        let is_notification = id.is_none();
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
                    "serverInfo": {"name": "kg", "version": env!("CARGO_PKG_VERSION")},
                });
                write_result(&mut out, id, result)?;
            }
            "notifications/initialized" => {}
            "ping" => write_result(&mut out, id, json!({}))?,
            "tools/list" => write_result(&mut out, id, json!({"tools": tools()}))?,
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
                if !tools()
                    .as_array()
                    .map(|t| t.iter().any(|t| t["name"] == tool_name))
                    .unwrap_or(false)
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
                let started = Instant::now();
                // Panic isolation (R6d A7): one tool must not kill the server.
                let (text, is_error, exit_code) =
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        call_tool(&root, &tool_name, &arguments)
                    })) {
                        Ok(v) => v,
                        Err(_) => (
                            "kg: internal: tool panicked — server still running\n".to_string(),
                            true,
                            6,
                        ),
                    };
                crate::journal::finalize(
                    Some(&root),
                    exit_code,
                    started.elapsed().as_micros() as u64,
                );
                write_result(
                    &mut out,
                    id,
                    json!({"content": [{"type": "text", "text": text}], "isError": is_error}),
                )?;
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

fn arg_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

fn arg_usize(args: &Value, key: &str, default: usize) -> usize {
    args.get(key)
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(default)
}

fn arg_bool(args: &Value, key: &str) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Render one tool result: (text, is_error, exit_code).
fn call_tool(root: &Path, tool: &str, args: &Value) -> (String, bool, i32) {
    let outcome: crate::error::Result<String> = dispatch_tool(root, tool, args);
    match outcome {
        Ok(text) => (text, false, 0),
        Err(e) => {
            let mut text = String::new();
            if let KgError::NoResultsCaveated { caveat, .. } = &e {
                text.push_str(caveat);
                text.push('\n');
            }
            if let KgError::Ambiguous { candidates, .. } = &e {
                for (path, line, qualname) in candidates {
                    text.push_str(&format!("{path}:{line} {qualname}\n"));
                }
            }
            text.push_str(&e.to_string());
            text.push('\n');
            (text, true, e.exit_code())
        }
    }
}

fn dispatch_tool(root: &Path, tool: &str, args: &Value) -> crate::error::Result<String> {
    let journal_args = tool_journal_args(tool, args);
    crate::journal::begin("mcp", tool, &journal_args);
    match tool {
        "build" => Ok(format!(
            "{}\n",
            crate::build::run_build(
                Some(root),
                arg_bool(args, "force"),
                args.get("max_parse_error_rate")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.05),
            )?
        )),
        "callers" => {
            let conn = crate::db::open_for_query(root)?;
            let out = crate::query::callers(
                &conn,
                root,
                &arg_str(args, "name").unwrap_or_default(),
                arg_usize(args, "limit", crate::query::DEFAULT_LIMIT),
            )?;
            record(&out);
            Ok(crate::query::render_output(&out))
        }
        "callees" => {
            let conn = crate::db::open_for_query(root)?;
            let out = crate::query::callees(
                &conn,
                root,
                &arg_str(args, "name").unwrap_or_default(),
                arg_usize(args, "limit", crate::query::DEFAULT_LIMIT),
                arg_usize(args, "depth", 1),
            )?;
            record(&out);
            Ok(crate::query::render_output(&out))
        }
        "context" => {
            let conn = crate::db::open_for_query(root)?;
            Ok(join_lines(&crate::query::context(
                &conn,
                root,
                &arg_str(args, "name").unwrap_or_default(),
                arg_usize(args, "limit", 10),
            )?))
        }
        "path" => {
            let conn = crate::db::open_for_query(root)?;
            Ok(join_lines(&crate::query::path(
                &conn,
                root,
                &arg_str(args, "from").unwrap_or_default(),
                &arg_str(args, "to").unwrap_or_default(),
                arg_usize(args, "depth", 10),
            )?))
        }
        "impact" => {
            let conn = crate::db::open_for_query(root)?;
            let out = crate::query::impact(
                &conn,
                root,
                &arg_str(args, "file").unwrap_or_default(),
                arg_usize(args, "limit", crate::query::DEFAULT_LIMIT),
            )?;
            record(&out);
            Ok(crate::query::render_output(&out))
        }
        "dead" => {
            let conn = crate::db::open_for_query(root)?;
            let out = crate::query::dead(
                &conn,
                root,
                arg_usize(args, "limit", crate::query::DEFAULT_LIMIT),
            )?;
            record(&out);
            Ok(crate::query::render_output(&out))
        }
        "unresolved" => {
            let conn = crate::db::open_for_query(root)?;
            Ok(join_lines(&crate::query::unresolved(
                &conn,
                arg_usize(args, "limit", crate::query::DEFAULT_LIMIT),
            )?))
        }
        "symbols" => {
            let conn = crate::db::open_for_query(root)?;
            let outline = args
                .get("outline")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(join_lines(&crate::query::symbols(
                &conn,
                root,
                &arg_str(args, "file").unwrap_or_default(),
                outline,
                arg_usize(args, "limit", crate::query::DEFAULT_LIMIT),
            )?))
        }
        "search" => {
            let conn = crate::db::open_for_query(root)?;
            Ok(join_lines(&crate::query::search(
                &conn,
                root,
                &arg_str(args, "terms").unwrap_or_default(),
                arg_usize(args, "limit", crate::query::DEFAULT_LIMIT),
            )?))
        }
        "clones" => {
            let conn = crate::db::open_for_query(root)?;
            let min_lines = args
                .get("min_lines")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(5);
            Ok(join_lines(&crate::query::clones(
                &conn,
                root,
                min_lines,
                arg_usize(args, "limit", crate::query::DEFAULT_LIMIT),
            )?))
        }
        "stats" => Ok(format!(
            "{}\n",
            crate::insights::stats(&crate::db::open_for_query(root)?, root)?
        )),
        "hotspots" => Ok(join_lines(&crate::insights::hotspots(
            &crate::db::open_for_query(root)?,
            root,
            arg_usize(args, "limit", 10),
        )?)),
        "report" => Ok(join_lines(&crate::insights::report(
            &crate::db::open_for_query(root)?,
            root,
        )?)),
        "usage" => {
            if args
                .get("verify")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                let conn = crate::db::open_for_query(root)?;
                Ok(join_lines(&crate::journal::verify(
                    root,
                    &conn,
                    arg_usize(args, "limit", 50),
                )?))
            } else {
                Ok(join_lines(&crate::journal::summary(root)?))
            }
        }
        _ => Err(KgError::Usage(format!("unknown tool '{tool}'"))),
    }
}

fn record(out: &crate::query::QueryOutput) {
    crate::journal::record_result(
        out.pointers.len(),
        out.shown,
        out.total,
        out.stale,
        out.pointers.iter().filter(|p| !p.tag.is_empty()).count(),
    );
}

fn join_lines(lines: &[String]) -> String {
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    text
}

fn tool_journal_args(tool: &str, args: &Value) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    match tool {
        "build" => out.push(("force", arg_bool(args, "force").to_string())),
        "callers" => {
            if let Some(name) = arg_str(args, "name") {
                out.push(("name", name));
            }
            out.push((
                "limit",
                arg_usize(args, "limit", crate::query::DEFAULT_LIMIT).to_string(),
            ));
        }
        "callees" => {
            if let Some(name) = arg_str(args, "name") {
                out.push(("name", name));
            }
            out.push(("depth", arg_usize(args, "depth", 1).to_string()));
            out.push((
                "limit",
                arg_usize(args, "limit", crate::query::DEFAULT_LIMIT).to_string(),
            ));
        }
        "context" => {
            if let Some(name) = arg_str(args, "name") {
                out.push(("name", name));
            }
            out.push(("limit", arg_usize(args, "limit", 10).to_string()));
        }
        "path" => {
            if let Some(from) = arg_str(args, "from") {
                out.push(("name", from));
            }
            if let Some(to) = arg_str(args, "to") {
                out.push(("to", to));
            }
            out.push(("depth", arg_usize(args, "depth", 10).to_string()));
        }
        "impact" => {
            if let Some(file) = arg_str(args, "file") {
                out.push(("file", file));
            }
            out.push((
                "limit",
                arg_usize(args, "limit", crate::query::DEFAULT_LIMIT).to_string(),
            ));
        }
        "symbols" => {
            if let Some(file) = arg_str(args, "file") {
                out.push(("file", file));
            }
            let outline = args
                .get("outline")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            out.push(("outline", outline.to_string()));
            out.push((
                "limit",
                arg_usize(args, "limit", crate::query::DEFAULT_LIMIT).to_string(),
            ));
        }
        "dead" | "unresolved" | "hotspots" => {
            out.push(("limit", arg_usize(args, "limit", 50).to_string()));
        }
        "search" => {
            if let Some(terms) = arg_str(args, "terms") {
                out.push(("terms", terms));
            }
            out.push((
                "limit",
                arg_usize(args, "limit", crate::query::DEFAULT_LIMIT).to_string(),
            ));
        }
        "clones" => {
            out.push((
                "min_lines",
                args.get("min_lines")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize)
                    .unwrap_or(5)
                    .to_string(),
            ));
            out.push((
                "limit",
                arg_usize(args, "limit", crate::query::DEFAULT_LIMIT).to_string(),
            ));
        }
        "usage" => {
            if args
                .get("verify")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                out.push(("verify", "true".to_string()));
                out.push(("limit", arg_usize(args, "limit", 50).to_string()));
            }
        }
        _ => {}
    }
    out
}
