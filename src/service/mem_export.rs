//! `mem export`: JSONL/Markdown rendering, time windows, and the advisory
//! suspected-secret scan. Split from `mem_ops.rs` to keep both files under
//! the 500-line hygiene cap.

use super::mem_ops::{open_mem_conn, MemoryMode};
use super::*;

/// Export wire format selected by `--format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportFormat {
    Jsonl,
    Markdown,
}

/// Validate the `--format` value at the shared service boundary, so CLI and
/// MCP reject an unknown format identically (exit 2).
fn parse_export_format(raw: Option<&str>) -> Result<ExportFormat> {
    match raw.unwrap_or("jsonl") {
        "jsonl" => Ok(ExportFormat::Jsonl),
        "markdown" => Ok(ExportFormat::Markdown),
        other => Err(EgError::Usage(format!(
            "mem export: --format must be 'jsonl' or 'markdown' (got '{other}')"
        ))),
    }
}

/// Current notes as JSONL or fenced Markdown on stdout; the caveat goes to
/// stderr. Both formats read the same expiry-filtered current-row query, so
/// an expired session note is never exported by either.
pub(super) fn run_mem_export(req: &CommandRequest, root_arg: Option<&Path>) -> Result<ArmOut> {
    let CommandRequest::MemExport {
        format,
        limit,
        global,
        since,
        until,
        scan,
    } = req
    else {
        unreachable!("run_mem_export called with a non-export request")
    };
    let window = crate::memory::TimeWindow {
        since: *since,
        until: *until,
    };
    crate::memory::validate_window(&window, "mem export")?;
    let format = parse_export_format(format.as_deref())?;
    if matches!(limit, Some(0)) {
        return Err(EgError::Usage(
            "mem export: --limit must be at least 1".to_string(),
        ));
    }
    let conn = open_mem_conn(root_arg, *global, MemoryMode::Query)?;
    let rows = crate::db::memory_current_rows(
        &conn,
        &crate::db::MemFilter {
            kind: None,
            scope: None,
            now: crate::db::unix_now(),
            include_superseded: false,
            window,
        },
        *limit,
    )?;
    if rows.is_empty() {
        return Err(EgError::NoResults(
            "memory store has no current notes to export".to_string(),
        ));
    }
    crate::journal::record_result(crate::journal::OutcomeCounts::uniform(rows.len()), false, 0);
    // stdout carries ONLY the payload; the caveat goes to stderr so restore
    // pipelines parse stdout directly (stream contract).
    let mut stdout = String::new();
    for row in &rows {
        match format {
            ExportFormat::Jsonl => stdout.push_str(&render_jsonl_row(row)),
            ExportFormat::Markdown => stdout.push_str(&render_markdown_document(row)),
        }
    }
    let mut stderr = format!("{}\n", crate::memory::MEMORY_CAVEAT);
    if *scan {
        let mut lines = Vec::new();
        let mut notes = 0usize;
        for row in &rows {
            let mut fields = Vec::new();
            if crate::memory::secret::suspected_secret(&row.key).is_some() {
                fields.push("key");
            }
            if crate::memory::secret::suspected_secret(&row.body).is_some() {
                fields.push("body");
            }
            if crate::memory::secret::suspected_secret(&row.tags).is_some() {
                fields.push("tags");
            }
            if crate::memory::secret::suspected_secret(&row.refs).is_some() {
                fields.push("refs");
            }
            if fields.is_empty() {
                continue;
            }
            notes += 1;
            let label = if fields.contains(&"key") {
                format!("row {}", row.id)
            } else {
                row.key.clone()
            };
            lines.push(format!("  {label}: {}", fields.join(", ")));
        }
        if notes == 1 {
            stderr.push_str("eg: scan: 1 note contains suspected secrets\n");
        } else if notes > 1 {
            stderr.push_str(&format!(
                "eg: scan: {notes} notes contain suspected secrets\n"
            ));
        }
        for line in lines {
            stderr.push_str(&line);
            stderr.push('\n');
        }
    }
    Ok(ArmOut::plain(stdout, stderr))
}

/// One JSONL export line (the shipped restore format).
fn render_jsonl_row(row: &crate::db::MemRow) -> String {
    let line = serde_json::json!({
        "key": row.key,
        "rev": row.rev,
        "kind": row.kind,
        "scope": row.scope,
        "session": row.session,
        "body": row.body,
        "tags": row.tags,
        "refs": row.refs,
        "source": row.source,
        "created_at": row.created_at,
        "expires_at": row.expires_at,
    });
    format!("{line}\n")
}

/// One fenced Markdown document: a delimiter line, one-line JSON
/// front-matter, the verbatim body, and the matching closer. The slug is
/// display-only; import matches the closer by the opener's slug.
fn render_markdown_document(row: &crate::db::MemRow) -> String {
    let slug = note_slug(&row.key);
    let tags: Vec<&str> = row.tags.split_whitespace().collect();
    let refs: Vec<&str> = row.refs.split_whitespace().collect();
    let front = serde_json::json!({
        "format": crate::memory::IMPORT_FORMAT,
        "key": row.key,
        "kind": row.kind,
        "scope": row.scope,
        "tags": tags,
        "refs": refs,
        "expires_at": row.expires_at,
    });
    format!(
        "@@ engram-note {slug} @@\n{front}\n{}\n@@ end engram-note {slug} @@\n",
        row.body
    )
}

/// Display slug for a note key: lowercase ASCII alphanumeric runs joined by
/// `-`, with an 8-hex key hash so two keys cannot share a document marker.
fn note_slug(key: &str) -> String {
    let mut slug = String::new();
    for c in key.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let prefix = slug.trim_end_matches('-');
    let prefix = if prefix.is_empty() { "note" } else { prefix };
    let hash = crate::hash::fnv1a64(key.as_bytes());
    format!("{prefix}-{hash:016x}")
}
