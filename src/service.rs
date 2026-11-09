//! Unified command dispatch shared by the CLI and MCP surfaces: one place
//! resolves the repo root, opens the index, journals the invocation, renders
//! stdout/stderr, and maps errors to exit codes. The CLI and the MCP server
//! both convert their input into a `CommandRequest` and print the returned
//! `ExecutionResult`, so the two surfaces stay byte-identical by construction.

use std::path::Path;
use std::time::Instant;

use crate::error::{EgError, Result};
use crate::journal::JournalArgs;

/// Which surface is dispatching; journaled as `surface` in `.engram/usage.log`.
#[derive(Clone, Copy, PartialEq, Eq)]
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
    },
    Callees {
        name: String,
        limit: usize,
        depth: usize,
    },
    Context {
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
        limit: usize,
    },
    Reuse {
        paths: Vec<String>,
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
        global: bool,
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
        global: bool,
        touch: bool,
    },
    MemQuery {
        terms: Vec<String>,
        all: bool,
        kind: Option<crate::memory::MemKind>,
        scope: Option<crate::memory::MemScope>,
        limit: usize,
        include_superseded: bool,
        global: bool,
        reference: Option<String>,
    },
    MemTouch {
        key: String,
        global: bool,
    },
    MemGet {
        key: String,
        global: bool,
    },
    MemDelete {
        key: String,
        global: bool,
    },
    MemGc {
        global: bool,
    },
    MemExport {
        format: Option<String>,
        limit: Option<usize>,
        global: bool,
    },
    MemImport {
        path: String,
        format: Option<String>,
        force: bool,
        global: bool,
    },
}

/// Rendered command outcome: everything the caller prints, plus the exit code
/// and the arm's displayed-item count (the MCP KPI footer's structural input).
pub struct ExecutionResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    /// Displayed result items the arm reported; `None` on the error path and
    /// for arms without an item count.
    pub shown: Option<usize>,
}

/// Journal one QueryOutput and render it exactly as the CLI prints it:
/// stdout keeps the header, pointers, and cap line; the cap notice goes to
/// stderr.
fn emit(out: crate::query::QueryOutput) -> Result<ArmOut> {
    let ambiguous = out.pointers.iter().filter(|p| !p.tag.is_empty()).count();
    let stderr = crate::query::journal_output(&out, ambiguous);
    Ok(ArmOut {
        shown: Some(out.shown),
        stdout: crate::query::render_output(&out),
        stderr,
    })
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
/// both surfaces produce the same `.engram/usage.log` line. On the error path the
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
    let (stdout, stderr, exit_code, shown) = match outcome {
        Ok(arm) => {
            if journal {
                // record_result for the query shapes ran inside the arm; the
                // exit is 0 here.
                crate::journal::finalize(root.as_deref(), 0, latency_us);
            }
            (arm.stdout, arm.stderr, 0, arm.shown)
        }
        Err(e) => {
            let mut stdout = String::new();
            if let EgError::NoResultsCaveated { caveat, .. } = &e {
                stdout.push_str(caveat);
                stdout.push('\n');
            }
            if let EgError::Ambiguous { candidates, .. } = &e {
                for (path, line, qualname) in candidates {
                    stdout.push_str(&format!("{path}:{line} {qualname}\n"));
                }
            }
            let stderr = format!("{e}\n");
            if journal {
                crate::journal::finalize(root.as_deref(), e.exit_code(), latency_us);
            }
            (stdout, stderr, e.exit_code(), None)
        }
    };
    ExecutionResult {
        stdout,
        stderr,
        exit_code,
        shown,
    }
}

/// Arm output: stdout bytes plus any stderr note the arm itself produced
/// (query cap notices are emitted here; diagnostics come from the error path),
/// and the displayed-item count the arm journaled. The MCP KPI footer reads
/// `shown` structurally instead of re-parsing rendered stdout; arms without
/// an item count (diagnostics, memory commands) carry `None`.
pub struct ArmOut {
    pub stdout: String,
    pub stderr: String,
    pub shown: Option<usize>,
}

impl ArmOut {
    /// Arm output with no item count.
    fn plain(stdout: String, stderr: String) -> ArmOut {
        ArmOut {
            stdout,
            stderr,
            shown: None,
        }
    }

    /// Arm output from a line-rendered query: joined stdout lines, the
    /// stderr cap notice, and the shown count.
    fn from_line_query(out: crate::query::LineQuery) -> ArmOut {
        ArmOut {
            shown: Some(out.shown),
            stdout: join_lines(out.lines),
            stderr: out.stderr,
        }
    }
}

mod args;
mod from_cli;
mod mem_import;
mod mem_ops;
mod mem_query;
mod mem_write;
mod run;
mod tools;

use from_cli::*;
use mem_import::*;
use mem_ops::*;
use mem_query::*;
use mem_write::*;
use run::*;

pub(crate) use args::from_tool_args;
pub use tools::{ParamKind, ParamSpec, ToolSpec, MAX_STR_ARRAY_ITEMS, TOOL_SPECS};
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
            },
            CommandRequest::Callees {
                name: "x".into(),
                limit: 50,
                depth: 1,
            },
            CommandRequest::Context {
                name: "x".into(),
                limit: 10,
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
                limit: 50,
            },
            CommandRequest::Reuse {
                paths: vec!["f".into()],
                limit: 10,
            },
            CommandRequest::Search {
                terms: "x".into(),
                limit: 50,
            },
            CommandRequest::Stats { path: None },
            CommandRequest::Usage {
                quality: false,
                global: false,
            },
            CommandRequest::MemWrite {
                key: "k".into(),
                body: "b".into(),
                kind: "fact".into(),
                tags: None,
                refs: None,
                scope: "durable".into(),
                ttl: None,
                supersedes: None,
                global: false,
                touch: false,
            },
            CommandRequest::MemQuery {
                terms: vec!["x".into()],
                all: false,
                kind: None,
                scope: None,
                limit: 10,
                include_superseded: false,
                global: false,
                reference: None,
            },
            CommandRequest::MemTouch {
                key: "k".into(),
                global: false,
            },
            CommandRequest::MemGet {
                key: "k".into(),
                global: false,
            },
            CommandRequest::MemDelete {
                key: "k".into(),
                global: false,
            },
            CommandRequest::MemGc { global: false },
            CommandRequest::MemExport {
                format: None,
                limit: None,
                global: false,
            },
            CommandRequest::MemImport {
                path: "memory.md".into(),
                format: None,
                force: false,
                global: false,
            },
        ]
    }

    #[test]
    fn tool_specs_cover_every_request_shape() {
        let dispatchable: Vec<&str> = all_requests()
            .iter()
            .filter(|r| {
                !matches!(
                    r,
                    CommandRequest::Unresolved { .. } | CommandRequest::Stats { .. }
                )
            })
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
    fn estimated_tokens_saved_covers_every_journaled_tool() {
        // Every journaled request shape must land in a priced arm or in the
        // explicit UNPRICED_TOOLS list — never in the silent zero fallback.
        for req in all_requests() {
            let name = req.tool_name();
            if crate::journal::UNPRICED_TOOLS.contains(&name) {
                continue;
            }
            assert!(
                crate::journal::estimated_tokens_saved(name, 1, 1, 1) > 0,
                "journaled tool '{name}' is neither priced nor in UNPRICED_TOOLS"
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
            assert!(!args.is_empty(), "no journal args for {req:?}");
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
