//! Memory operations: get, delete, export, garbage-collection entry, and
//! connection openers for both stores.

use super::*;

/// Current row for one exact key plus its revision count.
pub(super) fn run_mem_get(req: &CommandRequest, root_arg: Option<&Path>) -> Result<ArmOut> {
    let CommandRequest::MemGet { key, global } = req else {
        unreachable!("run_mem_get called with a non-get request")
    };
    let index_conn = stale_ref_conn(root_arg);

    if *global {
        let conn = crate::db::open_global_memory_query()?;
        return match crate::db::memory_get(&conn, key)? {
            None => Err(mem_unknown_key(key)),
            Some((row, revs)) => {
                let status = check_note_ref_status(index_conn.as_ref(), &row);
                Ok(render_mem_hit(
                    &row,
                    &store_tags(true, status, &row.source),
                    revs,
                ))
            }
        };
    }

    let stores = resolve_memory_stores(root_arg)?;
    for (is_global, conn) in [
        (false, stores.local.as_ref()),
        (true, stores.global.as_ref()),
    ] {
        let Some(conn) = conn else { continue };
        if let Some((row, revs)) = crate::db::memory_get(conn, key)? {
            let status = check_note_ref_status(index_conn.as_ref(), &row);
            return Ok(render_mem_hit(
                &row,
                &store_tags(is_global, status, &row.source),
                revs,
            ));
        }
    }

    if stores.local.is_none() && stores.global.is_none() {
        return Err(no_memory_store_error(root_arg));
    }
    Err(mem_unknown_key(key))
}

/// Hard-delete every revision of one key.
pub(super) fn run_mem_delete(req: &CommandRequest, root_arg: Option<&Path>) -> Result<ArmOut> {
    let CommandRequest::MemDelete { key, global } = req else {
        unreachable!("run_mem_delete called with a non-delete request")
    };
    let mut conn = open_mem_conn(root_arg, *global, MemoryMode::Mutation)?;
    match crate::db::memory_delete(&mut conn, key)? {
        None => Err(mem_unknown_key(key)),
        Some(revs) => {
            crate::journal::record_result(
                crate::journal::OutcomeCounts::uniform(revs.max(0) as usize),
                false,
                0,
            );
            Ok(ArmOut::plain(
                format!(
                    "{}\ndeleted '{key}' revs={revs}\n",
                    crate::memory::MEMORY_CAVEAT
                ),
                String::new(),
            ))
        }
    }
}

/// Refresh created_at timestamp on the active revision without revision churn.
pub(super) fn run_mem_touch(req: &CommandRequest, root_arg: Option<&Path>) -> Result<ArmOut> {
    let CommandRequest::MemTouch { key, global } = req else {
        unreachable!("run_mem_touch called with a non-touch request")
    };
    let now = crate::db::unix_now();
    let mut conn = open_mem_conn(root_arg, *global, MemoryMode::Mutation)?;
    match crate::db::memory_touch(&mut conn, key, now)? {
        None => Err(mem_unknown_key(key)),
        Some(rev) => {
            crate::journal::record_result(crate::journal::OutcomeCounts::uniform(1), false, 0);
            Ok(ArmOut::plain(
                format!(
                    "{}\nok: '{key}' rev {rev} touched\n",
                    crate::memory::MEMORY_CAVEAT
                ),
                String::new(),
            ))
        }
    }
}

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
    } = req
    else {
        unreachable!("run_mem_export called with a non-export request")
    };
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
    Ok(ArmOut::plain(
        stdout,
        format!("{}\n", crate::memory::MEMORY_CAVEAT),
    ))
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

/// Best-effort index connection for stale-ref verification: resolved from
/// the explicit root, else the discovered one; absence degrades reads to
/// unverified (never stale) refs. Shared by the mem query and get paths.
pub(super) fn stale_ref_conn(root_arg: Option<&Path>) -> Option<rusqlite::Connection> {
    root_arg
        .and_then(|r| crate::walk::discover_root(Some(r)).ok())
        .or_else(|| crate::walk::discover_root(None).ok())
        .and_then(|root| crate::db::open_for_query(&root).ok())
}

/// Caveated empty-result error shared by the mem read commands (stdout keeps
/// the caveat line, stderr the diagnostic, exit 1).
pub(super) fn mem_empty(detail: &str) -> EgError {
    EgError::NoResultsCaveated {
        caveat: crate::memory::MEMORY_CAVEAT,
        detail: detail.to_string(),
    }
}

/// The mem_empty error for a key with no current row: shared by mem get and
/// mem delete so the message cannot drift.
fn mem_unknown_key(key: &str) -> EgError {
    mem_empty(&format!("unknown memory key '{key}'"))
}

