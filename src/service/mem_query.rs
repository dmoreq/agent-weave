//! Memory queries: ranked FTS search, `--all` listings, multi-store merge,
//! stale-ref checks, and row rendering.

use super::*;

/// Ranked search or the explicit `--all` listing.
pub(super) fn run_mem_query(req: &CommandRequest, root_arg: Option<&Path>) -> Result<ArmOut> {
    let CommandRequest::MemQuery {
        terms,
        all,
        kind,
        scope,
        limit,
        include_superseded,
        global,
        reference,
    } = req
    else {
        unreachable!("run_mem_query called with a non-query request")
    };
    let clean_terms: Vec<String> = terms.iter().filter(|t| !t.is_empty()).cloned().collect();
    let effective_all = *all || clean_terms.is_empty();
    validate_mem_query(terms, *all, effective_all, *include_superseded)?;
    let index_conn = stale_ref_conn(root_arg);
    let args = MemQueryArgs {
        terms: &clean_terms,
        all: effective_all,
        limit: (*limit).clamp(1, crate::memory::MAX_MEM_LIMIT),
        filter: crate::db::MemFilter {
            kind: *kind,
            scope: *scope,
            now: crate::db::unix_now(),
            include_superseded: *include_superseded,
        },
        index_conn: index_conn.as_ref(),
        reference: reference.as_deref(),
    };

    if *global {
        let conn = crate::db::open_global_memory_query()?;
        return query_single_store(&conn, &args, true);
    }

    let stores = resolve_memory_stores(root_arg)?;
    match (stores.local, stores.global) {
        (None, None) => Err(no_memory_store_error(root_arg)),
        (Some(local), None) => query_single_store(&local, &args, false),
        (None, Some(global)) => query_single_store(&global, &args, true),
        (Some(local), Some(global)) => query_merged_stores(&local, &global, &args),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefStatus {
    Valid,
    Modified,
    Broken,
}

pub(crate) fn check_ref_status(
    index_conn: &rusqlite::Connection,
    r: &str,
    created_at: i64,
) -> RefStatus {
    let file_check: rusqlite::Result<(String, i64)> = index_conn.query_row(
        "SELECT path, mtime FROM files WHERE path = ?1 OR path LIKE ?2 LIMIT 1",
        rusqlite::params![r, format!("%/{r}")],
        |row| Ok((row.get(0)?, row.get(1)?)),
    );
    if let Ok((_path, mtime_nanos)) = file_check {
        return if (mtime_nanos / 1_000_000_000) > created_at {
            RefStatus::Modified
        } else {
            RefStatus::Valid
        };
    }

    let sym_check: rusqlite::Result<i64> = index_conn.query_row(
        "SELECT f.mtime FROM symbols s JOIN files f ON s.file_id = f.id
         WHERE s.qualname = ?1 OR s.name = ?1 LIMIT 1",
        rusqlite::params![r],
        |row| row.get(0),
    );
    if let Ok(mtime_nanos) = sym_check {
        return if (mtime_nanos / 1_000_000_000) > created_at {
            RefStatus::Modified
        } else {
            RefStatus::Valid
        };
    }

    RefStatus::Broken
}

pub(super) fn check_note_ref_status(
    index_conn: Option<&rusqlite::Connection>,
    row: &crate::db::MemRow,
) -> RefStatus {
    let Some(conn) = index_conn else {
        return RefStatus::Valid;
    };
    if row.refs.trim().is_empty() {
        return RefStatus::Valid;
    }
    let mut has_modified = false;
    for r in row.refs.split_whitespace() {
        match check_ref_status(conn, r, row.created_at) {
            RefStatus::Broken => return RefStatus::Broken,
            RefStatus::Modified => has_modified = true,
            RefStatus::Valid => {}
        }
    }
    if has_modified {
        RefStatus::Modified
    } else {
        RefStatus::Valid
    }
}

/// The `[global] [stale-ref]` store markers shared by `mem get` output and
/// the listing tag builder, so the suffix order cannot drift.
pub(super) fn store_tags(is_global: bool, status: RefStatus, source: &str) -> String {
    let mut tag = String::new();
    if source == "import" {
        tag.push_str(" [imported]");
    }
    if is_global {
        tag.push_str(" [global]");
    }
    match status {
        RefStatus::Broken => tag.push_str(" [stale-ref] [broken-ref]"),
        RefStatus::Modified => tag.push_str(" [stale-ref] [modified-ref]"),
        RefStatus::Valid => {}
    }
    tag
}

/// The `[superseded]`/`[global]`/`[stale-ref]` listing tag for one row, shared
/// by the single-store and merged listings so the markers cannot drift.
fn mem_row_tag(row: &crate::db::MemRow, is_global: bool, status: RefStatus) -> String {
    let mut tag = String::new();
    if row.superseded_by.is_some() {
        tag.push_str(" [superseded]");
    }
    tag.push_str(&store_tags(is_global, status, &row.source));
    tag
}

/// The caveated empty error for the `--all` listing, keyed on whether the
/// listing extended to superseded revisions.
fn mem_all_empty(include_superseded: bool) -> EgError {
    mem_empty(if include_superseded {
        "memory store has no notes"
    } else {
        "memory store has no current notes"
    })
}

/// The listing tail shared by the single-store and merged queries: the
/// empty guard with the caller's caveated error, the cap, the per-store
/// tag, and the journal. Each row carries its store marker (`true` = global).
fn render_mem_rows(
    rows: Vec<(crate::db::MemRow, bool)>,
    args: &MemQueryArgs<'_>,
    empty_error: impl FnOnce() -> EgError,
) -> Result<ArmOut> {
    let rows: Vec<_> = if let Some(target_ref) = args.reference {
        rows.into_iter()
            .filter(|(row, _)| crate::memory::ref_matches_target(&row.refs, target_ref))
            .collect()
    } else {
        rows
    };
    if rows.is_empty() {
        return Err(empty_error());
    }
    let total = rows.len();
    let shown = total.min(args.limit);
    let rendered: Vec<String> = rows
        .iter()
        .take(shown)
        .map(|(row, is_global)| {
            let status = check_note_ref_status(args.index_conn, row);
            crate::memory::render_row_tagged(row, &mem_row_tag(row, *is_global, status))
        })
        .collect();
    Ok(render_mem_list(&rendered, total, shown))
}

/// The ranked tail shared by the single-store and merged listings: fuse,
/// cap, render with the per-candidate store tag, and journal.
fn render_ranked(
    candidates: Vec<crate::memory::Candidate>,
    args: &MemQueryArgs<'_>,
) -> Result<ArmOut> {
    let fused = crate::memory::fuse(candidates);
    render_mem_rows(
        fused
            .into_iter()
            .map(|cand| (cand.row, cand.is_global))
            .collect(),
        args,
        || mem_empty(&format!("no memory matches '{}'", args.terms.join(" "))),
    )
}

/// The `--all` listing tail shared by the single-store and merged queries:
/// the caveated empty error keyed on superseded revisions.
fn render_all_rows(
    rows: Vec<(crate::db::MemRow, bool)>,
    args: &MemQueryArgs<'_>,
) -> Result<ArmOut> {
    render_mem_rows(rows, args, || mem_all_empty(args.filter.include_superseded))
}

fn query_single_store(
    conn: &rusqlite::Connection,
    args: &MemQueryArgs<'_>,
    is_global: bool,
) -> Result<ArmOut> {
    if args.all {
        let rows = crate::db::memory_current_rows(conn, &args.filter, None)?;
        return render_all_rows(rows.into_iter().map(|row| (row, is_global)).collect(), args);
    }
    let joined = args.terms.join(" ");
    let candidates = mem_ranked(conn, &joined, &args.filter, is_global, args.index_conn)?;
    render_ranked(candidates, args)
}

/// Local-first `--all` merge of the two stores: local current rows, then
/// global rows whose key the local store does not carry, newest first with
/// the key as tie-breaker. Each row keeps its store marker.
fn merge_all_rows(
    local_conn: &rusqlite::Connection,
    global_conn: &rusqlite::Connection,
    filter: &crate::db::MemFilter,
) -> Result<Vec<(crate::db::MemRow, bool)>> {
    let local_rows = crate::db::memory_current_rows(local_conn, filter, None)?;
    let global_rows = crate::db::memory_current_rows(global_conn, filter, None)?;
    let mut seen_keys = std::collections::HashSet::new();
    let mut combined = Vec::new();
    for row in local_rows {
        seen_keys.insert(row.key.clone());
        combined.push((row, false));
    }
    for row in global_rows {
        if !seen_keys.contains(&row.key) {
            combined.push((row, true));
        }
    }
    combined.sort_by(|(a, _), (b, _)| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.key.cmp(&b.key))
    });
    Ok(combined)
}

