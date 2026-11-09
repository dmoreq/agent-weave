//! MCP formatting, response framing, and token/pricing footers.

use serde_json::{json, Value};
use std::io::Write;

pub(super) fn format_mcp_output(
    tool: &str,
    text: String,
    is_error: bool,
    shown: Option<usize>,
    latency_ms: f64,
) -> String {
    // Project memory notes (mem_*) and telemetry (usage) are not
    // code-indexing / navigation queries; they maintain clean report format
    // without appending navigation KPI footers.
    if tool.starts_with("mem_") || tool == "usage" {
        return text;
    }

    let empty = text.trim().is_empty();
    if empty {
        return format_footer(0, latency_ms, None);
    }

    // The displayed-item count comes structurally from the service arm (the
    // same number the journal records), never from re-parsing this text; an
    // error footer prices zero items.
    let (shown, saved) = price_footer(tool, is_error, shown, &text);

    let mut out = text.trim_end().to_string();
    out.push_str("\n───\n");
    out.push_str(&format_footer(shown, latency_ms, saved));
    out
}

/// Price the footer of one non-empty, non-memory response: the shown item
/// count, and the token saving from the shared `journal::estimated_tokens_saved`
/// estimator minus this response's own payload.
fn price_footer(
    tool: &str,
    is_error: bool,
    shown: Option<usize>,
    text: &str,
) -> (usize, Option<usize>) {
    if is_error {
        return (0, None);
    }
    let shown = shown.unwrap_or(0);
    // Chars-per-token estimate of this response's own payload: a rough
    // average for English prose and code identifiers, used only to price
    // the output the agent would have had to emit either way. Kept in one
    // constant so a recalibration cannot leave the footer pricing a
    // different output size than the `estimated_tokens_saved` table
    // assumes.
    const CHARS_PER_TOKEN: usize = 4;
    let output_tokens = text.len() / CHARS_PER_TOKEN;

    // One shared estimator with `eg usage` (journal::estimated_tokens_saved);
    // this footer prices the single call, so it subtracts the response payload
    // the agent would not have had to emit either way.
    let saved_tokens = if shown == 0 {
        0
    } else {
        crate::journal::estimated_tokens_saved(tool, shown, shown, 1).saturating_sub(output_tokens)
    };
    (shown, (saved_tokens > 0).then_some(saved_tokens))
}

/// The shared KPI footer line: `⚡ eg: {latency}ms | {N} items[ | ~{T} tokens
/// saved vs file reads]`. The `\n───\n` separator is pushed by the caller only
/// when there is text above the footer.
fn format_footer(shown: usize, latency_ms: f64, saved: Option<usize>) -> String {
    let saved_part = saved
        .map(|tokens| {
            format!(
                " | ~{} tokens saved vs file reads",
                format_token_count(tokens)
            )
        })
        .unwrap_or_default();
    format!("⚡ eg: {latency_ms:.1}ms | {shown} items{saved_part}\n")
}

fn format_token_count(tokens: usize) -> String {
    if tokens >= 1000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        tokens.to_string()
    }
}

/// Reply to a request; a notification (`id: None`) gets no response frame.
/// Protocol-level errors on unparsable frames reply with an explicit null
/// id via `Some(Value::Null)`.
pub(super) fn write_result(
    out: &mut impl Write,
    id: Option<Value>,
    result: Value,
) -> std::io::Result<()> {
    let Some(id) = id else {
        return Ok(());
    };
    let msg = json!({"jsonrpc": "2.0", "id": id, "result": result});
    writeln!(out, "{msg}")
}

pub(super) fn write_error(
    out: &mut impl Write,
    id: Option<Value>,
    code: i64,
    message: &str,
) -> std::io::Result<()> {
    let Some(id) = id else {
        return Ok(());
    };
    let msg = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message},
    });
    writeln!(out, "{msg}")
}
