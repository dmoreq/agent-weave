//! Target grammar for `eg reuse`: repo-relative path, directory prefix,
//! `file:line`, and unique symbol names. Validation happens before the corpus
//! loads so an invalid target cannot emit partial candidates.

use std::collections::HashSet;

use super::status::{ResolvedScope, ScopeKind};
use super::*;
use crate::query::{NameCandidate, QueryCtx, MAX_AMBIGUOUS_CANDIDATES};

pub(super) fn resolve_scope(ctx: &QueryCtx<'_>, inputs: &[String]) -> Result<ResolvedScope> {
    let mut paths = Vec::new();
    let mut symbols = HashSet::new();
    let mut matched_files = HashSet::new();

    for input in inputs {
        validate_input(input)?;
        if let Some((file, line)) = split_file_line(ctx, input) {
            resolve_file_line(ctx, input, &file, line, &mut symbols, &mut matched_files)?;
            continue;
        }
        let abs = ctx.root.join(input);
        match std::fs::symlink_metadata(&abs) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(unsafe_path(input));
            }
            Ok(meta) if meta.is_file() => resolve_file(ctx, input, &mut paths, &mut matched_files)?,
            Ok(meta) if meta.is_dir() => resolve_dir(ctx, input, &mut paths, &mut matched_files)?,
            Ok(_) => {
                return Err(EgError::Usage(format!(
                    "path must be a file or directory: {input}"
                )));
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                resolve_symbol(ctx, input, &mut symbols, &mut matched_files)?;
            }
            Err(err) => return Err(EgError::Internal(format!("stat {input}: {err}"))),
        }
    }

    if !paths.is_empty() && !symbols.is_empty() {
        return Err(EgError::Usage(
            "mixed path and symbol targets are not supported".to_string(),
        ));
    }
    let kind = if paths.is_empty() && symbols.is_empty() {
        ScopeKind::Repo
    } else if !paths.is_empty() {
        ScopeKind::Files(paths.clone())
    } else {
        let ids = symbols
            .iter()
            .filter_map(|qualname| {
                ctx.conn
                    .query_row(
                        "SELECT id FROM symbols WHERE qualname = ?1 LIMIT 1",
                        [qualname],
                        |row| row.get(0),
                    )
                    .ok()
            })
            .collect();
        ScopeKind::Symbols(ids)
    };
    let mut files: Vec<String> = matched_files.into_iter().collect();
    files.sort();
    let path_matches = files.len();
    Ok(ResolvedScope {
        kind,
        paths,
        symbols,
        files,
        path_matches,
    })
}

fn validate_input(input: &str) -> Result<()> {
    if input.is_empty() {
        return Err(EgError::Usage("path must be non-empty".to_string()));
    }
    if input.starts_with('/') || input.contains('\\') || input.split('/').any(|part| part == "..") {
        return Err(unsafe_path(input));
    }
    Ok(())
}

fn unsafe_path(input: &str) -> EgError {
    EgError::Usage(format!(
        "path must be repo-relative and contained: {}",
        super::sanitize(input)
    ))
}

/// Split `<file>:<positive line>`. The prefix must exist on disk so a normal
/// filename containing a colon is not misread as a line target.
fn split_file_line(ctx: &QueryCtx<'_>, input: &str) -> Option<(String, i64)> {
    let (file, line) = input.rsplit_once(':')?;
    if file.is_empty() || !line.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let line: i64 = line.parse().ok()?;
    if line <= 0 {
        return None;
    }
    std::fs::metadata(ctx.root.join(file))
        .ok()
        .filter(|m| m.is_file())?;
    Some((file.to_string(), line))
}

fn resolve_file_line(
    ctx: &QueryCtx<'_>,
    input: &str,
    file: &str,
    line: i64,
    symbols: &mut HashSet<String>,
    matched_files: &mut HashSet<String>,
) -> Result<()> {
    let indexed: i64 = ctx.conn.query_row(
        "SELECT COUNT(*) FROM files WHERE path = ?1",
        [file],
        |row| row.get(0),
    )?;
    if indexed == 0 {
        return Err(EgError::NoResults(format!(
            "path not found/not indexed: {}",
            super::sanitize(input)
        )));
    }
    let qualname: Option<String> = ctx
        .conn
        .query_row(
            "SELECT s.qualname FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE f.path = ?1 AND s.is_test = 0 AND s.kind IN ('function', 'method')
               AND s.line_start <= ?2 AND s.line_end >= ?2
             ORDER BY (s.line_end - s.line_start), s.qualname LIMIT 1",
            rusqlite::params![file, line],
            |row| row.get(0),
        )
        .ok();
    let Some(qualname) = qualname else {
        return Err(EgError::NoResults(format!(
            "no symbol at {}",
            super::sanitize(input)
        )));
    };
    symbols.insert(qualname);
    matched_files.insert(file.to_string());
    Ok(())
}

