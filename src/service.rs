//! Unified command dispatch shared by the CLI and MCP surfaces: one place
//! resolves the repo root, opens the index, journals the invocation, renders
//! stdout/stderr, and maps errors to exit codes. The CLI and the MCP server
//! both convert their input into a `CommandRequest` and print the returned
//! `ExecutionResult`, so the two surfaces stay byte-identical by construction.

use std::io::Read;
use std::path::Path;
use std::time::Instant;

use crate::error::{AwError, Result};

/// Which surface is dispatching; journaled as `surface` in `.aw/usage.log`.
#[derive(Clone, Copy)]
pub enum Surface {
    Cli,
    Mcp,
}

impl Surface {
    fn as_str(self) -> &'static str {
        match self {
            Surface::Cli => "cli",
            Surface::Mcp => "mcp",
        }
    }
}

/// One dispatchable command with its resolved arguments.
#[derive(Debug)]
pub enum CommandRequest {
    Build {
        force: bool,
        max_parse_error_rate: f64,
    },
    Callers {
        name: String,
        limit: usize,
        depth: usize,
        explain: bool,
    },
    Callees {
        name: String,
        limit: usize,
        depth: usize,
        explain: bool,
    },
    Context {
        name: String,
        limit: usize,
        explain: bool,
    },
    Implementers {
        name: String,
        limit: usize,
    },
    Imports {
        file: String,
        limit: usize,
    },
    Path {
        from: String,
        to: String,
        depth: usize,
    },
    Impact {
        file: String,
        sites: bool,
        depth: usize,
        /// Validated `--edges` kinds (parsed at the surface boundary, so a bad
        /// spec is a usage error before dispatch).
        edges: Vec<crate::extract::EdgeKind>,
        context: bool,
        explain: bool,
        limit: usize,
    },
    Dead {
        exclude_tests: bool,
        path: Option<String>,
        limit: usize,
    },
    Unresolved {
        names: bool,
        path: Option<String>,
        limit: usize,
    },
    Symbols {
        file: String,
        outline: bool,
        metrics: bool,
        limit: usize,
    },
    Complexity {
        min_loc: usize,
        path: Option<String>,
        limit: usize,
    },
    Resolve {
        names: Vec<String>,
    },
    Files {
        path: Option<String>,
        limit: usize,
    },
    Search {
        terms: String,
        limit: usize,
    },
    Stats {
        path: Option<String>,
    },
    Doctor {
        invalidate: bool,
    },
    Usage {
        quality: bool,
    },
    MemWrite {
        key: String,
        body: String,
        kind: String,
        tags: Option<String>,
        refs: Option<String>,
        scope: String,
        ttl: Option<u64>,
        supersedes: Option<String>,
    },
    MemQuery {
        terms: Vec<String>,
        all: bool,
        kind: Option<String>,
        scope: Option<String>,
        limit: usize,
        include_superseded: bool,
    },
    MemGet {
        key: String,
    },
    MemDelete {
        key: String,
    },
    MemGc,
    MemExport {
        limit: Option<usize>,
    },
}

/// Coarse routing family shared by the CLI, journaling, and execution
/// dispatch. `family()` is exhaustive, so a new variant cannot be added
/// without choosing a family; the dispatchers then only match this enum.
///
/// Debt (S2): one command still edits 11 sites even with this enum; the
/// review's `<= 7` needs a macro/registry or a single clap-derived enum.
/// Accepted and tracked in `docs/reports/2026-11-06_s2-dispatch-edit-sites.md`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
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
        | crate::cli::Cmd::Mcp { .. } => Family::Diag,
        crate::cli::Cmd::Callers { .. }
        | crate::cli::Cmd::Callees { .. }
        | crate::cli::Cmd::Context { .. }
        | crate::cli::Cmd::Implementers { .. }
        | crate::cli::Cmd::Imports { .. }
        | crate::cli::Cmd::Path { .. }
        | crate::cli::Cmd::Impact { .. } => Family::Graph,
        crate::cli::Cmd::Dead { .. }
        | crate::cli::Cmd::Unresolved { .. }
        | crate::cli::Cmd::Symbols { .. }
        | crate::cli::Cmd::Complexity { .. }
        | crate::cli::Cmd::Resolve { .. }
        | crate::cli::Cmd::Files { .. }
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
fn graph_args(
    name: &str,
    depth: Option<usize>,
    limit: usize,
    explain: bool,
) -> Vec<(&'static str, String)> {
    match depth {
        Some(depth) => vec![
            ("name", name.to_string()),
            ("depth", depth.to_string()),
            ("limit", limit.to_string()),
            ("explain", explain.to_string()),
        ],
        None => vec![
            ("name", name.to_string()),
            ("limit", limit.to_string()),
            ("explain", explain.to_string()),
        ],
    }
}