/// The resolver: FTS candidates capped at CANDIDATE_CAP, ref/tag token
/// overlap, recency decay, fixed-weight fusion (D4-A). The fetch stays here;
/// the filter/signal work lives in `mem_candidate`, so this function returns
/// candidates with `is_global` marked. Ranked mode never lists superseded rows
/// (`validate_mem_query` pins `include_superseded` to `--all`), so the filter
/// rides through unchanged.
pub(super) fn mem_ranked(
    conn: &rusqlite::Connection,
    joined_terms: &str,
    filter: &crate::db::MemFilter,
    is_global: bool,
    index_conn: Option<&rusqlite::Connection>,
) -> Result<Vec<crate::memory::Candidate>> {
    let query = crate::memory::mem_query_terms(joined_terms);
    let hits = crate::db::memory_fts_candidates(conn, &query, crate::memory::CANDIDATE_CAP)?;
    let query_tokens_vec = crate::memory::mem_tokens(joined_terms);
    let query_tokens: std::collections::BTreeSet<String> =
        query_tokens_vec.iter().cloned().collect();

    let mut hit_map: std::collections::HashMap<i64, f64> = hits.into_iter().collect();
    let ref_rows = if !query_tokens_vec.is_empty() {
        crate::db::memory_ref_candidate_rows(conn, &query_tokens_vec, crate::memory::CANDIDATE_CAP)?
    } else {
        Vec::new()
    };

    let mut by_id: std::collections::HashMap<i64, crate::db::MemRow> =
        std::collections::HashMap::new();
    for row in ref_rows {
        hit_map.entry(row.id).or_insert(0.0);
        by_id.insert(row.id, row);
    }

    if hit_map.is_empty() && index_conn.is_none() {
        return Ok(Vec::new());
    }

    let missing_ids: Vec<i64> = hit_map
        .keys()
        .copied()
        .filter(|id| !by_id.contains_key(id))
        .collect();
    if !missing_ids.is_empty() {
        for row in crate::db::memory_rows_by_ids(conn, &missing_ids)? {
            by_id.insert(row.id, row);
        }
    }

    let mut candidates: Vec<crate::memory::Candidate> = hit_map
        .into_iter()
        .filter_map(|(id, lex_raw)| {
            mem_candidate(by_id.get(&id)?, lex_raw, &query_tokens, filter, is_global)
        })
        .collect();

    // Graph expansion is best-effort: any index error already degraded to an
    // empty expansion inside `graph_expand`, so the lexical ranking survives.
    if let Some(index) = index_conn {
        let known_ids: std::collections::HashSet<i64> =
            candidates.iter().map(|c| c.row.id).collect();
        let expansion = crate::memory::graph_expand(
            index,
            conn,
            &query_tokens,
            filter,
            &known_ids,
            crate::memory::CANDIDATE_CAP,
        );
        for candidate in &mut candidates {
            candidate.graph_raw = expansion
                .scores
                .get(&candidate.row.id)
                .copied()
                .unwrap_or(0.0);
        }
        for (row, decay) in expansion.extra {
            let Some(mut candidate) = mem_candidate(
                &row,
                crate::memory::GRAPH_LEX_RAW,
                &query_tokens,
                filter,
                is_global,
            ) else {
                continue;
            };
            candidate.graph_raw = decay;
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

/// One ranked candidate: the current-row filter (same rules as the `--all`
/// SQL filter via `db::memory_row_is_current`) plus the ref/tag overlap and
/// recency signals. `None` when the row is not current or its FTS entry
/// outlived the row.
fn mem_candidate(
    row: &crate::db::MemRow,
    lex_raw: f64,
    query_tokens: &std::collections::BTreeSet<String>,
    filter: &crate::db::MemFilter,
    is_global: bool,
) -> Option<crate::memory::Candidate> {
    // Defense in depth: `memory_write` removes the FTS row on supersession,
    // so a superseded row here only comes from a store written by an older
    // build.
    if !crate::db::memory_row_is_current(row, filter) {
        return None;
    }
    let set = crate::memory::mem_ref_tag_tokens(&row.refs, &row.tags);
    let ref_raw = if set.is_empty() {
        0.0
    } else {
        set.intersection(query_tokens).count() as f64 / set.len() as f64
    };
    let age_days = (filter.now - row.created_at).max(0) as f64 / 86400.0;
    Some(crate::memory::Candidate {
        row: row.clone(),
        lex_raw,
        ref_raw,
        graph_raw: 0.0,
        age_days,
        score: 0.0,
        is_global,
    })
}

/// Resolve the repo root and open the index read-only (the shared query gate:
/// missing -> 1, corrupt -> 3, skew -> 5).
pub(super) fn open_query(
    root_arg: Option<&Path>,
) -> Result<(std::path::PathBuf, rusqlite::Connection)> {
    let root = crate::walk::discover_root(root_arg)?;
    let conn = crate::db::open_for_query(&root)?;
    Ok((root, conn))
}

/// The three memory-store open gates, in the shared `open_query` shape.
pub(super) enum MemoryMode {
    /// Read-only (missing -> 1, corrupt -> 3, skew -> 5).
    Query,
    /// Read-write mutation without creating a store (missing -> 1).
    Mutation,
    /// Read-write with first-use creation.
    Store,
}

/// Resolve the repo root and open the memory store in `mode` for the mem
/// commands (the same root gate as `open_query`).
fn open_memory(root_arg: Option<&Path>, mode: MemoryMode) -> Result<rusqlite::Connection> {
    let root = crate::walk::discover_root(root_arg)?;
    match mode {
        MemoryMode::Query => crate::db::open_memory_query(&root),
        MemoryMode::Mutation => crate::db::open_memory_mutation(&root),
        MemoryMode::Store => crate::db::open_memory_store(&root),
    }
}

/// The single mem-command connection gate: `global` routes to the home-dir
/// store, otherwise the project store under `root_arg`, both in `mode`.
pub(super) fn open_mem_conn(
    root_arg: Option<&Path>,
    global: bool,
    mode: MemoryMode,
) -> Result<rusqlite::Connection> {
    if global {
        match mode {
            MemoryMode::Query => crate::db::open_global_memory_query(),
            MemoryMode::Mutation => crate::db::open_global_memory_mutation(),
            MemoryMode::Store => crate::db::open_global_memory_store(),
        }
    } else {
        open_memory(root_arg, mode)
    }
}

// ---------------------------------------------------------------------------
// MCP tool registry: the single declarative table both the JSON Schema
// renderer and the required-argument validator read.
// ---------------------------------------------------------------------------
