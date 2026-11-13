//! Unified command dispatch shared by the CLI and MCP surfaces: one place
//! resolves the repo root, opens the index, journals the invocation, renders
//! stdout/stderr, and maps errors to exit codes. The CLI and the MCP server
//! both convert their input into a `CommandRequest` and print the returned
//! `ExecutionResult`, so the two surfaces stay byte-identical by construction.

use std::path::Path;
use std::time::Instant;

use crate::error::{EgError, Result};

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
        exit_code: None,
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
    // One resolution for both the arm and the journal line. `RepoRef` also
    // walks a subdirectory up to its git root, so a `--root src` cannot invent
    // a repository named after the directory.
    let resolved_root = crate::repo::RepoRef::resolve(root_arg)
        .ok()
        .map(|r| r.root().to_path_buf());
    let outcome = run(req, resolved_root.as_deref().or(root_arg), surface);
    let latency_us = started.elapsed().as_micros() as u64;
    let (stdout, stderr, exit_code, shown) = match outcome {
        Ok(arm) => {
            let code = arm.exit_code.unwrap_or(0);
            if journal {
                // record_result for the query shapes ran inside the arm.
                crate::journal::finalize(resolved_root.as_deref(), code, latency_us);
            }
            (arm.stdout, arm.stderr, code, arm.shown)
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
                crate::journal::finalize(resolved_root.as_deref(), e.exit_code(), latency_us);
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
    /// Optional process exit for arms whose document is still valid on a
    /// non-zero code (audit's no-data and `--fail-on` paths).
    pub exit_code: Option<i32>,
}

impl ArmOut {
    /// Arm output with no item count.
    fn plain(stdout: String, stderr: String) -> ArmOut {
        ArmOut {
            stdout,
            stderr,
            shown: None,
            exit_code: None,
        }
    }

    /// Arm output from a line-rendered query: joined stdout lines, the
    /// stderr cap notice, and the shown count.
    fn from_line_query(out: crate::query::LineQuery) -> ArmOut {
        ArmOut {
            shown: Some(out.shown),
            stdout: join_lines(out.lines),
            stderr: out.stderr,
            exit_code: None,
        }
    }
}

mod args;
mod explain;
mod from_cli;
mod mem_export;
mod mem_import;
mod mem_ops;
mod mem_query;
mod mem_reindex;
mod mem_write;
mod request;
mod run;
mod scope;
mod tools;

use from_cli::*;
use mem_export::*;
use mem_import::*;
use mem_ops::*;
use mem_query::*;
use mem_reindex::*;
use mem_write::*;
use run::*;

pub(crate) use args::from_tool_args;
pub use request::CommandRequest;
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
                deep_callee: false,
                min_confidence: "low".into(),
                propose: None,
                callees: Vec::new(),
                explain: false,
                show_suppressed: false,
                coverage: Vec::new(),
                judge: false,
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
            CommandRequest::Audit {
                json: false,
                global: false,
                days: 7,
                baseline_days: 7,
                all_checks: false,
                reveal_paths: false,
                fail_on: None,
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
                since: None,
                until: None,
                local: false,
                explain: false,
                legacy: false,
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
                since: None,
                until: None,
                scan: false,
            },
            CommandRequest::MemImport {
                path: "memory.md".into(),
                format: None,
                force: false,
                global: false,
            },
            CommandRequest::MemReindex { global: false },
        ]
    }

    #[test]
    fn tool_specs_cover_every_request_shape() {
        let dispatchable: Vec<&str> = all_requests()
            .iter()
            .filter(|r| {
                !matches!(
                    r,
                    CommandRequest::Unresolved { .. }
                        | CommandRequest::Stats { .. }
                        | CommandRequest::Audit { .. }
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