/// Shared trailing journal fields for the path-scoped analysis commands:
/// `head` first, then the optional path (empty when absent) and the limit.
/// Field order matches the historical journal line.
fn path_limit_args(
    head: &[(&'static str, String)],
    path: Option<&String>,
    limit: usize,
) -> Vec<(&'static str, String)> {
    let mut out = head.to_vec();
    out.push(("path", path.cloned().unwrap_or_default()));
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
            crate::cli::Cmd::Usage { quality } => CommandRequest::Usage { quality },
            crate::cli::Cmd::Mcp { .. } => {
                unreachable!("mcp is a protocol loop, handled before dispatch")
            }
            _ => unreachable!("from_cli_diag called with a non-matching command"),
        })
    }
    /// Graph traversal and call-site commands.
    fn from_cli_graph(cmd: crate::cli::Cmd) -> Result<CommandRequest> {
        Ok(match cmd {
            crate::cli::Cmd::Callers {
                name,
                limit,
                depth,
                explain,
            } => CommandRequest::Callers {
                name,
                limit,
                depth,
                explain,
            },
            crate::cli::Cmd::Callees {
                name,
                limit,
                depth,
                explain,
            } => CommandRequest::Callees {
                name,
                limit,
                depth,
                explain,
            },
            crate::cli::Cmd::Context {
                name,
                limit,
                explain,
            } => CommandRequest::Context {
                name,
                limit,
                explain,
            },
            crate::cli::Cmd::Implementers { name, limit } => {
                CommandRequest::Implementers { name, limit }
            }
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
            context,
            explain,
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
            context,
            explain,
            limit,
        })
    }
    /// Analysis, inventory, and batch resolution.
    fn from_cli_analysis(cmd: crate::cli::Cmd) -> Result<CommandRequest> {
        Ok(match cmd {
            crate::cli::Cmd::Dead {
                exclude_tests,
                path,
                limit,
            } => CommandRequest::Dead {
                exclude_tests,
                path,
                limit,
            },
            crate::cli::Cmd::Unresolved { names, path, limit } => {
                CommandRequest::Unresolved { names, path, limit }
            }
            crate::cli::Cmd::Symbols {
                file,
                outline,
                metrics,
                limit,
            } => CommandRequest::Symbols {
                file,
                outline,
                metrics,
                limit,
            },
            crate::cli::Cmd::Complexity {
                min_loc,
                path,
                limit,
            } => CommandRequest::Complexity {
                min_loc,
                path,
                limit,
            },
            crate::cli::Cmd::Resolve { stdin } => {
                if !stdin {
                    return Err(AwError::Usage(
                        "resolve reads names from stdin; pass --stdin".to_string(),
                    ));
                }
                CommandRequest::Resolve {
                    names: read_resolve_stdin()?,
                }
            }
            crate::cli::Cmd::Files { path, limit } => CommandRequest::Files { path, limit },
            crate::cli::Cmd::Search { terms, limit } => CommandRequest::Search { terms, limit },
            crate::cli::Cmd::Stats { path } => CommandRequest::Stats { path },
            _ => unreachable!("from_cli_analysis called with a non-matching command"),
        })
    }
    /// Memory subcommands.
    fn from_cli_memory(cmd: crate::cli::Cmd) -> Result<CommandRequest> {
        Ok(match cmd {
            crate::cli::Cmd::Mem { action } => match action {
                crate::cli::MemAction::Write {
                    key,
                    body,
                    kind,
                    tags,
                    refs,
                    scope,
                    ttl,
                    supersedes,
                } => CommandRequest::MemWrite {
                    key,
                    body,
                    kind,
                    tags,
                    refs,
                    scope,
                    ttl,
                    supersedes,
                },
                crate::cli::MemAction::Query {
                    terms,
                    all,
                    kind,
                    scope,
                    limit,
                    include_superseded,
                } => CommandRequest::MemQuery {
                    terms,
                    all,
                    kind,
                    scope,
                    limit,
                    include_superseded,
                },
                crate::cli::MemAction::Get { key } => CommandRequest::MemGet { key },
                crate::cli::MemAction::Delete { key } => CommandRequest::MemDelete { key },
                crate::cli::MemAction::Export { limit } => CommandRequest::MemExport { limit },
                crate::cli::MemAction::Gc => CommandRequest::MemGc,
            },
            _ => unreachable!("from_cli_memory called with a non-matching command"),
        })
    }

    /// Routing family for dispatch. Exhaustive: a new variant must be
    /// classified here before the family dispatchers can route it.
    fn family(&self) -> Family {
        match self {
            CommandRequest::Build { .. }
            | CommandRequest::Doctor { .. }
            | CommandRequest::Usage { .. } => Family::Diag,
            CommandRequest::Callers { .. }
            | CommandRequest::Callees { .. }
            | CommandRequest::Context { .. }
            | CommandRequest::Implementers { .. }
            | CommandRequest::Imports { .. }
            | CommandRequest::Path { .. }
            | CommandRequest::Impact { .. } => Family::Graph,
            CommandRequest::Dead { .. }
            | CommandRequest::Unresolved { .. }
            | CommandRequest::Symbols { .. }
            | CommandRequest::Complexity { .. }
            | CommandRequest::Resolve { .. }
            | CommandRequest::Files { .. }
            | CommandRequest::Search { .. }
            | CommandRequest::Stats { .. } => Family::Analysis,
            CommandRequest::MemWrite { .. }
            | CommandRequest::MemQuery { .. }
            | CommandRequest::MemGet { .. }
            | CommandRequest::MemDelete { .. }
            | CommandRequest::MemGc
            | CommandRequest::MemExport { .. } => Family::Memory,
        }
    }

    /// Tool name as journaled and keyed by the MCP tool registry.
    pub fn tool_name(&self) -> &'static str {
        match self {
            CommandRequest::Build { .. } => "build",
            CommandRequest::Callers { .. } => "callers",
            CommandRequest::Callees { .. } => "callees",
            CommandRequest::Context { .. } => "context",
            CommandRequest::Implementers { .. } => "implementers",
            CommandRequest::Imports { .. } => "imports",
            CommandRequest::Path { .. } => "path",
            CommandRequest::Impact { .. } => "impact",
            CommandRequest::Dead { .. } => "dead",
            CommandRequest::Unresolved { .. } => "unresolved",
            CommandRequest::Symbols { .. } => "symbols",
            CommandRequest::Complexity { .. } => "complexity",
            CommandRequest::Resolve { .. } => "resolve",
            CommandRequest::Files { .. } => "files",
            CommandRequest::Search { .. } => "search",
            CommandRequest::Stats { .. } => "stats",
            CommandRequest::Doctor { .. } => "doctor",
            CommandRequest::Usage { .. } => "usage",
            CommandRequest::MemWrite { .. } => "mem_write",
            CommandRequest::MemQuery { .. } => "mem_query",
            CommandRequest::MemGet { .. } => "mem_get",
            CommandRequest::MemDelete { .. } => "mem_delete",
            CommandRequest::MemGc => "mem_gc",
            CommandRequest::MemExport { .. } => "mem_export",
        }
    }

    /// Journal argument object, identical on both surfaces (one source of
    /// truth replaces the former per-surface argument tables).
    pub fn journal_args(&self) -> Vec<(&'static str, String)> {
        match self.family() {
            Family::Diag => self.journal_args_diag(),
            Family::Graph => self.journal_args_graph(),
            Family::Analysis => self.journal_args_analysis(),
            Family::Memory => self.journal_args_memory(),
        }
    }

    /// Journal fields for build, doctor, and usage.
    fn journal_args_diag(&self) -> Vec<(&'static str, String)> {
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
            CommandRequest::Usage { quality } => vec![("quality", quality.to_string())],
            _ => unreachable!("journal_args_diag called with a non-matching request"),
        }
    }
    /// Journal fields for the graph queries.
    fn journal_args_graph(&self) -> Vec<(&'static str, String)> {
        match self {
            CommandRequest::Callers {
                name,
                limit,
                depth,
                explain,
            }
            | CommandRequest::Callees {
                name,
                limit,
                depth,
                explain,
            } => graph_args(name, Some(*depth), *limit, *explain),
            CommandRequest::Context {
                name,
                limit,
                explain,
            } => graph_args(name, None, *limit, *explain),
            CommandRequest::Implementers { name, limit } => {
                vec![("name", name.clone()), ("limit", limit.to_string())]
            }
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
                context,
                explain,
                limit,
            } => vec![
                ("file", file.clone()),
                ("sites", sites.to_string()),
                ("depth", depth.to_string()),
                ("edges", edge_kinds_csv(edges)),
                ("context", context.to_string()),
                ("explain", explain.to_string()),
                ("limit", limit.to_string()),
            ],
            _ => unreachable!("journal_args_graph called with a non-matching request"),
        }
    }
    /// Journal fields for analysis and inventory.
    fn journal_args_analysis(&self) -> Vec<(&'static str, String)> {
        match self {
            CommandRequest::Dead {
                exclude_tests,
                path,
                limit,
            } => path_limit_args(
                &[("exclude_tests", exclude_tests.to_string())],
                path.as_ref(),
                *limit,
            ),
            CommandRequest::Unresolved { names, path, limit } => {
                path_limit_args(&[("names", names.to_string())], path.as_ref(), *limit)
            }
            CommandRequest::Symbols {
                file,
                outline,
                metrics,
                limit,
            } => vec![
                ("file", file.clone()),
                ("outline", outline.to_string()),
                ("metrics", metrics.to_string()),
                ("limit", limit.to_string()),
            ],
            CommandRequest::Complexity {
                min_loc,
                path,
                limit,
            } => path_limit_args(&[("min_loc", min_loc.to_string())], path.as_ref(), *limit),
            CommandRequest::Resolve { names } => vec![
                ("count", names.len().to_string()),
                (
                    "candidate_limit",
                    crate::query::MAX_AMBIGUOUS_CANDIDATES.to_string(),
                ),
            ],
            CommandRequest::Files { path, limit } => path_limit_args(&[], path.as_ref(), *limit),
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
    /// — never the body text (journal privacy contract).
    fn journal_args_memory(&self) -> Vec<(&'static str, String)> {
        match self {
            CommandRequest::MemWrite {
                key,
                body,
                kind,
                scope,
                ttl,
                supersedes,
                ..
            } => vec![
                ("key", key.clone()),
                ("kind", kind.clone()),
                ("scope", scope.clone()),
                ("ttl", ttl.map_or_else(String::new, |t| t.to_string())),
                ("supersedes", supersedes.clone().unwrap_or_default()),
                ("body_len", body.len().to_string()),
            ],
            CommandRequest::MemQuery {
                terms,
                kind,
                scope,
                limit,
                include_superseded,
                ..
            } => vec![
                ("terms", terms.join(" ")),
                ("kind", kind.clone().unwrap_or_default()),
                ("scope", scope.clone().unwrap_or_default()),
                ("limit", limit.to_string()),
                ("include_superseded", include_superseded.to_string()),
            ],
            CommandRequest::MemGet { key } => vec![("key", key.clone())],
            CommandRequest::MemDelete { key } => vec![("key", key.clone())],
            CommandRequest::MemGc => vec![],
            CommandRequest::MemExport { limit } => {
                vec![("limit", limit.map_or_else(String::new, |n| n.to_string()))]
            }
            _ => unreachable!("journal_args_memory called with a non-matching request"),
        }
    }
}

/// Rendered command outcome: everything the caller prints, plus the exit code.
pub struct ExecutionResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Journal one QueryOutput and render it exactly as the CLI prints it:
/// stdout keeps the header, pointers, and cap line; the cap notice goes to
/// stderr.
fn emit(out: crate::query::QueryOutput) -> Result<ArmOut> {
    let ambiguous = out.pointers.iter().filter(|p| !p.tag.is_empty()).count();
    let stderr = crate::query::journal_output(&out, ambiguous);
    Ok((crate::query::render_output(&out), stderr))
}

/// Join report lines into stdout bytes (one trailing newline per line).
fn join_lines(lines: Vec<String>) -> String {
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    text
}

/// Dispatch one command. Journaling (begin/record/finalize) is owned here so
/// both surfaces produce the same `.aw/usage.log` line. On the error path the
/// caveat and ambiguous candidates land on stdout (the dead caveat contract
/// and CUJ4) and the diagnostic on stderr.
pub fn execute(
    req: &CommandRequest,
    root_arg: Option<&Path>,
    surface: Surface,
    journal: bool,
) -> ExecutionResult {
    if journal {
        crate::journal::begin(surface.as_str(), req.tool_name(), &req.journal_args());
    }
    let started = Instant::now();
    let outcome = run(req, root_arg, surface);
    let latency_us = started.elapsed().as_micros() as u64;
    let root = crate::walk::discover_root(root_arg).ok();
    let (stdout, stderr, exit_code) = match outcome {
        Ok((stdout, stderr)) => {
            if journal {
                // record_result for the query shapes ran inside the arm; the
                // exit is 0 here.
                crate::journal::finalize(root.as_deref(), 0, latency_us);
            }
            (stdout, stderr, 0)
        }
        Err(AwError::BatchOutput { text, code }) => {
            // Batch output still journals: `resolve --stdin` exits 1/2 through
            // this arm, and `record_result` only stages the pending line.
            if journal {
                crate::journal::finalize(root.as_deref(), code, latency_us);
            }
            (text, String::new(), code)
        }
        Err(e) => {
            let mut stdout = String::new();
            if let AwError::NoResultsCaveated { caveat, .. } = &e {
                stdout.push_str(caveat);
                stdout.push('\n');
            }
            if let AwError::Ambiguous { candidates, .. } = &e {
                for (path, line, qualname) in candidates {
                    stdout.push_str(&format!("{path}:{line} {qualname}\n"));
                }
            }
            let stderr = format!("{e}\n");
            if journal {
                crate::journal::finalize(root.as_deref(), e.exit_code(), latency_us);
            }
            (stdout, stderr, e.exit_code())
        }
    };
    ExecutionResult {
        stdout,
        stderr,
        exit_code,
    }
}

/// Arm output: stdout bytes plus any stderr note the arm itself produced
/// (query cap notices are emitted here; diagnostics come from the error path).
type ArmOut = (String, String);

fn run(req: &CommandRequest, root_arg: Option<&Path>, surface: Surface) -> Result<ArmOut> {
    match req.family() {
        Family::Diag => run_diag(req, root_arg),
        Family::Graph | Family::Analysis => run_query(req, root_arg),
        Family::Memory => run_memory(req, root_arg, surface),
    }
}

/// Build, doctor, and usage (the CLI's MCP hand-off is handled before this).
fn run_diag(req: &CommandRequest, root_arg: Option<&Path>) -> Result<ArmOut> {
    match req {
        CommandRequest::Build {
            force,
            max_parse_error_rate,
        } => {
            let line = crate::build::run_build(root_arg, *force, *max_parse_error_rate)?;
            Ok((format!("{line}\n"), String::new()))
        }
        CommandRequest::Doctor { invalidate } => run_doctor(root_arg, *invalidate),
        CommandRequest::Usage { quality } => run_usage(root_arg, *quality),
        _ => unreachable!("run_diag called with a non-diagnostic request"),
    }
}

/// Index-only query arms: graph traversal, analysis, and inventory.
fn run_query(req: &CommandRequest, root_arg: Option<&Path>) -> Result<ArmOut> {
    match req.family() {
        Family::Graph => run_graph_query(req, root_arg),
        Family::Analysis => run_analysis_query(req, root_arg),
        Family::Diag | Family::Memory => unreachable!("run_query called with a non-query request"),
    }
}

/// Graph traversal and call-site queries.
fn run_graph_query(req: &CommandRequest, root_arg: Option<&Path>) -> Result<ArmOut> {
    let (root, conn) = open_query(root_arg)?;
    let ctx = crate::query::QueryCtx {
        conn: &conn,
        root: &root,
    };
    match req {
        CommandRequest::Callers { .. } | CommandRequest::Callees { .. } => run_traversal(req, &ctx),
        CommandRequest::Context {
            name,
            limit,
            explain,
        } => {
            let lines = crate::query::context(&ctx, name, *limit, *explain)?;
            Ok((join_lines(lines), String::new()))
        }
        CommandRequest::Implementers { name, limit } => {
            emit(crate::query::implementers(&ctx, name, *limit)?)
        }
        CommandRequest::Imports { file, limit } => emit(crate::query::imports(&ctx, file, *limit)?),
        CommandRequest::Path { from, to, depth } => {
            let lines = crate::query::path(&ctx, from, to, *depth)?;
            Ok((join_lines(lines), String::new()))
        }
        CommandRequest::Impact { .. } => run_impact(req, &ctx),
        _ => unreachable!("run_graph_query called with a non-graph request"),
    }
}

/// `callers`/`callees`: one name plus the shared limit/depth/explain bundle.
fn run_traversal(req: &CommandRequest, ctx: &crate::query::QueryCtx<'_>) -> Result<ArmOut> {
    match req {
        CommandRequest::Callers {
            name,
            limit,
            depth,
            explain,
        } => emit(crate::query::callers(
            ctx,
            name,
            graph_opts(*limit, *depth, *explain),
        )?),
        CommandRequest::Callees {
            name,
            limit,
            depth,
            explain,
        } => emit(crate::query::callees(
            ctx,
            name,
            graph_opts(*limit, *depth, *explain),
        )?),
        _ => unreachable!("run_traversal called with a non-traversal request"),
    }
}

/// Shared graph-traversal knobs for `callers`/`callees` (ImpactOpts-style).
fn graph_opts(limit: usize, depth: usize, explain: bool) -> crate::query::GraphQueryOpts {
    crate::query::GraphQueryOpts {
        limit,
        depth,
        explain,
    }
}

/// `impact`: one file plus the extended ImpactOpts bundle.
fn run_impact(req: &CommandRequest, ctx: &crate::query::QueryCtx<'_>) -> Result<ArmOut> {
    let CommandRequest::Impact {
        file,
        sites,
        depth,
        edges,
        context,
        explain,
        limit,
    } = req
    else {
        unreachable!("run_impact called with a non-impact request")
    };
    let out = crate::query::impact(
        ctx,
        file,
        *limit,
        crate::query::ImpactOpts {
            sites: *sites,
            depth: *depth,
            kinds: edges.clone(),
            context: *context,
            explain: *explain,
        },
    )?;
    emit(out)
}

/// Inventory, analysis, and batch-resolution queries.
fn run_analysis_query(req: &CommandRequest, root_arg: Option<&Path>) -> Result<ArmOut> {
    let (root, conn) = open_query(root_arg)?;
    let ctx = crate::query::QueryCtx {
        conn: &conn,
        root: &root,
    };
    match req {
        CommandRequest::Dead {
            exclude_tests,
            path,
            limit,
        } => {
            let out = crate::query::dead(&ctx, *limit, *exclude_tests, path.as_deref())?;
            emit(out)
        }
        CommandRequest::Unresolved { .. }
        | CommandRequest::Symbols { .. }
        | CommandRequest::Complexity { .. }
        | CommandRequest::Files { .. }
        | CommandRequest::Search { .. } => run_line_query(req, &ctx),
        CommandRequest::Resolve { names } => run_resolve_batch(&conn, names),
        CommandRequest::Stats { path } => {
            let line = crate::insights::stats(&ctx, path.as_deref())?;
            crate::journal::record_result(crate::journal::OutcomeCounts::uniform(1), false, 0);
            Ok((format!("{line}\n"), String::new()))
        }
        _ => unreachable!("run_analysis_query called with a non-analysis request"),
    }
}

/// Analysis arms that render plain lines (no QueryOutput journaling). The
/// stderr half carries the queries' cap notices (the single cap-notice
/// channel).
fn run_line_query(req: &CommandRequest, ctx: &crate::query::QueryCtx<'_>) -> Result<ArmOut> {
    let (lines, stderr) = match req {
        CommandRequest::Unresolved { names, path, limit } => {
            if *names {
                crate::query::unresolved_names(ctx, path.as_deref(), *limit)?
            } else {
                crate::query::unresolved(ctx.conn, path.as_deref(), *limit)?
            }
        }
        CommandRequest::Symbols {
            file,
            outline,
            metrics,
            limit,
        } => crate::query::symbols(
            ctx,
            file,
            *limit,
            crate::query::SymbolsOpts {
                outline: *outline,
                metrics: *metrics,
            },
        )?,
        CommandRequest::Complexity {
            min_loc,
            path,
            limit,
        } => crate::query::complexity(ctx, *min_loc, path.as_deref(), *limit)?,
        CommandRequest::Files { path, limit } => {
            crate::query::files(ctx.conn, path.as_deref(), *limit)?
        }
        CommandRequest::Search { terms, limit } => crate::query::search(ctx, terms, *limit)?,
        _ => unreachable!("run_line_query called with a non-line request"),
    };
    Ok((join_lines(lines), stderr))
}

/// Batch `resolve`: journal the staged result, then exit 1/2 through the
/// BatchOutput arm when a name is ambiguous or missing.
fn run_resolve_batch(conn: &rusqlite::Connection, names: &[String]) -> Result<ArmOut> {
    let (lines, exit) = crate::query::resolve_names(conn, names)?;
    crate::journal::record_result(
        crate::journal::OutcomeCounts::uniform(lines.len()),
        false,
        0,
    );
    if exit != 0 {
        return Err(AwError::BatchOutput {
            text: join_lines(lines),
            code: exit,
        });
    }
    Ok((join_lines(lines), String::new()))
}

/// Memory-store dispatcher. Each command owns its request shape in a small
/// helper; the outer match only selects the command.
fn run_memory(req: &CommandRequest, root_arg: Option<&Path>, surface: Surface) -> Result<ArmOut> {
    match req {
        CommandRequest::MemWrite { .. } => run_mem_write(req, root_arg, surface),
        CommandRequest::MemQuery { .. } => run_mem_query(req, root_arg),
        CommandRequest::MemGet { .. } => run_mem_get(req, root_arg),
        CommandRequest::MemDelete { .. } => run_mem_delete(req, root_arg),
        CommandRequest::MemGc => run_mem_gc(root_arg),
        CommandRequest::MemExport { .. } => run_mem_export(req, root_arg),
        _ => unreachable!("run_memory called with a non-memory request"),
    }
}

fn run_mem_gc(root_arg: Option<&Path>) -> Result<ArmOut> {
    let (_root, mut conn) = open_memory(root_arg, MemoryMode::Mutation)?;
    let (expired, superseded, durable) = crate::db::memory_gc(&mut conn, crate::db::unix_now())?;
    crate::journal::record_result(crate::journal::OutcomeCounts::uniform(1), false, 0);
    Ok((
        format!(
            "{}\nexpired={expired} removed, superseded={superseded} removed, durable={durable} rows\n",
            crate::memory::MEMORY_CAVEAT
        ),
        String::new(),
    ))
}

/// Persist one note (validation, normalization, and the write outcome line).
fn run_mem_write(
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
    let outcome = write_note(root_arg, &note, surface, supersedes.as_deref())?;
    crate::journal::record_result(crate::journal::OutcomeCounts::uniform(1), false, 0);
    let line = outcome_line(&outcome, &note.key);
    Ok((
        format!("{}\n{line}\n", crate::memory::MEMORY_CAVEAT),
        String::new(),
    ))
}

fn write_note(
    root_arg: Option<&Path>,
    note: &crate::memory::NormalizedNote,
    surface: Surface,
    supersedes: Option<&str>,
) -> Result<crate::db::MemWriteOutcome> {
    let (_root, mut conn) = open_memory(root_arg, MemoryMode::Store)?;
    let session = crate::journal::session_tag();
    let now = crate::db::unix_now();
    crate::db::memory_write(
        &mut conn,
        &crate::db::MemNoteInput {
            key: &note.key,
            kind: &note.kind,
            tags: &note.tags,
            refs: &note.refs,
            scope: &note.scope,
            session: &session,
            body: &note.body,
            source: surface.as_str(),
            expires_at: if note.ttl_secs == 0 {
                0
            } else {
                now + note.ttl_secs
            },
            supersedes,
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
    }
}

/// Ranked search or the explicit `--all` listing.
fn run_mem_query(req: &CommandRequest, root_arg: Option<&Path>) -> Result<ArmOut> {
    let CommandRequest::MemQuery {
        terms,
        all,
        kind,
        scope,
        limit,
        include_superseded,
    } = req
    else {
        unreachable!("run_mem_query called with a non-query request")
    };
    let limit = (*limit).clamp(1, crate::memory::MAX_MEM_LIMIT);
    validate_mem_query(terms, *all, *include_superseded)?;
    let (_root, conn) = open_memory(root_arg, MemoryMode::Query)?;
    let filter = crate::db::MemFilter {
        kind: kind.as_deref(),
        scope: scope.as_deref(),
        now: crate::db::unix_now(),
        include_superseded: *include_superseded,
    };
    let (rendered, total, shown) = if *all {
        let rows = crate::db::memory_current_rows(&conn, &filter)?;
        if rows.is_empty() {
            return Err(mem_empty(if *include_superseded {
                "memory store has no notes"
            } else {
                "memory store has no current notes"
            }));
        }
        // `--all` order is pure recency (created_at desc), key as tie-break;
        // the SQL already sorts this way.
        render_rows(
            rows.iter().map(|row| {
                let tag = if row.superseded_by.is_some() {
                    " [superseded]"
                } else {
                    ""
                };
                (row, tag)
            }),
            limit,
        )
    } else {
        let joined = terms.join(" ");
        let candidates = mem_ranked(&conn, &joined, &filter)?;
        if candidates.is_empty() {
            return Err(mem_empty(&format!("no memory matches '{joined}'")));
        }
        render_rows(candidates.iter().map(|cand| (&cand.row, "")), limit)
    };
    Ok(render_mem_list(&rendered, total, shown))
}

/// Reject the flag combinations the two query modes cannot serve.
fn validate_mem_query(terms: &[String], all: bool, include_superseded: bool) -> Result<()> {
    if all && !terms.is_empty() {
        return Err(AwError::Usage(
            "mem query: --all and terms are mutually exclusive".to_string(),
        ));
    }
    if !all && terms.iter().all(|t| t.trim().is_empty()) {
        return Err(AwError::Usage(
            "mem query: provide terms or --all".to_string(),
        ));
    }
    // Superseded rows participate in the explicit listing only; the
    // ranked resolver keeps supersession-at-result-time exclusion.
    if include_superseded && !all {
        return Err(AwError::Usage(
            "mem query: --include-superseded requires --all".to_string(),
        ));
    }
    Ok(())
}

/// Shared row rendering for both mem query modes: the first `limit` rows in
/// input order, each with its tag; returns (rendered, total, shown).
fn render_rows<'a>(
    rows: impl ExactSizeIterator<Item = (&'a crate::db::MemRow, &'a str)>,
    limit: usize,
) -> (Vec<String>, usize, usize) {
    let total = rows.len();
    let shown = total.min(limit);
    let rendered = rows
        .take(shown)
        .map(|(row, tag)| crate::memory::render_row_tagged(row, tag))
        .collect();
    (rendered, total, shown)
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
        stdout.push_str(&crate::memory::render_showing(shown, total));
        stdout.push('\n');
    }
    crate::journal::record_result(crate::journal::OutcomeCounts::of(shown, total), false, 0);
    (stdout, crate::query::cap_stderr(shown, total))
}

