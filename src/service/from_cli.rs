//! CLI-to-request conversion: command family routing and the
//! `CommandRequest::from_cli` mapping for every subcommand.

use super::*;

/// Coarse routing family shared by the CLI, journaling, and execution
/// dispatch. `family()` is exhaustive, so a new variant cannot be added
/// without choosing a family; the dispatchers then only match this enum.
///
/// Debt (S2): one command still edits 11 sites even with this enum; the
/// review's `<= 7` needs a macro/registry or a single clap-derived enum.
/// Accepted and tracked in `docs/archive/reports/2026-11-06_s2-dispatch-edit-sites.md`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Family {
    Diag,
    Graph,
    Analysis,
    Memory,
}

/// CLI command family (the CLI has the extra `Mcp` hand-off variant).
fn cli_family(cmd: &crate::cli::Cmd) -> Family {
    match cmd {
        crate::cli::Cmd::Build { .. }
        | crate::cli::Cmd::Doctor { .. }
        | crate::cli::Cmd::Usage { .. }
        | crate::cli::Cmd::Mcp { .. }
        | crate::cli::Cmd::Dashboard { .. }
        | crate::cli::Cmd::Monitor { .. } => Family::Diag,
        crate::cli::Cmd::Callers { .. }
        | crate::cli::Cmd::Callees { .. }
        | crate::cli::Cmd::Context { .. }
        | crate::cli::Cmd::Imports { .. }
        | crate::cli::Cmd::Path { .. }
        | crate::cli::Cmd::Impact { .. } => Family::Graph,
        crate::cli::Cmd::Unresolved { .. }
        | crate::cli::Cmd::Symbols { .. }
        | crate::cli::Cmd::Reuse { .. }
        | crate::cli::Cmd::Search { .. }
        | crate::cli::Cmd::Stats { .. } => Family::Analysis,
        crate::cli::Cmd::Mem { .. } => Family::Memory,
    }
}

