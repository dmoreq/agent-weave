//! Memory writes: note validation, normalization, revision outcome lines,
//! and local/global store resolution.

use super::*;

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
            return Err(AwError::Usage(
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
    Ok(ArmOut::plain(
        format!("{}\n{line}\n", crate::memory::MEMORY_CAVEAT),
        String::new(),
    ))
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
            Err(AwError::NoMemory(_)) => None,
            Err(e) => return Err(e),
        },
        Err(_) => None,
    };
    let global = match crate::db::open_global_memory_query() {
        Ok(conn) => Some(conn),
        Err(AwError::NoMemory(_)) => None,
        Err(e) => return Err(e),
    };
    Ok(MemoryStores { local, global })
}

/// The shared "no memory store anywhere" error: name the local root when
/// discovery worked, the generic write hint otherwise.
pub(super) fn no_memory_store_error(root_arg: Option<&Path>) -> AwError {
    if let Ok(root) = crate::walk::discover_root(root_arg) {
        crate::db::no_memory_error(&root)
    } else {
        AwError::NoMemory("no memory store found — write with aw mem write".to_string())
    }
}