/// Current row for one exact key plus its revision count.
fn run_mem_get(req: &CommandRequest, root_arg: Option<&Path>) -> Result<ArmOut> {
    let CommandRequest::MemGet { key } = req else {
        unreachable!("run_mem_get called with a non-get request")
    };
    let (_root, conn) = open_memory(root_arg, MemoryMode::Query)?;
    match crate::db::memory_get(&conn, key)? {
        None => Err(mem_unknown_key(key)),
        Some((row, revs)) => {
            crate::journal::record_result(crate::journal::OutcomeCounts::uniform(1), false, 0);
            Ok((
                format!(
                    "{}\n{}\nrevs: {}\n",
                    crate::memory::MEMORY_CAVEAT,
                    crate::memory::render_row_tagged(&row, ""),
                    revs
                ),
                String::new(),
            ))
        }
    }
}

/// Hard-delete every revision of one key.
fn run_mem_delete(req: &CommandRequest, root_arg: Option<&Path>) -> Result<ArmOut> {
    let CommandRequest::MemDelete { key } = req else {
        unreachable!("run_mem_delete called with a non-delete request")
    };
    let (_root, mut conn) = open_memory(root_arg, MemoryMode::Mutation)?;
    match crate::db::memory_delete(&mut conn, key)? {
        None => Err(mem_unknown_key(key)),
        Some(revs) => {
            crate::journal::record_result(
                crate::journal::OutcomeCounts::uniform(revs.max(0) as usize),
                false,
                0,
            );
            Ok((
                format!(
                    "{}\ndeleted '{key}' revs={revs}\n",
                    crate::memory::MEMORY_CAVEAT
                ),
                String::new(),
            ))
        }
    }
}