/// Comma-joined `--edges` kinds for the `impact` journal line.
fn edge_kinds_csv(kinds: &[crate::extract::EdgeKind]) -> String {
    kinds
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

/// Shared journal fields for the graph queries; `depth` joins only for the
/// two call-graph directions (callers/callees). Built in explicit branches so
/// field order never depends on a positional `insert`.
fn graph_args(name: &str, depth: Option<usize>, limit: usize) -> JournalArgs {
    match depth {
        Some(depth) => vec![
            ("name", name.to_string()),
            ("depth", depth.to_string()),
            ("limit", limit.to_string()),
        ],
        None => vec![("name", name.to_string()), ("limit", limit.to_string())],
    }
}

/// Shared trailing journal fields for the path-scoped analysis commands:
/// `head` first, then the optional path (empty when absent) and the limit.
/// Field order matches the historical journal line.
fn path_limit_args(head: &JournalArgs, path: Option<&str>, limit: usize) -> JournalArgs {
    let mut out = head.to_vec();
    out.push(("path", path.map(str::to_string).unwrap_or_default()));
    out.push(("limit", limit.to_string()));
    out
}

impl CommandRequest {
    /// CLI conversion. `resolve --stdin` reads its names here so the service
    /// stays free of surface-specific input shapes (MCP passes a JSON array).
    pub fn from_cli(cmd: crate::cli::Cmd) -> Result<CommandRequest> {
        match cli_family(&cmd) {
            Family::Diag => Self::from_cli_diag(cmd),
            Family::Graph => Self::from_cli_graph(cmd),
            Family::Analysis => Self::from_cli_analysis(cmd),
            Family::Memory => Self::from_cli_memory(cmd),
        }
    }

    /// Build, doctor, usage, and the MCP hand-off.
    fn from_cli_diag(cmd: crate::cli::Cmd) -> Result<CommandRequest> {
        Ok(match cmd {
            crate::cli::Cmd::Build {
                force,
                max_parse_error_rate,
            } => CommandRequest::Build {
                force,
                max_parse_error_rate,
            },
            crate::cli::Cmd::Doctor { invalidate } => CommandRequest::Doctor { invalidate },
            crate::cli::Cmd::Usage { quality, global } => CommandRequest::Usage { quality, global },
            crate::cli::Cmd::Mcp { .. } => {
                unreachable!("mcp is a protocol loop, handled before dispatch")
            }
            _ => unreachable!("from_cli_diag called with a non-matching command"),
        })
    }
    /// Graph traversal and call-site commands.
    fn from_cli_graph(cmd: crate::cli::Cmd) -> Result<CommandRequest> {
        Ok(match cmd {
            crate::cli::Cmd::Callers { name, limit, depth } => {
                CommandRequest::Callers { name, limit, depth }
            }
            crate::cli::Cmd::Callees { name, limit, depth } => {
                CommandRequest::Callees { name, limit, depth }
            }
            crate::cli::Cmd::Context { name, limit } => CommandRequest::Context { name, limit },
            crate::cli::Cmd::Imports { file, limit } => CommandRequest::Imports { file, limit },
            crate::cli::Cmd::Path { from, to, depth } => CommandRequest::Path { from, to, depth },
            crate::cli::Cmd::Impact { .. } => Self::impact_request(cmd)?,
            _ => unreachable!("from_cli_graph called with a non-matching command"),
        })
    }

    /// The `impact` request shape, destructured from the CLI variant: `--edges`
    /// is parsed at this service boundary so the request carries validated
    /// kinds (a bad spec is a usage error before dispatch, on both surfaces).
    fn impact_request(cmd: crate::cli::Cmd) -> Result<CommandRequest> {
        let crate::cli::Cmd::Impact {
            file,
            sites,
            depth,
            edges,
            limit,
        } = cmd
        else {
            unreachable!("impact_request called with a non-impact command")
        };
        Ok(CommandRequest::Impact {
            file,
            sites,
            depth,
            edges: crate::query::parse_edge_kinds(&edges)?,
            limit,
        })
    }
    /// Analysis, inventory, and anti-reinvention.
    fn from_cli_analysis(cmd: crate::cli::Cmd) -> Result<CommandRequest> {
        Ok(match cmd {
            crate::cli::Cmd::Unresolved { names, path, limit } => {
                CommandRequest::Unresolved { names, path, limit }
            }
            crate::cli::Cmd::Symbols {
                file,
                outline,
                limit,
            } => CommandRequest::Symbols {
                file,
                outline,
                limit,
            },
            crate::cli::Cmd::Reuse {
                paths,
                limit,
                deep_callee,
                judge,
            } => CommandRequest::Reuse {
                paths,
                limit,
                deep_callee,
                judge,
            },
            crate::cli::Cmd::Search { terms, limit } => CommandRequest::Search { terms, limit },
            crate::cli::Cmd::Stats { path } => CommandRequest::Stats { path },
            _ => unreachable!("from_cli_analysis called with a non-matching command"),
        })
    }
    /// Memory subcommands. `write`/`query` destructure through their own
    /// request helpers (the same surface-boundary split `impact_request` uses).
    fn from_cli_memory(cmd: crate::cli::Cmd) -> Result<CommandRequest> {
        let crate::cli::Cmd::Mem { action } = cmd else {
            unreachable!("from_cli_memory called with a non-matching command")
        };
        match action {
            crate::cli::MemAction::Write { .. } => Self::mem_write_request(action),
            crate::cli::MemAction::Touch { key, global } => {
                Ok(CommandRequest::MemTouch { key, global })
            }
            crate::cli::MemAction::Query { .. } => Self::mem_query_request(action),
            crate::cli::MemAction::Get { key, global } => {
                Ok(CommandRequest::MemGet { key, global })
            }
            crate::cli::MemAction::Delete { key, global } => {
                Ok(CommandRequest::MemDelete { key, global })
            }
            crate::cli::MemAction::Export {
                format,
                limit,
                global,
            } => Ok(CommandRequest::MemExport {
                format,
                limit,
                global,
            }),
            crate::cli::MemAction::Import {
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
            crate::cli::MemAction::Gc { global } => Ok(CommandRequest::MemGc { global }),
        }
    }

    /// The `mem write` request shape, destructured from the CLI variant.
    fn mem_write_request(action: crate::cli::MemAction) -> Result<CommandRequest> {
        let crate::cli::MemAction::Write {
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
            tags,
            refs,
            scope,
            ttl,
            supersedes,
            global,
            touch,
        })
    }

    /// The `mem query` request shape, destructured from the CLI variant.
    fn mem_query_request(action: crate::cli::MemAction) -> Result<CommandRequest> {
        let crate::cli::MemAction::Query {
            terms,
            all,
            reference,
            kind,
            scope,
            limit,
            include_superseded,
            global,
        } = action
        else {
            unreachable!("mem_query_request called with a non-query action")
        };
        Ok(CommandRequest::MemQuery {
            terms,
            all,
            kind: crate::memory::parse_kind_filter("mem query", kind.as_deref())?,
            scope: crate::memory::parse_scope_filter("mem query", scope.as_deref())?,
            limit,
            include_superseded,
            global,
            reference,
        })
    }

    /// Routing family for dispatch. Exhaustive: a new variant must be
    /// classified here before the family dispatchers can route it.
    pub(super) fn family(&self) -> Family {
        match self {
            CommandRequest::Build { .. }
            | CommandRequest::Doctor { .. }
            | CommandRequest::Usage { .. } => Family::Diag,
            CommandRequest::Callers { .. }
            | CommandRequest::Callees { .. }
            | CommandRequest::Context { .. }
            | CommandRequest::Imports { .. }
            | CommandRequest::Path { .. }
            | CommandRequest::Impact { .. } => Family::Graph,
            CommandRequest::Unresolved { .. }
            | CommandRequest::Symbols { .. }
            | CommandRequest::Reuse { .. }
            | CommandRequest::Search { .. }
            | CommandRequest::Stats { .. } => Family::Analysis,
            CommandRequest::MemWrite { .. }
            | CommandRequest::MemTouch { .. }
            | CommandRequest::MemQuery { .. }
            | CommandRequest::MemGet { .. }
            | CommandRequest::MemDelete { .. }
            | CommandRequest::MemGc { .. }
            | CommandRequest::MemExport { .. }
            | CommandRequest::MemImport { .. } => Family::Memory,
        }
    }

    /// Tool name as journaled and keyed by the MCP tool registry.
    pub fn tool_name(&self) -> &'static str {
        match self {
            CommandRequest::Build { .. } => "build",
            CommandRequest::Callers { .. } => "callers",
            CommandRequest::Callees { .. } => "callees",
            CommandRequest::Context { .. } => "context",
            CommandRequest::Imports { .. } => "imports",
            CommandRequest::Path { .. } => "path",
            CommandRequest::Impact { .. } => "impact",
            CommandRequest::Unresolved { .. } => "unresolved",
            CommandRequest::Symbols { .. } => "symbols",
            CommandRequest::Reuse { .. } => "reuse",
            CommandRequest::Search { .. } => "search",
            CommandRequest::Stats { .. } => "stats",
            CommandRequest::Doctor { .. } => "doctor",
            CommandRequest::Usage { .. } => "usage",
            CommandRequest::MemWrite { .. } => "mem_write",
            CommandRequest::MemTouch { .. } => "mem_touch",
            CommandRequest::MemQuery { .. } => "mem_query",
            CommandRequest::MemGet { .. } => "mem_get",
            CommandRequest::MemDelete { .. } => "mem_delete",
            CommandRequest::MemGc { .. } => "mem_gc",
            CommandRequest::MemExport { .. } => "mem_export",
            CommandRequest::MemImport { .. } => "mem_import",
        }
    }

    /// Journal argument object, identical on both surfaces (one source of
    /// truth replaces the former per-surface argument tables).
    pub fn journal_args(&self) -> JournalArgs {
        match self.family() {
            Family::Diag => self.journal_args_diag(),
            Family::Graph => self.journal_args_graph(),
            Family::Analysis => self.journal_args_analysis(),
            Family::Memory => self.journal_args_memory(),
        }
    }

    /// Journal fields for build, doctor, and usage.
    fn journal_args_diag(&self) -> JournalArgs {
        match self {
            CommandRequest::Build {
                force,
                max_parse_error_rate,
            } => vec![
                ("force", force.to_string()),
                ("threshold", max_parse_error_rate.to_string()),
            ],
            CommandRequest::Doctor { invalidate } => {
                vec![("invalidate", invalidate.to_string())]
            }
            CommandRequest::Usage { quality, global } => vec![
                ("quality", quality.to_string()),
                ("global", global.to_string()),
            ],
            _ => unreachable!("journal_args_diag called with a non-matching request"),
        }
    }
    /// Journal fields for the graph queries.
    fn journal_args_graph(&self) -> JournalArgs {
        match self {
            CommandRequest::Callers { name, limit, depth }
            | CommandRequest::Callees { name, limit, depth } => {
                graph_args(name, Some(*depth), *limit)
            }
            CommandRequest::Context { name, limit } => graph_args(name, None, *limit),
            CommandRequest::Imports { file, limit } => {
                vec![("file", file.clone()), ("limit", limit.to_string())]
            }
            CommandRequest::Path { from, to, depth } => vec![
                ("name", from.clone()),
                ("to", to.clone()),
                ("depth", depth.to_string()),
            ],
            CommandRequest::Impact {
                file,
                sites,
                depth,
                edges,
                limit,
            } => vec![
                ("file", file.clone()),
                ("sites", sites.to_string()),
                ("depth", depth.to_string()),
                ("edges", edge_kinds_csv(edges)),
                ("limit", limit.to_string()),
            ],
            _ => unreachable!("journal_args_graph called with a non-matching request"),
        }
    }
    /// Journal fields for analysis and inventory.
    fn journal_args_analysis(&self) -> JournalArgs {
        match self {
            CommandRequest::Unresolved { names, path, limit } => {
                path_limit_args(&vec![("names", names.to_string())], path.as_deref(), *limit)
            }
            CommandRequest::Symbols {
                file,
                outline,
                limit,
            } => vec![
                ("file", file.clone()),
                ("outline", outline.to_string()),
                ("limit", limit.to_string()),
            ],
            CommandRequest::Reuse {
                paths,
                limit,
                deep_callee,
                judge,
            } => vec![
                ("paths", paths.join(",")),
                ("limit", limit.to_string()),
                ("deep_callee", deep_callee.to_string()),
                ("judge", judge.to_string()),
            ],
            CommandRequest::Search { terms, limit } => {
                vec![("terms", terms.clone()), ("limit", limit.to_string())]
            }
            CommandRequest::Stats { path } => {
                vec![("path", path.clone().unwrap_or_default())]
            }
            _ => unreachable!("journal_args_analysis called with a non-matching request"),
        }
    }
    /// Journal fields for memory commands: key/kind/scope/terms/limit/body_len
    /// — never the body text (journal privacy contract). The two wide variants
    /// destructure through their own helpers so each function stays small.
    fn journal_args_memory(&self) -> JournalArgs {
        match self {
            CommandRequest::MemWrite { .. } => mem_write_args(self),
            CommandRequest::MemTouch { key, global } => {
                vec![("key", key.clone()), ("global", global.to_string())]
            }
            CommandRequest::MemQuery { .. } => mem_query_args(self),
            CommandRequest::MemGet { key, global } => {
                vec![("key", key.clone()), ("global", global.to_string())]
            }
            CommandRequest::MemDelete { key, global } => {
                vec![("key", key.clone()), ("global", global.to_string())]
            }
            CommandRequest::MemGc { global } => vec![("global", global.to_string())],
            CommandRequest::MemExport {
                format,
                limit,
                global,
            } => vec![
                ("format", format.clone().unwrap_or_default()),
                ("limit", limit.map_or_else(String::new, |n| n.to_string())),
                ("global", global.to_string()),
            ],
            CommandRequest::MemImport {
                format,
                force,
                global,
                ..
            } => vec![
                ("format", format.clone().unwrap_or_default()),
                ("force", force.to_string()),
                ("global", global.to_string()),
            ],
            _ => unreachable!("journal_args_memory called with a non-matching request"),
        }
    }
}
