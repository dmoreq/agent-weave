//! Memory writes: note validation, normalization, revision outcome lines,
//! and local/global store resolution.

use super::*;
use crate::journal::JournalArgs;

/// Persist one note (validation, normalization, and the write outcome line).
pub(super) fn run_mem_write(
    req: &CommandRequest,
    root_arg: Option<&Path>,
    surface: Surface,
) -> Result<ArmOut> {
    let CommandRequest::MemWrite {
        key,
        body,
        kind,
        tags,
        refs,
        scope,
        ttl,
        supersedes,
        global,
        touch,
    } = req
    else {
        unreachable!("run_mem_write called with a non-write request")
    };
    if let Some(target) = supersedes {
        if target.trim() == key.trim() {
            return Err(EgError::Usage(
                "mem write: --supersedes must name a different key (same-key updates go through the revision chain)"
                    .to_string(),
            ));
        }
    }
    let note = crate::memory::normalize_note(crate::memory::NoteInput {
        key,
        kind,
        tags: tags.as_deref(),
        refs: refs.as_deref(),
        scope,
        body,
        ttl: *ttl,
    })?;
    // Advisory only: a near-duplicate under another key suggests --supersedes.
    // Same-key updates and explicit --supersedes already express intent.
    let consolidation = if supersedes.is_none() {
        open_mem_conn(root_arg, *global, MemoryMode::Query)
            .ok()
            .and_then(|conn| crate::memory::find_consolidation_match(&conn, &note))
    } else {
        None
    };
    let outcome = write_note(
        root_arg,
        &note,
        surface,
        supersedes.as_deref(),
        *global,
        *touch,
    )?;
    crate::journal::record_result(crate::journal::OutcomeCounts::uniform(1), false, 0);
    let line = outcome_line(&outcome, &note.key);
    let mut stdout = format!("{}\n{line}\n", crate::memory::MEMORY_CAVEAT);
    if let Some(m) = consolidation {
        stdout.push_str(&m.hint_line(&note.key));
        stdout.push('\n');
    }
    Ok(ArmOut::plain(stdout, String::new()))
}

fn write_note(
    root_arg: Option<&Path>,
    note: &crate::memory::NormalizedNote,
    surface: Surface,
    supersedes: Option<&str>,
    global: bool,
    touch: bool,
) -> Result<crate::db::MemWriteOutcome> {
    let mut conn = open_mem_conn(root_arg, global, MemoryMode::Store)?;
    let session = crate::journal::session_tag();
    let now = crate::db::unix_now();
    crate::db::memory_write(
        &mut conn,
        &crate::db::MemNoteInput {
            key: &note.key,
            kind: note.kind.as_str(),
            tags: &note.tags,
            refs: &note.refs,
            scope: note.scope.as_str(),
            session: &session,
            body: &note.body,
            source: surface.as_str(),
            expires_at: if note.ttl_secs == 0 {
                0
            } else {
                now + note.ttl_secs
            },
            supersedes,
            touch,
        },
        now,
    )
}

fn outcome_line(outcome: &crate::db::MemWriteOutcome, key: &str) -> String {
    match outcome {
        crate::db::MemWriteOutcome::Written {
            rev,
            superseded_rev,
        } => match superseded_rev {
            Some(old) => format!("ok: '{key}' rev {rev} written (supersedes rev {old})"),
            None => format!("ok: '{key}' rev {rev} written"),
        },
        crate::db::MemWriteOutcome::Duplicate { rev } => {
            format!("ok: '{key}' unchanged at rev {rev} (identical content)")
        }
        crate::db::MemWriteOutcome::Touched { rev } => {
            format!("ok: '{key}' touched at rev {rev} (timestamp refreshed)")
        }
    }
}