/// Current notes as JSONL on stdout; the caveat goes to stderr.
fn run_mem_export(req: &CommandRequest, root_arg: Option<&Path>) -> Result<ArmOut> {
    let CommandRequest::MemExport { limit } = req else {
        unreachable!("run_mem_export called with a non-export request")
    };
    let (_root, conn) = open_memory(root_arg, MemoryMode::Query)?;
    let rows = crate::db::memory_export_rows(&conn, *limit)?;
    if rows.is_empty() {
        return Err(AwError::NoResults(
            "memory store has no current notes to export".to_string(),
        ));
    }
    crate::journal::record_result(crate::journal::OutcomeCounts::uniform(rows.len()), false, 0);
    // stdout carries ONLY the JSONL; the caveat goes to stderr so
    // restore pipelines parse stdout directly (stream contract).
    let mut stdout = String::new();
    for row in &rows {
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
        stdout.push_str(&line.to_string());
        stdout.push('\n');
    }
    Ok((stdout, format!("{}\n", crate::memory::MEMORY_CAVEAT)))
}

fn run_doctor(root_arg: Option<&Path>, invalidate: bool) -> Result<ArmOut> {
    let root = crate::walk::discover_root(root_arg)?;
    let lines = if invalidate {
        crate::doctor::invalidate(&root)?
    } else {
        crate::doctor::report(&root)?
    };
    Ok((join_lines(lines), String::new()))
}

