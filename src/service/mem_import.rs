//! Markdown/JSONL import: stream parsing, format autodetection, and the
//! transactional batch write. The parser is pure text handling; the store is
//! opened only after every note validates, so a rejected import writes nothing.

use super::*;
use std::io::Read;

/// One parsed note: validated front-matter plus its verbatim body.
type ParsedNote = (crate::memory::NoteFrontmatter, String);

/// Import one file (or stdin) into the memory store. `force` is required to
/// overwrite a key whose local content differs; identical content is a no-op.
pub(super) fn run_mem_import(
    req: &CommandRequest,
    root_arg: Option<&Path>,
    surface: Surface,
) -> Result<ArmOut> {
    let CommandRequest::MemImport {
        path,
        format,
        force,
        global,
    } = req
    else {
        unreachable!("run_mem_import called with a non-import request")
    };
    let text = read_input(path, root_arg, surface)?;
    let format = detect_format(format.as_deref(), path, &text)?;
    let parsed = match format {
        ImportFormat::Markdown => parse_markdown(&text)?,
        ImportFormat::Jsonl => parse_jsonl(&text)?,
    };
    if parsed.len() > crate::memory::MAX_IMPORT_NOTES {
        return Err(EgError::Usage(format!(
            "mem import: {} notes exceeds the {}-note cap",
            parsed.len(),
            crate::memory::MAX_IMPORT_NOTES
        )));
    }
    reject_duplicate_keys(&parsed)?;

    let now = crate::db::unix_now();
    let mut normalized = Vec::with_capacity(parsed.len());
    let mut expired = 0usize;
    for (fm, body) in &parsed {
        let note = crate::memory::normalize_import_note(fm, body, now)?;
        if note.expired {
            expired += 1;
            continue;
        }
        normalized.push(note);
    }
    if normalized.is_empty() && expired == 0 {
        return Err(EgError::NoResults(
            "mem import: no importable notes".to_string(),
        ));
    }

    // Open/create the store only after every note passed validation.
    let mut conn = open_mem_conn(root_arg, *global, MemoryMode::Store)?;
    let session = crate::journal::session_tag();
    let inputs: Vec<crate::db::MemNoteInput<'_>> = normalized
        .iter()
        .map(|note| crate::db::MemNoteInput {
            key: &note.key,
            kind: note.kind.as_str(),
            tags: &note.tags,
            refs: &note.refs,
            scope: note.scope.as_str(),
            session: &session,
            body: &note.body,
            source: "import",
            expires_at: note.expires_at,
            supersedes: None,
            touch: false,
        })
        .collect();
    let mut stats = crate::db::memory_write_batch(&mut conn, &inputs, now, *force)?;
    stats.expired += expired;

    if stats.imported == 0 && stats.conflicts > 0 {
        return Err(EgError::Usage(format!(
            "mem import: {} conflict(s) skipped; re-run with --force to overwrite",
            stats.conflicts
        )));
    }
    crate::journal::record_result(
        crate::journal::OutcomeCounts::uniform(stats.imported),
        false,
        0,
    );
    let mut stderr = String::new();
    if stats.conflicts > 0 {
        stderr.push_str(&format!(
            "eg: import: {} conflict(s) skipped; re-run with --force to overwrite\n",
            stats.conflicts
        ));
    }
    Ok(ArmOut::plain(
        format!(
            "{}\nok: imported {}, skipped {}, conflicts {}, expired {}\n",
            crate::memory::MEMORY_CAVEAT,
            stats.imported,
            stats.skipped,
            stats.conflicts,
            stats.expired
        ),
        stderr,
    ))
}

/// Import wire format. `--format` wins; otherwise `.jsonl`/`.md` extension or
/// the first non-empty line decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportFormat {
    Jsonl,
    Markdown,
}

fn detect_format(raw: Option<&str>, path: &str, text: &str) -> Result<ImportFormat> {
    if let Some(raw) = raw {
        return match raw {
            "jsonl" => Ok(ImportFormat::Jsonl),
            "markdown" => Ok(ImportFormat::Markdown),
            other => Err(EgError::Usage(format!(
                "mem import: --format must be 'jsonl' or 'markdown' (got '{other}')"
            ))),
        };
    }
    if path != "-" {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".jsonl") {
            return Ok(ImportFormat::Jsonl);
        }
        if lower.ends_with(".md") || lower.ends_with(".markdown") {
            return Ok(ImportFormat::Markdown);
        }
    }
    let first = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if first.starts_with("@@ engram-note") {
        Ok(ImportFormat::Markdown)
    } else if first.starts_with('{') {
        Ok(ImportFormat::Jsonl)
    } else {
        Err(EgError::Usage(
            "mem import: cannot detect format; pass --format jsonl|markdown".to_string(),
        ))
    }
}

/// Resolve the import path: CLI paths are used as the shell passed them; MCP
/// paths must be repo-relative and are joined to the active root.
fn resolve_import_path(
    path: &str,
    root_arg: Option<&Path>,
    surface: Surface,
) -> Result<std::path::PathBuf> {
    let candidate = std::path::Path::new(path);
    if surface == Surface::Mcp {
        if candidate.is_absolute() {
            return Err(EgError::Usage(format!(
                "mem import: '{path}' must be repo-relative over MCP"
            )));
        }
        if let Some(root) = root_arg {
            return Ok(root.join(candidate));
        }
    }
    Ok(candidate.to_path_buf())
}