fn resolve_file(
    ctx: &QueryCtx<'_>,
    input: &str,
    paths: &mut Vec<String>,
    matched_files: &mut HashSet<String>,
) -> Result<()> {
    let indexed: i64 = ctx.conn.query_row(
        "SELECT COUNT(*) FROM files WHERE path = ?1",
        [input],
        |row| row.get(0),
    )?;
    if indexed == 0 {
        return Err(EgError::NoResults(format!(
            "path is not indexed: {}; run eg build",
            super::sanitize(input)
        )));
    }
    paths.push(input.to_string());
    matched_files.insert(input.to_string());
    Ok(())
}

fn resolve_dir(
    ctx: &QueryCtx<'_>,
    input: &str,
    paths: &mut Vec<String>,
    matched_files: &mut HashSet<String>,
) -> Result<()> {
    let prefix = format!("{}/%", escape_like(input));
    let mut stmt = ctx
        .conn
        .prepare("SELECT path FROM files WHERE path = ?1 OR path LIKE ?2 ESCAPE '\\'")?;
    let rows = stmt.query_map(rusqlite::params![input, prefix], |row| {
        row.get::<_, String>(0)
    })?;
    let mut found = 0usize;
    for row in rows {
        let path = row?;
        if path_matches(&path, &[input.to_string()]) {
            matched_files.insert(path);
            found += 1;
        }
    }
    if found == 0 {
        return Err(EgError::NoResults(format!(
            "no indexed files under path: {}",
            super::sanitize(input)
        )));
    }
    paths.push(input.to_string());
    Ok(())
}

fn resolve_symbol(
    ctx: &QueryCtx<'_>,
    input: &str,
    symbols: &mut HashSet<String>,
    matched_files: &mut HashSet<String>,
) -> Result<()> {
    let escaped = escape_like(input);
    let module_suffix = format!("%::{escaped}");
    let dot_suffix = format!("%.{escaped}");
    let mut stmt = ctx.conn.prepare(
        "SELECT file_path, line_start, qualname FROM symbols
         WHERE qualname = ?1 OR name = ?1
            OR qualname LIKE ?2 ESCAPE '\\' OR qualname LIKE ?3 ESCAPE '\\'
         ORDER BY file_path, line_start, qualname",
    )?;
    let rows = stmt.query_map(rusqlite::params![input, module_suffix, dot_suffix], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let all: Vec<NameCandidate> = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let total = all.len();
    match total {
        0 => {
            let detail = if input.contains('/') || input.contains('.') {
                format!("path not found: {}", super::sanitize(input))
            } else {
                format!("symbol not in index: {}", super::sanitize(input))
            };
            Err(EgError::NoResults(detail))
        }
        1 => {
            let (file, _, qualname) = all.into_iter().next().unwrap();
            matched_files.insert(file);
            symbols.insert(qualname);
            Ok(())
        }
        _ => {
            let candidates = all
                .into_iter()
                .take(MAX_AMBIGUOUS_CANDIDATES)
                .map(|(path, line, qualname)| {
                    (super::sanitize(&path), line, super::sanitize(&qualname))
                })
                .collect();
            Err(EgError::Ambiguous {
                name: super::sanitize(input),
                candidates,
                total,
                role: None,
            })
        }
    }
}

/// Escape LIKE metacharacters so a target containing `%`, `_`, or `\` is
/// matched literally.
fn escape_like(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// A path is in scope when it equals a target file or sits below a target
/// directory. The boundary check keeps `src/query` from matching `src/query2`.
pub(super) fn path_matches(path: &str, targets: &[String]) -> bool {
    targets.iter().any(|target| {
        path == target
            || path
                .strip_prefix(target)
                .is_some_and(|rest| rest.starts_with('/'))
    })
}