/// The query-shaped fields of `MemQuery`, bundled once and passed to the
/// single-store and merged-store listings (the Data Clump those functions used
/// to receive as loose parameters; `include_superseded` rides in `filter`).
pub(super) struct MemQueryArgs<'a> {
    pub(super) terms: &'a [String],
    pub(super) all: bool,
    pub(super) limit: usize,
    pub(super) filter: crate::db::MemFilter,
    pub(super) index_conn: Option<&'a rusqlite::Connection>,
    pub(super) reference: Option<&'a str>,
    pub(super) explain: bool,
    pub(super) legacy: bool,
}

/// Journal fields for `mem write` (key/kind/scope/ttl/supersedes/body_len —
/// never the body text).
pub(super) fn mem_write_args(req: &CommandRequest) -> JournalArgs {
    let CommandRequest::MemWrite {
        key,
        body,
        kind,
        scope,
        ttl,
        supersedes,
        global,
        touch,
        ..
    } = req
    else {
        unreachable!("mem_write_args called with a non-write request")
    };
    vec![
        ("key", key.clone()),
        ("kind", kind.clone()),
        ("scope", scope.clone()),
        ("ttl", ttl.map_or_else(String::new, |t| t.to_string())),
        ("supersedes", supersedes.clone().unwrap_or_default()),
        ("body_len", body.len().to_string()),
        ("global", global.to_string()),
        ("touch", touch.to_string()),
    ]
}

/// Journal fields for `mem query` (terms/ref/kind/scope/limit/...).
pub(super) fn mem_query_args(req: &CommandRequest) -> JournalArgs {
    let CommandRequest::MemQuery {
        terms,
        all: _,
        kind,
        scope,
        limit,
        include_superseded,
        global,
        reference,
        since,
        until,
        local,
        explain,
        legacy,
    } = req
    else {
        unreachable!("mem_query_args called with a non-query request")
    };
    vec![
        ("terms", terms.join(" ")),
        ("ref", reference.clone().unwrap_or_default()),
        (
            "kind",
            kind.map_or_else(String::new, |k| k.as_str().to_string()),
        ),
        (
            "scope",
            scope.map_or_else(String::new, |s| s.as_str().to_string()),
        ),
        ("limit", limit.to_string()),
        ("include_superseded", include_superseded.to_string()),
        ("global", global.to_string()),
        ("since", since.map_or_else(String::new, |n| n.to_string())),
        ("until", until.map_or_else(String::new, |n| n.to_string())),
        ("local", local.to_string()),
        ("explain", explain.to_string()),
        ("legacy", legacy.to_string()),
    ]
}

/// The local and global memory stores a read command resolves once: `None`
/// means the store is absent (open errors other than "no store" propagate).
pub(super) struct MemoryStores {
    pub(super) local: Option<rusqlite::Connection>,
    pub(super) global: Option<rusqlite::Connection>,
}

/// Open the memory stores both `mem query` and `mem get` read: the local store
/// (only when root discovery and the open succeed) and the global one.
pub(super) fn resolve_memory_stores(root_arg: Option<&Path>) -> Result<MemoryStores> {
    let local = match crate::walk::discover_root(root_arg) {
        Ok(root) => match crate::db::open_memory_query(&root) {
            Ok(conn) => Some(conn),
            Err(EgError::NoMemory(_)) => None,
            Err(e) => return Err(e),
        },
        // An explicit --root that cannot resolve is a usage error; an implicit
        // lookup with no git ancestor is not — the global store still applies.
        Err(e) if root_arg.is_some() => return Err(e),
        Err(_) => None,
    };
    let global = match crate::db::open_global_memory_query() {
        Ok(conn) => Some(conn),
        Err(EgError::NoMemory(_)) => None,
        Err(e) => return Err(e),
    };
    Ok(MemoryStores { local, global })
}

/// The shared "no memory store anywhere" error: name the local root when
/// discovery worked, the generic write hint otherwise.
pub(super) fn no_memory_store_error(root_arg: Option<&Path>) -> EgError {
    if let Ok(root) = crate::walk::discover_root(root_arg) {
        crate::db::no_memory_error(&root)
    } else {
        EgError::NoMemory("no memory store found — write with eg mem write".to_string())
    }
}