/// Read the import source: one regular file, or stdin (`-`). A symlink or a
/// directory is rejected; the size cap runs before the read.
fn read_input(path: &str, root_arg: Option<&Path>, surface: Surface) -> Result<String> {
    if path == "-" {
        let mut buf = String::new();
        // Bound the read itself: one byte past the cap is enough to reject,
        // so unbounded stdin cannot exhaust memory.
        std::io::stdin()
            .take(crate::memory::MAX_IMPORT_FILE_BYTES as u64 + 1)
            .read_to_string(&mut buf)
            .map_err(|e| EgError::Internal(format!("io error: {e}")))?;
        if buf.len() > crate::memory::MAX_IMPORT_FILE_BYTES {
            return Err(EgError::Usage(format!(
                "mem import: stdin exceeds {} bytes",
                crate::memory::MAX_IMPORT_FILE_BYTES
            )));
        }
        return Ok(buf);
    }
    let resolved = resolve_import_path(path, root_arg, surface)?;
    let meta = std::fs::symlink_metadata(&resolved).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            EgError::NoResults(format!("mem import: '{path}' not found"))
        }
        _ => EgError::Internal(format!("io error: {e}")),
    })?;
    if meta.file_type().is_symlink() {
        return Err(EgError::Usage(format!(
            "mem import: '{path}' is a symlink; import a regular file"
        )));
    }
    if meta.is_dir() {
        return Err(EgError::Usage(format!(
            "mem import: '{path}' is a directory; import one file or '-'"
        )));
    }
    if meta.len() > crate::memory::MAX_IMPORT_FILE_BYTES as u64 {
        return Err(EgError::Usage(format!(
            "mem import: '{path}' exceeds {} bytes",
            crate::memory::MAX_IMPORT_FILE_BYTES
        )));
    }
    std::fs::read_to_string(&resolved).map_err(|e| EgError::Internal(format!("io error: {e}")))
}

/// Fenced Markdown parser: `@@ engram-note <slug> @@`, one JSON front-matter
/// line, the body, then `@@ end engram-note <slug> @@`. CRLF is normalized.
fn parse_markdown(text: &str) -> Result<Vec<ParsedNote>> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        if line.is_empty() {
            i += 1;
            continue;
        }
        let Some(slug) = line
            .strip_prefix("@@ engram-note ")
            .and_then(|s| s.strip_suffix(" @@"))
        else {
            return Err(EgError::Usage(format!(
                "mem import: expected '@@ engram-note <slug> @@', got '{line}'"
            )));
        };
        let slug = slug.trim();
        i += 1;
        let Some(matter) = lines.get(i) else {
            return Err(EgError::Usage(format!(
                "mem import: missing front-matter after '{slug}'"
            )));
        };
        let value: serde_json::Value = serde_json::from_str(matter.trim())
            .map_err(|e| EgError::Usage(format!("mem import: invalid front-matter JSON: {e}")))?;
        let frontmatter = crate::memory::NoteFrontmatter::from_json(&value)?;
        i += 1;
        let closer = format!("@@ end engram-note {slug} @@");
        let mut body_lines = Vec::new();
        loop {
            let Some(&body_line) = lines.get(i) else {
                return Err(EgError::Usage(format!(
                    "mem import: missing closer '@@ end engram-note {slug} @@'"
                )));
            };
            if body_line.trim() == closer {
                i += 1;
                break;
            }
            body_lines.push(body_line);
            i += 1;
        }
        out.push((frontmatter, body_lines.join("\n")));
    }
    if out.is_empty() {
        return Err(EgError::Usage("mem import: no notes found".to_string()));
    }
    Ok(out)
}

/// JSONL parser: one exported note object per line. The shipped export omits
/// `format`, so it is injected before the shared front-matter parser runs.
fn parse_jsonl(text: &str) -> Result<Vec<ParsedNote>> {
    let mut out = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(raw).map_err(|e| {
            EgError::Usage(format!("mem import: line {}: invalid JSON: {e}", index + 1))
        })?;
        let body = value
            .get("body")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                EgError::Usage(format!("mem import: line {}: missing 'body'", index + 1))
            })?
            .to_string();
        let mut obj = value;
        if obj.get("format").is_none() {
            if let Some(map) = obj.as_object_mut() {
                map.insert(
                    "format".to_string(),
                    serde_json::Value::String(crate::memory::IMPORT_FORMAT.to_string()),
                );
            }
        }
        let frontmatter = crate::memory::NoteFrontmatter::from_json(&obj)?;
        out.push((frontmatter, body));
    }
    if out.is_empty() {
        return Err(EgError::Usage("mem import: no notes found".to_string()));
    }
    Ok(out)
}

/// A key may appear at most once per import; a second appearance is ambiguous
/// (two revisions in one batch) and is rejected before any write.
fn reject_duplicate_keys(parsed: &[ParsedNote]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for (frontmatter, _) in parsed {
        let key = frontmatter.key.trim();
        if !seen.insert(key.to_string()) {
            return Err(EgError::Usage(format!(
                "mem import: duplicate key '{key}' in one import"
            )));
        }
    }
    Ok(())
}