fn run_usage(root_arg: Option<&Path>, quality: bool) -> Result<ArmOut> {
    let root = crate::walk::discover_root(root_arg)?;
    let lines = crate::journal::summary(&root, quality)?;
    Ok((join_lines(lines), String::new()))
}

/// Caveated empty-result error shared by the mem read commands (stdout keeps
/// the caveat line, stderr the diagnostic, exit 1).
fn mem_empty(detail: &str) -> AwError {
    AwError::NoResultsCaveated {
        caveat: crate::memory::MEMORY_CAVEAT,
        detail: detail.to_string(),
    }
}

/// The mem_empty error for a key with no current row: shared by mem get and
/// mem delete so the message cannot drift.
fn mem_unknown_key(key: &str) -> AwError {
    mem_empty(&format!("unknown memory key '{key}'"))
}

/// The resolver: FTS candidates capped at CANDIDATE_CAP, ref/tag token
/// overlap, recency decay, fixed-weight fusion (D4-A). The fetch stays here;
/// the filter/signal work lives in `mem_candidate`, so this function only
/// merges through `memory::fuse`. Ranked mode never lists superseded rows
/// (`validate_mem_query` pins `include_superseded` to `--all`), so the filter
/// rides through unchanged.
fn mem_ranked(
    conn: &rusqlite::Connection,
    joined_terms: &str,
    filter: &crate::db::MemFilter<'_>,
) -> Result<Vec<crate::memory::Candidate>> {
    let query = crate::memory::mem_query_terms(joined_terms);
    let hits = crate::db::memory_fts_candidates(conn, &query, crate::memory::CANDIDATE_CAP)?;
    if hits.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i64> = hits.iter().map(|(id, _)| *id).collect();
    let by_id: std::collections::HashMap<i64, crate::db::MemRow> =
        crate::db::memory_rows_by_ids(conn, &ids)?
            .into_iter()
            .map(|row| (row.id, row))
            .collect();
    let query_tokens: std::collections::BTreeSet<String> = crate::memory::mem_tokens(joined_terms)
        .into_iter()
        .collect();
    let candidates = hits
        .into_iter()
        .filter_map(|(id, lex_raw)| mem_candidate(by_id.get(&id)?, lex_raw, &query_tokens, filter))
        .collect();
    Ok(crate::memory::fuse(candidates))
}

/// One ranked candidate: the current-row filter (same rules as the `--all`
/// SQL filter via `db::memory_row_is_current`) plus the ref/tag overlap and
/// recency signals. `None` when the row is not current or its FTS entry
/// outlived the row.
fn mem_candidate(
    row: &crate::db::MemRow,
    lex_raw: f64,
    query_tokens: &std::collections::BTreeSet<String>,
    filter: &crate::db::MemFilter<'_>,
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
        age_days,
        score: 0.0,
    })
}

/// Resolve the repo root and open the index read-only (the shared query gate:
/// missing -> 1, corrupt -> 3, skew -> 5).
fn open_query(root_arg: Option<&Path>) -> Result<(std::path::PathBuf, rusqlite::Connection)> {
    let root = crate::walk::discover_root(root_arg)?;
    let conn = crate::db::open_for_query(&root)?;
    Ok((root, conn))
}

/// The three memory-store open gates, in the shared `open_query` shape.
enum MemoryMode {
    /// Read-only (missing -> 1, corrupt -> 3, skew -> 5).
    Query,
    /// Read-write mutation without creating a store (missing -> 1).
    Mutation,
    /// Read-write with first-use creation.
    Store,
}

/// Resolve the repo root and open the memory store in `mode`, mirroring
/// `open_query` for the mem commands.
fn open_memory(
    root_arg: Option<&Path>,
    mode: MemoryMode,
) -> Result<(std::path::PathBuf, rusqlite::Connection)> {
    let root = crate::walk::discover_root(root_arg)?;
    let conn = match mode {
        MemoryMode::Query => crate::db::open_memory_query(&root)?,
        MemoryMode::Mutation => crate::db::open_memory_mutation(&root)?,
        MemoryMode::Store => crate::db::open_memory_store(&root)?,
    };
    Ok((root, conn))
}

/// Shared bounds for one `resolve` name list (CLI stdin and MCP names array):
/// at most 1000 names, 4096 bytes per name, 1 MiB total. One copy keeps the
/// two surfaces from drifting; `empty` is the surface-specific error for a
/// list with no usable names (blank entries and `#` comments never count).
fn validate_resolve_names(names: &[String], empty: &str) -> Result<()> {
    if names.len() > RESOLVE_MAX_NAMES {
        return Err(AwError::Usage(format!(
            "resolve accepts at most {RESOLVE_MAX_NAMES} names"
        )));
    }
    if names.iter().any(|n| n.len() > RESOLVE_MAX_NAME_BYTES) {
        return Err(AwError::Usage(format!(
            "resolve name exceeds {RESOLVE_MAX_NAME_BYTES} bytes"
        )));
    }
    if names.iter().map(|n| n.len()).sum::<usize>() > RESOLVE_MAX_TOTAL_BYTES {
        return Err(AwError::Usage(format!(
            "resolve names exceed {} MiB total",
            RESOLVE_MAX_TOTAL_BYTES / (1024 * 1024)
        )));
    }
    if names
        .iter()
        .all(|n| n.trim().is_empty() || n.trim().starts_with('#'))
    {
        return Err(AwError::NoResults(empty.to_string()));
    }
    Ok(())
}

/// Read and validate `resolve --stdin` input (CLI-only input shape; the MCP
/// surface passes names as a JSON array). Bounds mirror the MCP dispatch via
/// `validate_resolve_names`: 1 MiB total input, 4096 bytes per line, 1000
/// retained names, `#` comments.
pub fn read_resolve_stdin() -> Result<Vec<String>> {
    let mut input = String::new();
    std::io::stdin()
        .take(RESOLVE_MAX_TOTAL_BYTES as u64 + 1)
        .read_to_string(&mut input)?;
    if input.len() > RESOLVE_MAX_TOTAL_BYTES {
        return Err(AwError::Usage(format!(
            "resolve input exceeds {} MiB",
            RESOLVE_MAX_TOTAL_BYTES / (1024 * 1024)
        )));
    }
    let mut names = Vec::new();
    for line in input.lines() {
        if line.len() > RESOLVE_MAX_NAME_BYTES {
            return Err(AwError::Usage(format!(
                "resolve line exceeds {RESOLVE_MAX_NAME_BYTES} bytes"
            )));
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            names.push(trimmed.to_string());
        }
    }
    validate_resolve_names(&names, "no names on stdin")?;
    Ok(names)
}

// ---------------------------------------------------------------------------
// MCP tool registry: the single declarative table both the JSON Schema
// renderer and the required-argument validator read.
// ---------------------------------------------------------------------------

/// JSON type of one MCP tool argument.
pub enum ParamKind {
    Str,
    Int {
        min: i64,
        max: Option<i64>,
    },
    Float {
        min: f64,
        max: f64,
    },
    Bool,
    /// Array of strings, at most 1000 entries (the `resolve` names input).
    StrArray,
}

pub struct ParamSpec {
    pub name: &'static str,
    pub kind: ParamKind,
    pub required: bool,
    pub description: &'static str,
}

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub params: &'static [ParamSpec],
    /// Default `aw mcp` surface (without `--all-tools`): the eight core
    /// navigation tools plus mem_write/mem_query (OQ1 owner decision,
    /// 2026-11-05). One source of truth — a spec without this flag stays
    /// behind `--all-tools` and cannot silently drop off the default set.
    pub core: bool,
}

const LIMIT_PARAM: ParamSpec = ParamSpec {
    name: "limit",
    kind: ParamKind::Int {
        min: 1,
        max: Some(10_000),
    },
    required: false,
    description: "Maximum results shown (default 50)",
};

const DEPTH_PARAM: ParamSpec = ParamSpec {
    name: "depth",
    kind: ParamKind::Int {
        min: 1,
        max: Some(10),
    },
    required: false,
    description: "Transitive depth (default 1)",
};

const EXPLAIN_PARAM: ParamSpec = ParamSpec {
    name: "explain",
    kind: ParamKind::Bool,
    required: false,
    description: "Append edge kind, confidence, site, and destination metadata",
};