fn query_merged_stores(
    local_conn: &rusqlite::Connection,
    global_conn: &rusqlite::Connection,
    args: &MemQueryArgs<'_>,
) -> Result<ArmOut> {
    if args.all {
        let combined = merge_all_rows(local_conn, global_conn, &args.filter)?;
        return render_all_rows(combined, args);
    }
    let joined = args.terms.join(" ");
    let mut combined = mem_ranked(local_conn, &joined, &args.filter, false, args.index_conn)?;
    let seen_keys: std::collections::HashSet<String> =
        combined.iter().map(|c| c.row.key.clone()).collect();
    let global = mem_ranked(global_conn, &joined, &args.filter, true, args.index_conn)?;
    // One batched existence probe replaces the per-candidate `memory_get`
    // (N+1 with a revision COUNT per key): only keys the local ranking has
    // not already surfaced need the shadowing check.
    let probe: Vec<String> = global
        .iter()
        .map(|c| c.row.key.clone())
        .filter(|key| !seen_keys.contains(key))
        .collect();
    let local_keys: std::collections::HashSet<String> =
        crate::db::memory_current_keys(local_conn, &probe)?
            .into_iter()
            .collect();
    for c in global {
        if seen_keys.contains(&c.row.key) || local_keys.contains(&c.row.key) {
            continue;
        }
        combined.push(c);
    }
    render_ranked(combined, args)
}

