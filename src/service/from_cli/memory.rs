//! CLI-to-request conversion for the memory family. Kept in a submodule so
//! the parent mapper stays under the 500-line hygiene cap.

use crate::cli::MemAction;
use crate::error::Result;
use crate::service::scope::StoreScope;
use crate::service::CommandRequest;

impl CommandRequest {
    /// Memory subcommands. `write`/`query`/`export` destructure through their
    /// own request helpers (the same surface-boundary split `impact_request`
    /// uses).
    pub(super) fn from_cli_memory(cmd: crate::cli::Cmd) -> Result<CommandRequest> {
        let crate::cli::Cmd::Mem { action } = cmd else {
            unreachable!("from_cli_memory called with a non-matching command")
        };
        match action {
            MemAction::Write { .. } => Self::mem_write_request(action),
            MemAction::Touch { key, global } => Ok(CommandRequest::MemTouch { key, global }),
            MemAction::Query { .. } => Self::mem_query_request(action),
            MemAction::Get { key, global } => Ok(CommandRequest::MemGet { key, global }),
            MemAction::Delete { key, global } => Ok(CommandRequest::MemDelete { key, global }),
            MemAction::Export { .. } => Self::mem_export_request(action),
            MemAction::Import {
                path,
                format,
                force,
                global,
            } => Ok(CommandRequest::MemImport {
                path,
                format,
                force,
                global,
            }),
            MemAction::Gc { global } => Ok(CommandRequest::MemGc { global }),
            MemAction::Reindex { global } => Ok(CommandRequest::MemReindex { global }),
        }
    }

    /// The `mem write` request shape, destructured from the CLI variant.
    fn mem_write_request(action: MemAction) -> Result<CommandRequest> {
        let MemAction::Write {
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
        } = action
        else {
            unreachable!("mem_write_request called with a non-write action")
        };
        Ok(CommandRequest::MemWrite {
            key,
            body,
            kind,
            tags: Self::join_list(tags),
            refs: Self::join_list(refs),
            scope,
            ttl,
            supersedes,
            global,
            touch,
        })
    }

    /// Collapse `--tags a b` and `--tags a --tags b` into the one
    /// space-separated string the request carries.
    fn join_list(values: Vec<String>) -> Option<String> {
        let joined = values.join(" ");
        (!joined.is_empty()).then_some(joined)
    }

    /// The `mem query` request shape, destructured from the CLI variant.
    fn mem_query_request(action: MemAction) -> Result<CommandRequest> {
        let MemAction::Query {
            terms,
            all,
            reference,
            kind,
            scope,
            limit,
            include_superseded,
            global,
            since,
            until,
            local,
            explain,
            legacy,
        } = action
        else {
            unreachable!("mem_query_request called with a non-query action")
        };
        StoreScope::from_flags(local, global)?;
        let window = crate::memory::TimeWindow {
            since: since
                .as_deref()
                .map(|r| crate::memory::parse_time_bound(r, "mem query", "--since"))
                .transpose()?,
            until: until
                .as_deref()
                .map(|r| crate::memory::parse_time_bound(r, "mem query", "--until"))
                .transpose()?,
        };
        crate::memory::validate_window(&window, "mem query")?;
        Ok(CommandRequest::MemQuery {
            terms,
            all,
            kind: crate::memory::parse_kind_filter("mem query", kind.as_deref())?,
            scope: crate::memory::parse_scope_filter("mem query", scope.as_deref())?,
            limit,
            include_superseded,
            global,
            reference,
            since: window.since,
            until: window.until,
            local,
            explain,
            legacy,
        })
    }

    /// The `mem export` request shape, destructured from the CLI variant.
    fn mem_export_request(action: MemAction) -> Result<CommandRequest> {
        let MemAction::Export {
            format,
            limit,
            global,
            since,
            until,
            scan,
        } = action
        else {
            unreachable!("mem_export_request called with a non-export action")
        };
        let window = crate::memory::TimeWindow {
            since: since
                .as_deref()
                .map(|r| crate::memory::parse_time_bound(r, "mem export", "--since"))
                .transpose()?,
            until: until
                .as_deref()
                .map(|r| crate::memory::parse_time_bound(r, "mem export", "--until"))
                .transpose()?,
        };
        crate::memory::validate_window(&window, "mem export")?;
        Ok(CommandRequest::MemExport {
            format,
            limit,
            global,
            since: window.since,
            until: window.until,
            scan,
        })
    }
}