const PATH_PARAM: ParamSpec = ParamSpec {
    name: "path",
    kind: ParamKind::Str,
    required: false,
    description: "Restrict to a repo-relative file or directory prefix",
};

const NAME_PARAM: ParamSpec = ParamSpec {
    name: "name",
    kind: ParamKind::Str,
    required: true,
    description: "Symbol name or dotted qualname",
};

const FILE_PARAM: ParamSpec = ParamSpec {
    name: "file",
    kind: ParamKind::Str,
    required: true,
    description: "Repo-relative path of the indexed file",
};

const KEY_PARAM: ParamSpec = ParamSpec {
    name: "key",
    kind: ParamKind::Str,
    required: true,
    description: "The note's stable identity",
};

pub const TOOL_SPECS: &[ToolSpec] = &[
    ToolSpec {
        name: "build",
        description: "Index the repository into .aw/index.db (full atomic replacement)",
        params: &[
            ParamSpec {
                name: "force",
                kind: ParamKind::Bool,
                required: false,
                description: "Re-extract everything (disable the fast path) and bypass the parse-error abort",
            },
            ParamSpec {
                name: "max_parse_error_rate",
                kind: ParamKind::Float {
                    min: 0.0,
                    max: 1.0,
                },
                required: false,
                description: "Abort above this fraction of parse-error files (default 0.05)",
            },
        ],
        core: true,
    },
    ToolSpec {
        name: "callers",
        description: "Who calls this symbol — direct by default; transitive with depth; pointer list path:line",
        params: &[
            NAME_PARAM,
            DEPTH_PARAM,
            LIMIT_PARAM,
            EXPLAIN_PARAM,
        ],
        core: true,
    },
    ToolSpec {
        name: "callees",
        description: "What does this symbol call — transitive with depth",
        params: &[
            NAME_PARAM,
            DEPTH_PARAM,
            LIMIT_PARAM,
            EXPLAIN_PARAM,
        ],
        core: true,
    },
    ToolSpec {
        name: "path",
        description: "Shortest call path between two symbols — hop call-site pointers path:line over resolved edges; dynamic dispatch may hide paths",
        params: &[
            ParamSpec { name: "from", kind: ParamKind::Str, required: true, description: "Symbol the path starts from" },
            ParamSpec { name: "to", kind: ParamKind::Str, required: true, description: "Symbol the path must reach" },
            ParamSpec { name: "depth", kind: ParamKind::Int { min: 1, max: Some(10) }, required: false, description: "Maximum hops (default 10)" },
        ],
        core: true,
    },
    ToolSpec {
        name: "context",
        description: "One-call symbol context: definition + signature, direct callers, direct callees over resolved edges plus tagged ambiguous candidates",
        params: &[
            NAME_PARAM,
            ParamSpec { name: "limit", kind: ParamKind::Int { min: 1, max: Some(50) }, required: false, description: "Callers/callees shown per section (default 10)" },
            EXPLAIN_PARAM,
        ],
        core: true,
    },
    ToolSpec {
        name: "implementers",
        description: "Who inherits from or implements this symbol — resolved inherits edges only; cross-file impls omitted",
        params: &[
            NAME_PARAM,
            LIMIT_PARAM,
        ],
        core: false,
    },
    ToolSpec {
        name: "imports",
        description: "What does this file import — outbound resolved imports; external/unresolved omitted",
        params: &[
            FILE_PARAM,
            LIMIT_PARAM,
        ],
        core: true,
    },
    ToolSpec {
        name: "impact",
        description: "Files that call into <file> — one row per caller file with a site count; an empty calls-only result names import/inherit counts; sites=true lists every call site; edges/depth extend the dependency set",
        params: &[
            FILE_PARAM,
            ParamSpec { name: "sites", kind: ParamKind::Bool, required: false, description: "List every call site instead of one row per file" },
            DEPTH_PARAM,
            ParamSpec { name: "edges", kind: ParamKind::Str, required: false, description: "Edge kinds: calls,imports,inherits,all (default calls)" },
            ParamSpec { name: "context", kind: ParamKind::Bool, required: false, description: "Append the enclosing symbol to each pointer" },
            EXPLAIN_PARAM,
            LIMIT_PARAM,
        ],
        core: true,
    },
    ToolSpec {
        name: "dead",
        description: "Dead-code candidates (no resolved inbound calls/imports/inherits) — candidates, never proof; exclude_tests and path scope filters",
        params: &[
            ParamSpec { name: "exclude_tests", kind: ParamKind::Bool, required: false, description: "Exclude tests/ paths and test_* symbols" },
            PATH_PARAM,
            LIMIT_PARAM,
        ],
        core: false,
    },
    ToolSpec {
        name: "unresolved",
        description: "Per-file unresolved-callee counts, or names=true for path:line kind name sites",
        params: &[
            ParamSpec { name: "names", kind: ParamKind::Bool, required: false, description: "List unresolved sites as path:line kind name" },
            PATH_PARAM,
            LIMIT_PARAM,
        ],
        core: false,
    },
    ToolSpec {
        name: "symbols",
        description: "Symbols of one indexed file or directory, ordered by line; outline adds signatures, metrics adds the effective line count",
        params: &[
            ParamSpec { name: "file", kind: ParamKind::Str, required: true, description: "Repo-relative path of the indexed file or directory" },
            ParamSpec { name: "outline", kind: ParamKind::Bool, required: false, description: "Append the one-line signature column" },
            ParamSpec { name: "metrics", kind: ParamKind::Bool, required: false, description: "Append the per-symbol effective line count" },
            LIMIT_PARAM,
        ],
        core: true,
    },
    ToolSpec {
        name: "complexity",
        description: "Per-symbol complexity ranking: effective LOC",
        params: &[
            ParamSpec { name: "min_loc", kind: ParamKind::Int { min: 0, max: None }, required: false, description: "Minimum effective lines of code (default 0)" },
            PATH_PARAM,
            LIMIT_PARAM,
        ],
        core: false,
    },
    ToolSpec {
        name: "files",
        description: "Indexed file inventory with symbol/byte counts and generated tag",
        params: &[PATH_PARAM, LIMIT_PARAM],
        core: false,
    },
    ToolSpec {
        name: "resolve",
        description: "Batch-resolve names: found/ambiguous/missing per name",
        params: &[ParamSpec {
            name: "names",
            kind: ParamKind::StrArray,
            required: true,
            description: "Names to resolve, at most 1000",
        }],
        core: false,
    },
    ToolSpec {
        name: "search",
        description: "Symbol-level search: bm25-ranked FTS5 over split identifier terms",
        params: &[
            ParamSpec { name: "terms", kind: ParamKind::Str, required: true, description: "Space-separated terms; camelCase/snake_case are split and matched" },
            LIMIT_PARAM,
        ],
        core: false,
    },
    ToolSpec {
        name: "stats",
        description: "Index totals: files, symbols, edges, parse errors, sizes",
        params: &[PATH_PARAM],
        core: false,
    },
    ToolSpec {
        name: "usage",
        description: "Usage-journal summary: calls, errors, latency, empty/stale rates",
        params: &[ParamSpec {
            name: "quality",
            kind: ParamKind::Bool,
            required: false,
            description: "Add the empty-result split, cap count, and percentages",
        }],
        core: false,
    },
    ToolSpec {
        name: "mem_write",
        description: "Persist a project-memory note (agent-authored prose, never source dumps) — identical content under the same key is a no-op; different content opens a new revision",
        params: &[
            ParamSpec {
                name: "key",
                kind: ParamKind::Str,
                required: true,
                description: "Stable identity for the note (<= 200 chars, no control characters)",
            },
            ParamSpec {
                name: "body",
                kind: ParamKind::Str,
                required: true,
                description: "Note body, agent-authored prose (<= 8192 bytes)",
            },
            ParamSpec {
                name: "kind",
                kind: ParamKind::Str,
                required: true,
                description: "One of: decision, gotcha, fact, task",
            },
            ParamSpec {
                name: "tags",
                kind: ParamKind::Str,
                required: false,
                description: "Whitespace-separated lowercase tags (<= 16 x 32 chars)",
            },
            ParamSpec {
                name: "refs",
                kind: ParamKind::Str,
                required: false,
                description: "Whitespace-separated qualnames / repo paths the note is about",
            },
            ParamSpec {
                name: "scope",
                kind: ParamKind::Str,
                required: false,
                description: "durable (default, never expires) or session (expires at now + ttl, default 7 days)",
            },
            ParamSpec {
                name: "ttl",
                kind: ParamKind::Int {
                    min: 1,
                    max: None,
                },
                required: false,
                description: "Session lifetime in seconds; only valid with scope=session (default 604800 = 7 days)",
            },
            ParamSpec {
                name: "supersedes",
                kind: ParamKind::Str,
                required: false,
                description: "Key whose current note this one replaces (cross-key supersession; unknown target is exit 1)",
            },
        ],
        core: true,
    },
    ToolSpec {
        name: "mem_query",
        description: "Rank current project-memory notes for the terms (bm25 + ref-token overlap + recency, deterministic); all=true lists by recency instead",
        params: &[
            ParamSpec {
                name: "terms",
                kind: ParamKind::Str,
                required: false,
                description: "Space-separated terms (mutually exclusive with all)",
            },
            ParamSpec {
                name: "all",
                kind: ParamKind::Bool,
                required: false,
                description: "List current notes by recency instead of ranking",
            },
            ParamSpec {
                name: "kind",
                kind: ParamKind::Str,
                required: false,
                description: "Restrict to one kind: decision, gotcha, fact, task",
            },
            ParamSpec {
                name: "scope",
                kind: ParamKind::Str,
                required: false,
                description: "Restrict to one scope: durable or session",
            },
            ParamSpec {
                name: "limit",
                kind: ParamKind::Int {
                    min: 1,
                    max: Some(100),
                },
                required: false,
                description: "Maximum results shown (default 10)",
            },
            ParamSpec {
                name: "include_superseded",
                kind: ParamKind::Bool,
                required: false,
                description: "With all=true, also list superseded revisions (tagged [superseded])",
            },
        ],
        core: true,
    },
    ToolSpec {
        name: "mem_get",
        description: "Show the current project-memory note for one exact key plus its revision count",
        params: &[KEY_PARAM],
        core: false,
    },
    ToolSpec {
        name: "mem_delete",
        description: "Hard-delete every revision of one project-memory key (its search-index rows go with it)",
        params: &[KEY_PARAM],
        core: false,
    },
    ToolSpec {
        name: "mem_gc",
        description: "Reclaim project-memory disk: purge expired session notes and superseded revisions older than the 30-day retention clock, then report counts",
        params: &[],
        core: false,
    },
    ToolSpec {
        name: "mem_export",
        description: "Write current project-memory notes as JSONL to the result text (restore = mem_write replay); the caveat is reported separately so the JSONL parses cleanly",
        params: &[ParamSpec {
            name: "limit",
            kind: ParamKind::Int {
                min: 1,
                max: None,
            },
            required: false,
            description: "Take the N most recent notes instead of all of them",
        }],
        core: false,
    },
];