/// Reject the flag combinations the two query modes cannot serve.
fn validate_mem_query(
    terms: &[String],
    explicit_all: bool,
    effective_all: bool,
    include_superseded: bool,
) -> Result<()> {
    if explicit_all && !terms.is_empty() {
        return Err(EgError::Usage(
            "mem query: --all and terms are mutually exclusive".to_string(),
        ));
    }
    if !terms.is_empty() && terms.iter().all(|t| !t.is_empty() && t.trim().is_empty()) {
        return Err(EgError::Usage(
            "mem query: provide terms or --all".to_string(),
        ));
    }
    // Superseded rows participate in the explicit listing only; the
    // ranked resolver keeps supersession-at-result-time exclusion.
    if include_superseded && !effective_all {
        return Err(EgError::Usage(
            "mem query: --include-superseded requires --all".to_string(),
        ));
    }
    Ok(())
}

/// Shared cap, journal, and stderr contract for the two mem query modes.
/// `rendered` holds exactly `shown` rows; `total` drives the cap footer.
fn render_mem_list(rendered: &[String], total: usize, shown: usize) -> ArmOut {
    let mut stdout = String::new();
    stdout.push_str(crate::memory::MEMORY_CAVEAT);
    stdout.push('\n');
    for line in rendered {
        stdout.push_str(line);
        stdout.push('\n');
    }
    if total > shown {
        stdout.push_str(&crate::query::cap_line(shown, total, "").unwrap_or_default());
        stdout.push('\n');
    }
    crate::journal::record_result(crate::journal::OutcomeCounts::of(shown, total), false, 0);
    ArmOut::plain(stdout, crate::query::cap_stderr(shown, total))
}

/// One mem get hit: journal, the caveat, the tagged row, and the revision
/// count (`tag` is the store marker, "" for local and " [global]" for global).
pub(super) fn render_mem_hit(row: &crate::db::MemRow, tag: &str, revs: i64) -> ArmOut {
    crate::journal::record_result(crate::journal::OutcomeCounts::uniform(1), false, 0);
    ArmOut::plain(
        format!(
            "{}\n{}\nrevs: {}\n",
            crate::memory::MEMORY_CAVEAT,
            crate::memory::render_row_tagged(row, tag),
            revs
        ),
        String::new(),
    )
}
