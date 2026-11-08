//! One-call symbol context: definition row plus direct caller and callee pages.

use super::frame::{direct_callees, direct_callers, CallEdgeRow};
use super::*;

/// One-call symbol context (Amendment 17): definition line with signature,
/// direct callers, and direct callees — replaces composing callers/callees/
/// symbols for "what does this symbol do and who uses it" questions.
pub fn context(ctx: &QueryCtx<'_>, name: &str, limit: usize) -> Result<LineQuery> {
    let QueryCtx { conn, .. } = *ctx;
    // The CLI documents max 50 per section; clamp here so MCP callers cannot
    // bypass the contract.
    let limit = clamp_limit(limit)?.min(50);
    let (ids, _) = resolve_targets(conn, name, None)?;
    let data = fetch_context(conn, ids[0], limit)?;
    render_context(ctx, &data)
}

/// Definition row plus one page of direct callers and callees.
struct ContextData {
    def_path: String,
    def_line: i64,
    def_end: i64,
    kind: String,
    qualname: String,
    signature: String,
    callers: Vec<CallEdgeRow>,
    callers_total: usize,
    callees: Vec<CallEdgeRow>,
    callees_total: usize,
}

fn fetch_context(conn: &Connection, id: i64, limit: usize) -> Result<ContextData> {
    let def_sql = "SELECT f.path, s.line_start, s.line_end, s.kind, s.qualname, s.signature
                   FROM symbols s JOIN files f ON f.id = s.file_id WHERE s.id = ?1";
    let (def_path, def_line, def_end, kind, qualname, signature): (
        String,
        i64,
        i64,
        String,
        String,
        String,
    ) = conn.query_row(def_sql, [id], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
        ))
    })?;
    let (callers, callers_total) = direct_callers(conn, &[id], limit)?;
    let (callees, callees_total) = direct_callees(conn, &[id], limit)?;
    Ok(ContextData {
        def_path,
        def_line,
        def_end,
        kind,
        qualname,
        signature,
        callers,
        callers_total,
        callees,
        callees_total,
    })
}

fn render_context(ctx: &QueryCtx<'_>, data: &ContextData) -> Result<LineQuery> {
    // Staleness over the displayed set (definition + shown callers/callees).
    let files = context_file_set(data);
    let (header, stale) = staleness_header(ctx, &files);
    // Caller/callee lines carry the identity suffix added in R7.
    let (caller_lines, callee_lines) = context_rows(data);
    let total = data.callers_total + data.callees_total;
    let shown = caller_lines.len() + callee_lines.len();
    crate::journal::record_result(crate::journal::OutcomeCounts::of(shown, total), stale, 0);
    let mut out = vec![CONTEXT_CAVEAT.to_string()];
    if let Some(header) = header {
        out.push(header);
    }
    let sig = if data.signature.is_empty() {
        String::new()
    } else {
        format!(" {}", data.signature)
    };
    out.push(format!(
        "{}:{}-{} {} {}{sig}",
        data.def_path, data.def_line, data.def_end, data.kind, data.qualname
    ));
    out.push(format!(
        "callers (showing {} of {}):",
        data.callers.len(),
        data.callers_total
    ));
    out.extend(caller_lines);
    out.push(format!(
        "callees (showing {} of {}):",
        data.callees.len(),
        data.callees_total
    ));
    out.extend(callee_lines);
    Ok(LineQuery {
        lines: out,
        stderr: String::new(),
        shown,
    })
}

fn context_file_set(data: &ContextData) -> BTreeSet<String> {
    let mut files = BTreeSet::new();
    files.insert(data.def_path.clone());
    files.extend(data.callers.iter().map(|row| row.path.clone()));
    files.extend(data.callees.iter().map(|row| row.path.clone()));
    files
}

fn context_rows(data: &ContextData) -> (Vec<String>, Vec<String>) {
    let caller_lines = data
        .callers
        .iter()
        .map(|row| format!("{}:{} {}", row.path, row.site_line, row.qualname))
        .collect();
    let callee_lines = data
        .callees
        .iter()
        .map(|row| {
            Pointer {
                file: row.path.clone(),
                line: row.site_line,
                tag: confidence_tag(&row.confidence),
                detail: row.qualname.clone(),
            }
            .render()
        })
        .collect();
    (caller_lines, callee_lines)
}