/// The `ParamSpec` declaring argument `name` of tool `tool`. Every Int/Float
/// read goes through its spec so the declared bounds execute at the MCP
/// boundary instead of living only in the JSON Schema renderer; a missing
/// entry is an internal error (an argument wired into `from_tool_args`
/// without a TOOL_SPECS declaration).
fn param_spec(tool: &str, name: &str) -> Result<&'static ParamSpec> {
    TOOL_SPECS
        .iter()
        .find(|t| t.name == tool)
        .and_then(|t| t.params.iter().find(|p| p.name == name))
        .ok_or_else(|| {
            AwError::Internal(format!(
                "argument '{name}' of tool '{tool}' has no ParamSpec declaration"
            ))
        })
}

/// Build a `CommandRequest` from MCP tool arguments (untrusted JSON: every
/// field is type-checked here; anything else is a usage error).
/// Typed reader over untrusted MCP tool arguments.
struct ArgReader<'a>(&'a serde_json::Value);

impl ArgReader<'_> {
    fn s(&self, key: &str) -> Option<String> {
        self.0.get(key).and_then(|v| v.as_str()).map(str::to_string)
    }

    fn b(&self, key: &str) -> bool {
        self.0.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
    }

    fn req(&self, key: &str) -> Result<String> {
        self.s(key)
            .ok_or_else(|| AwError::Usage(format!("missing required argument '{key}'")))
    }

    fn raw(&self) -> &serde_json::Value {
        self.0
    }

    /// Read an Int argument per its TOOL_SPECS declaration (absent or null
    /// falls back to `default`). A non-integer is a usage error (the module's
    /// untrusted-JSON type contract), and a value above the declared max is
    /// clamped to it — the same clamp the design's cap contract assigns to
    /// every declared max ("N > 10,000 clamped to 10,000"; depth "clamp
    /// 1-10"). Values below min pass through untouched: the documented
    /// below-min policies differ per parameter (`limit` rejects < 1 with
    /// exit 2, depth clamps), so the min stays with the consumer that owns
    /// that contract. A param added to TOOL_SPECS cannot ship without its
    /// max executing here.
    fn n_spec(&self, tool: &str, name: &str, default: usize) -> Result<usize> {
        Ok(self.n_opt_spec(tool, name)?.unwrap_or(default))
    }

    /// The Option form for Int arguments that are genuinely optional inputs
    /// (`ttl`, `mem_export` limit): `None` only when the key is absent/null.
    fn n_opt_spec(&self, tool: &str, name: &str) -> Result<Option<usize>> {
        let ParamKind::Int { max, .. } = param_spec(tool, name)?.kind else {
            return Err(AwError::Internal(format!(
                "argument '{name}' of tool '{tool}' is not declared as Int"
            )));
        };
        let Some(v) = self.0.get(name).filter(|v| !v.is_null()) else {
            return Ok(None);
        };
        let mut n = v.as_u64().ok_or_else(|| {
            AwError::Usage(format!(
                "argument '{name}' must be a non-negative integer, got {v}"
            ))
        })? as usize;
        if let Some(max) = max {
            n = n.min(max.max(0) as usize);
        }
        Ok(Some(n))
    }

    /// Read a Float argument per its TOOL_SPECS declaration (absent or null
    /// falls back to `default`). A non-number is a usage error; out-of-range
    /// or non-finite values are rejected, matching the one declared Float's
    /// consumer contract (`build.rs` rejects outside [0, 1]).
    fn f_spec(&self, tool: &str, name: &str, default: f64) -> Result<f64> {
        let ParamKind::Float { min, max } = param_spec(tool, name)?.kind else {
            return Err(AwError::Internal(format!(
                "argument '{name}' of tool '{tool}' is not declared as Float"
            )));
        };
        let Some(v) = self.0.get(name).filter(|v| !v.is_null()) else {
            return Ok(default);
        };
        let x = v
            .as_f64()
            .filter(|x| x.is_finite() && *x >= min && *x <= max)
            .ok_or_else(|| {
                AwError::Usage(format!(
                    "argument '{name}' must be a finite number in [{min}, {max}], got {v}"
                ))
            })?;
        Ok(x)
    }
}

/// Usage error for a tool name the `from_tool_args*` family does not route;
/// shared so the message cannot drift between the families.
fn unknown_tool(tool: &str) -> AwError {
    AwError::Usage(format!("unknown tool '{tool}'"))
}

pub fn from_tool_args(tool: &str, args: &serde_json::Value) -> Result<CommandRequest> {
    let a = ArgReader(args);
    match tool {
        "build" | "usage" => from_tool_args_diag(&a, tool),
        "callers" | "callees" | "context" | "implementers" | "imports" | "path" | "impact" => {
            from_tool_args_graph(&a, tool)
        }
        "dead" | "unresolved" | "symbols" | "complexity" | "files" | "resolve" | "search"
        | "stats" => from_tool_args_analysis(&a, tool),
        "mem_write" | "mem_query" | "mem_get" | "mem_delete" | "mem_gc" | "mem_export" => {
            from_tool_args_memory(&a, tool)
        }
        _ => Err(unknown_tool(tool)),
    }
}

fn from_tool_args_diag(a: &ArgReader<'_>, tool: &str) -> Result<CommandRequest> {
    Ok(match tool {
        "build" => CommandRequest::Build {
            force: a.b("force"),
            max_parse_error_rate: a.f_spec(tool, "max_parse_error_rate", 0.05)?,
        },
        "usage" => CommandRequest::Usage {
            quality: a.b("quality"),
        },
        _ => return Err(unknown_tool(tool)),
    })
}
fn from_tool_args_graph(a: &ArgReader<'_>, tool: &str) -> Result<CommandRequest> {
    Ok(match tool {
        "callers" => CommandRequest::Callers {
            name: a.req("name")?,
            limit: a.n_spec(tool, "limit", crate::query::DEFAULT_LIMIT)?,
            depth: a.n_spec(tool, "depth", 1)?,
            explain: a.b("explain"),
        },
        "callees" => CommandRequest::Callees {
            name: a.req("name")?,
            limit: a.n_spec(tool, "limit", crate::query::DEFAULT_LIMIT)?,
            depth: a.n_spec(tool, "depth", 1)?,
            explain: a.b("explain"),
        },
        "context" => CommandRequest::Context {
            name: a.req("name")?,
            limit: a.n_spec(tool, "limit", 10)?,
            explain: a.b("explain"),
        },
        "implementers" => CommandRequest::Implementers {
            name: a.req("name")?,
            limit: a.n_spec(tool, "limit", crate::query::DEFAULT_LIMIT)?,
        },
        "imports" => CommandRequest::Imports {
            file: a.req("file")?,
            limit: a.n_spec(tool, "limit", crate::query::DEFAULT_LIMIT)?,
        },
        "path" => CommandRequest::Path {
            from: a.req("from")?,
            to: a.req("to")?,
            depth: a.n_spec(tool, "depth", 10)?,
        },
        "impact" => CommandRequest::Impact {
            file: a.req("file")?,
            sites: a.b("sites"),
            depth: a.n_spec(tool, "depth", 1)?,
            edges: crate::query::parse_edge_kinds(
                &a.s("edges").unwrap_or_else(|| "calls".to_string()),
            )?,
            context: a.b("context"),
            explain: a.b("explain"),
            limit: a.n_spec(tool, "limit", crate::query::DEFAULT_LIMIT)?,
        },
        _ => return Err(unknown_tool(tool)),
    })
}
fn from_tool_args_analysis(a: &ArgReader<'_>, tool: &str) -> Result<CommandRequest> {
    Ok(match tool {
        "dead" => CommandRequest::Dead {
            exclude_tests: a.b("exclude_tests"),
            path: a.s("path"),
            limit: a.n_spec(tool, "limit", crate::query::DEFAULT_LIMIT)?,
        },
        "unresolved" => CommandRequest::Unresolved {
            names: a.b("names"),
            path: a.s("path"),
            limit: a.n_spec(tool, "limit", crate::query::DEFAULT_LIMIT)?,
        },
        "symbols" => CommandRequest::Symbols {
            file: a.req("file")?,
            outline: a.b("outline"),
            metrics: a.b("metrics"),
            limit: a.n_spec(tool, "limit", crate::query::DEFAULT_LIMIT)?,
        },
        "complexity" => CommandRequest::Complexity {
            min_loc: a.n_spec(tool, "min_loc", 0)?,
            path: a.s("path"),
            limit: a.n_spec(tool, "limit", crate::query::DEFAULT_LIMIT)?,
        },
        "files" => CommandRequest::Files {
            path: a.s("path"),
            limit: a.n_spec(tool, "limit", crate::query::DEFAULT_LIMIT)?,
        },
        "resolve" => CommandRequest::Resolve {
            names: resolve_names_from_args(a.raw())?,
        },
        "search" => CommandRequest::Search {
            terms: a.req("terms")?,
            limit: a.n_spec(tool, "limit", crate::query::DEFAULT_LIMIT)?,
        },
        "stats" => CommandRequest::Stats { path: a.s("path") },
        _ => return Err(unknown_tool(tool)),
    })
}
fn from_tool_args_memory(a: &ArgReader<'_>, tool: &str) -> Result<CommandRequest> {
    Ok(match tool {
        "mem_write" => CommandRequest::MemWrite {
            key: a.req("key")?,
            body: a.req("body")?,
            kind: a.req("kind")?,
            tags: a.s("tags"),
            refs: a.s("refs"),
            scope: a.s("scope").unwrap_or_else(|| "durable".to_string()),
            ttl: a.n_opt_spec(tool, "ttl")?.map(|v| v as u64),
            supersedes: a.s("supersedes"),
        },
        "mem_query" => CommandRequest::MemQuery {
            terms: a
                .s("terms")
                .map(|t| t.split_whitespace().map(str::to_string).collect())
                .unwrap_or_default(),
            all: a.b("all"),
            kind: a.s("kind"),
            scope: a.s("scope"),
            limit: a.n_spec(tool, "limit", crate::memory::DEFAULT_MEM_LIMIT)?,
            include_superseded: a.b("include_superseded"),
        },
        "mem_get" => CommandRequest::MemGet { key: a.req("key")? },
        "mem_delete" => CommandRequest::MemDelete { key: a.req("key")? },
        "mem_gc" => CommandRequest::MemGc,
        "mem_export" => CommandRequest::MemExport {
            limit: a.n_opt_spec(tool, "limit")?,
        },
        _ => return Err(unknown_tool(tool)),
    })
}

/// Validate and collect the `resolve` names array (untrusted input bounds).
pub(crate) const RESOLVE_MAX_NAMES: usize = 1000;
const RESOLVE_MAX_NAME_BYTES: usize = 4096;
const RESOLVE_MAX_TOTAL_BYTES: usize = 1024 * 1024;

fn resolve_names_from_args(args: &serde_json::Value) -> Result<Vec<String>> {
    let arr = args
        .get("names")
        .and_then(|v| v.as_array())
        .ok_or_else(|| AwError::Usage("resolve requires a names array".to_string()))?;
    let mut names: Vec<String> = Vec::new();
    for value in arr {
        let name = value
            .as_str()
            .ok_or_else(|| AwError::Usage("resolve names must be strings".to_string()))?;
        names.push(name.to_string());
    }
    // MCP has no stdin: the empty-list error names the argument, not stdin.
    validate_resolve_names(&names, "no names provided")?;
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every request shape, used by the surface-parity tests.
    fn all_requests() -> Vec<CommandRequest> {
        vec![
            CommandRequest::Build {
                force: false,
                max_parse_error_rate: 0.05,
            },
            CommandRequest::Callers {
                name: "x".into(),
                limit: 50,
                depth: 1,
                explain: false,
            },
            CommandRequest::Callees {
                name: "x".into(),
                limit: 50,
                depth: 1,
                explain: false,
            },
            CommandRequest::Context {
                name: "x".into(),
                limit: 10,
                explain: false,
            },
            CommandRequest::Implementers {
                name: "x".into(),
                limit: 50,
            },
            CommandRequest::Imports {
                file: "f".into(),
                limit: 50,
            },
            CommandRequest::Path {
                from: "a".into(),
                to: "b".into(),
                depth: 10,
            },
            CommandRequest::Impact {
                file: "f".into(),
                sites: false,
                depth: 1,
                edges: vec![crate::extract::EdgeKind::Calls],
                context: false,
                explain: false,
                limit: 50,
            },
            CommandRequest::Dead {
                exclude_tests: false,
                path: None,
                limit: 50,
            },
            CommandRequest::Unresolved {
                names: false,
                path: None,
                limit: 50,
            },
            CommandRequest::Symbols {
                file: "f".into(),
                outline: false,
                metrics: false,
                limit: 50,
            },
            CommandRequest::Complexity {
                min_loc: 0,
                path: None,
                limit: 50,
            },
            CommandRequest::Resolve { names: vec![] },
            CommandRequest::Files {
                path: None,
                limit: 50,
            },
            CommandRequest::Search {
                terms: "x".into(),
                limit: 50,
            },
            CommandRequest::Stats { path: None },
            CommandRequest::Usage { quality: false },
            CommandRequest::MemWrite {
                key: "k".into(),
                body: "b".into(),
                kind: "fact".into(),
                tags: None,
                refs: None,
                scope: "durable".into(),
                ttl: None,
                supersedes: None,
            },
            CommandRequest::MemQuery {
                terms: vec!["x".into()],
                all: false,
                kind: None,
                scope: None,
                limit: 10,
                include_superseded: false,
            },
            CommandRequest::MemGet { key: "k".into() },
            CommandRequest::MemDelete { key: "k".into() },
            CommandRequest::MemGc,
            CommandRequest::MemExport { limit: None },
        ]
    }

    #[test]
    fn tool_specs_cover_every_request_shape() {
        let dispatchable: Vec<&str> = all_requests()
            .iter()
            .map(CommandRequest::tool_name)
            .collect();
        for name in dispatchable {
            assert!(
                TOOL_SPECS.iter().any(|t| t.name == name),
                "no tool spec for '{name}'"
            );
        }
    }

    #[test]
    fn journal_args_and_family_cover_every_request_shape() {
        // A missing family arm panics inside the helper; this walks every
        // shape so the defensive `unreachable!` fallbacks stay covered.
        for req in all_requests() {
            let _ = req.tool_name();
            let args = req.journal_args();
            if !matches!(req, CommandRequest::MemGc) {
                assert!(!args.is_empty(), "no journal args for {req:?}");
            }
        }
    }

    #[test]
    fn from_tool_args_rejects_missing_required_strings() {
        let err = from_tool_args("callers", &serde_json::json!({})).unwrap_err();
        assert!(err.to_string().contains("'name'"), "{err}");
    }

    #[test]
    fn from_tool_args_rejects_unknown_tools() {
        let err = from_tool_args("doctor", &serde_json::json!({})).unwrap_err();
        assert!(err.to_string().contains("unknown tool 'doctor'"), "{err}");
    }
}
