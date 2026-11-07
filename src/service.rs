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
        edges: String,
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

impl CommandRequest {
    /// CLI conversion. `resolve --stdin` reads its names here so the service
    /// stays free of surface-specific input shapes (MCP passes a JSON array).
    pub fn from_cli(cmd: crate::cli::Cmd) -> Result<CommandRequest> {
        Ok(match cmd {
            crate::cli::Cmd::Build {
                force,
                max_parse_error_rate,
            } => CommandRequest::Build {
                force,
                max_parse_error_rate,
            },
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
            crate::cli::Cmd::Impact {
                file,
                sites,
                depth,
                edges,
                context,
                explain,
                limit,
            } => CommandRequest::Impact {
                file,
                sites,
                depth,
                edges,
                context,
                explain,
                limit,
            },
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
            crate::cli::Cmd::Resolve { .. } => CommandRequest::Resolve {
                names: read_resolve_stdin()?,
            },
            crate::cli::Cmd::Files { path, limit } => CommandRequest::Files { path, limit },
            crate::cli::Cmd::Search { terms, limit } => CommandRequest::Search { terms, limit },
            crate::cli::Cmd::Stats { path } => CommandRequest::Stats { path },
            crate::cli::Cmd::Doctor { invalidate } => CommandRequest::Doctor { invalidate },
            crate::cli::Cmd::Usage { quality } => CommandRequest::Usage { quality },
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
            crate::cli::Cmd::Mcp { .. } => {
                unreachable!("mcp is a protocol loop, handled before dispatch")
            }
        })
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
        match self {
            CommandRequest::Build {
                force,
                max_parse_error_rate,
            } => vec![
                ("force", force.to_string()),
                ("threshold", max_parse_error_rate.to_string()),
            ],
            CommandRequest::Callers {
                name,
                limit,
                depth,
                explain,
            } => vec![
                ("name", name.clone()),
                ("depth", depth.to_string()),
                ("limit", limit.to_string()),
                ("explain", explain.to_string()),
            ],
            CommandRequest::Callees {
                name,
                limit,
                depth,
                explain,
            } => vec![
                ("name", name.clone()),
                ("depth", depth.to_string()),
                ("limit", limit.to_string()),
                ("explain", explain.to_string()),
            ],
            CommandRequest::Context {
                name,
                limit,
                explain,
            } => vec![
                ("name", name.clone()),
                ("limit", limit.to_string()),
                ("explain", explain.to_string()),
            ],
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
                ("edges", edges.clone()),
                ("context", context.to_string()),
                ("explain", explain.to_string()),
                ("limit", limit.to_string()),
            ],
            CommandRequest::Dead {
                exclude_tests,
                path,
                limit,
            } => vec![
                ("exclude_tests", exclude_tests.to_string()),
                ("path", path.clone().unwrap_or_default()),
                ("limit", limit.to_string()),
            ],
            CommandRequest::Unresolved { names, path, limit } => vec![
                ("names", names.to_string()),
                ("path", path.clone().unwrap_or_default()),
                ("limit", limit.to_string()),
            ],
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
            } => vec![
                ("min_loc", min_loc.to_string()),
                ("path", path.clone().unwrap_or_default()),
                ("limit", limit.to_string()),
            ],
            CommandRequest::Resolve { names } => vec![
                ("count", names.len().to_string()),
                ("candidate_limit", "50".to_string()),
            ],
            CommandRequest::Files { path, limit } => vec![
                ("path", path.clone().unwrap_or_default()),
                ("limit", limit.to_string()),
            ],
            CommandRequest::Search { terms, limit } => {
                vec![("terms", terms.clone()), ("limit", limit.to_string())]
            }
            CommandRequest::Stats { path } => {
                vec![("path", path.clone().unwrap_or_default())]
            }
            CommandRequest::Doctor { invalidate } => {
                vec![("invalidate", invalidate.to_string())]
            }
            CommandRequest::Usage { quality } => vec![("quality", quality.to_string())],
            // Memory journal args carry key/kind/scope/terms/limit/body_len —
            // never the body text (journal privacy contract).
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
        }
    }
}

/// Rendered command outcome: everything the caller prints, plus the exit code.
pub struct ExecutionResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Render a QueryOutput exactly as the CLI prints it: stdout keeps the header,
/// pointers, and cap line; the cap notice goes to stderr.
fn render_query_output(out: &crate::query::QueryOutput) -> (String, String) {
    let stdout = crate::query::render_output(out);
    let stderr = if out.total > out.shown {
        format!(
            "[aw] capped at {} results ({} total)\n",
            out.shown, out.total
        )
    } else {
        String::new()
    };
    (stdout, stderr)
}

fn record_query_output(out: &crate::query::QueryOutput) {
    crate::journal::record_result(
        out.pointers.len(),
        out.shown,
        out.total,
        out.stale,
        out.pointers.iter().filter(|p| !p.tag.is_empty()).count(),
    );
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
/// caveat and ambiguous candidates land on stdout (the dead/clones caveat
/// contract and CUJ4) and the diagnostic on stderr.
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
        Err(AwError::BatchOutput { text, code }) => (text, String::new(), code),
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
    match req {
        CommandRequest::Build {
            force,
            max_parse_error_rate,
        } => {
            let line = crate::build::run_build(root_arg, *force, *max_parse_error_rate)?;
            Ok((format!("{line}\n"), String::new()))
        }
        CommandRequest::Callers {
            name,
            limit,
            depth,
            explain,
        } => {
            let (root, conn) = open_query(root_arg)?;
            let out = crate::query::callers(&conn, &root, name, *limit, *depth, *explain)?;
            record_query_output(&out);
            Ok(render_query_output(&out))
        }
        CommandRequest::Callees {
            name,
            limit,
            depth,
            explain,
        } => {
            let (root, conn) = open_query(root_arg)?;
            let out = crate::query::callees(&conn, &root, name, *limit, *depth, *explain)?;
            record_query_output(&out);
            Ok(render_query_output(&out))
        }
        CommandRequest::Context {
            name,
            limit,
            explain,
        } => {
            let (root, conn) = open_query(root_arg)?;
            let lines = crate::query::context(&conn, &root, name, *limit, *explain)?;
            Ok((join_lines(lines), String::new()))
        }
        CommandRequest::Implementers { name, limit } => {
            let (root, conn) = open_query(root_arg)?;
            let out = crate::query::implementers(&conn, &root, name, *limit)?;
            record_query_output(&out);
            Ok(render_query_output(&out))
        }
        CommandRequest::Imports { file, limit } => {
            let (root, conn) = open_query(root_arg)?;
            let out = crate::query::imports(&conn, &root, file, *limit)?;
            record_query_output(&out);
            Ok(render_query_output(&out))
        }
        CommandRequest::Path { from, to, depth } => {
            let (root, conn) = open_query(root_arg)?;
            let lines = crate::query::path(&conn, &root, from, to, *depth)?;
            Ok((join_lines(lines), String::new()))
        }
        CommandRequest::Impact {
            file,
            sites,
            depth,
            edges,
            context,
            explain,
            limit,
        } => {
            let (root, conn) = open_query(root_arg)?;
            let out = crate::query::impact(
                &conn, &root, file, *limit, *sites, *depth, edges, *context, *explain,
            )?;
            record_query_output(&out);
            Ok(render_query_output(&out))
        }
        CommandRequest::Dead {
            exclude_tests,
            path,
            limit,
        } => {
            let (root, conn) = open_query(root_arg)?;
            let out = crate::query::dead(&conn, &root, *limit, *exclude_tests, path.as_deref())?;
            record_query_output(&out);
            Ok(render_query_output(&out))
        }
        CommandRequest::Unresolved { names, path, limit } => {
            let (root, conn) = open_query(root_arg)?;
            let lines = if *names {
                crate::query::unresolved_names(&conn, &root, path.as_deref(), *limit)?
            } else {
                crate::query::unresolved(&conn, path.as_deref(), *limit)?
            };
            Ok((join_lines(lines), String::new()))
        }
        CommandRequest::Symbols {
            file,
            outline,
            metrics,
            limit,
        } => {
            let (root, conn) = open_query(root_arg)?;
            let lines = crate::query::symbols(&conn, &root, file, *outline, *metrics, *limit)?;
            Ok((join_lines(lines), String::new()))
        }
        CommandRequest::Complexity {
            min_loc,
            path,
            limit,
        } => {
            let (root, conn) = open_query(root_arg)?;
            let lines = crate::query::complexity(&conn, &root, *min_loc, path.as_deref(), *limit)?;
            Ok((join_lines(lines), String::new()))
        }
        CommandRequest::Resolve { names } => {
            let (_root, conn) = open_query(root_arg)?;
            let (lines, exit) = crate::query::resolve_names(&conn, names)?;
            crate::journal::record_result(lines.len(), lines.len(), lines.len(), false, 0);
            if exit != 0 {
                return Err(AwError::BatchOutput {
                    text: join_lines(lines),
                    code: exit,
                });
            }
            Ok((join_lines(lines), String::new()))
        }
        CommandRequest::Files { path, limit } => {
            let (_root, conn) = open_query(root_arg)?;
            let lines = crate::query::files(&conn, path.as_deref(), *limit)?;
            Ok((join_lines(lines), String::new()))
        }
        CommandRequest::Search { terms, limit } => {
            let (root, conn) = open_query(root_arg)?;
            let lines = crate::query::search(&conn, &root, terms, *limit)?;
            Ok((join_lines(lines), String::new()))
        }
        CommandRequest::Stats { path } => {
            let (root, conn) = open_query(root_arg)?;
            let line = crate::insights::stats(&conn, &root, path.as_deref())?;
            crate::journal::record_result(1, 1, 1, false, 0);
            Ok((format!("{line}\n"), String::new()))
        }
        CommandRequest::Doctor { invalidate } => {
            let root = crate::walk::discover_root(root_arg)?;
            let lines = if *invalidate {
                crate::doctor::invalidate(&root)?
            } else {
                crate::doctor::report(&root)?
            };
            Ok((join_lines(lines), String::new()))
        }
        CommandRequest::Usage { quality } => {
            let root = crate::walk::discover_root(root_arg)?;
            let lines = crate::journal::summary(&root, *quality)?;
            Ok((join_lines(lines), String::new()))
        }
        CommandRequest::MemWrite {
            key,
            body,
            kind,
            tags,
            refs,
            scope,
            ttl,
            supersedes,
        } => {
            if let Some(target) = supersedes {
                if target.trim() == key.trim() {
                    return Err(AwError::Usage(
                        "mem write: --supersedes must name a different key (same-key updates go through the revision chain)"
                            .to_string(),
                    ));
                }
            }
            let note = crate::memory::normalize_note(
                key,
                kind,
                tags.as_deref(),
                refs.as_deref(),
                scope,
                body,
                *ttl,
            )?;
            let root = crate::walk::discover_root(root_arg)?;
            let mut conn = crate::db::open_memory_store(&root)?;
            let outcome = crate::db::memory_write(
                &mut conn,
                &crate::db::MemNoteInput {
                    key: &note.key,
                    kind: &note.kind,
                    tags: &note.tags,
                    refs: &note.refs,
                    scope: &note.scope,
                    session: &crate::journal::session_tag(),
                    body: &note.body,
                    source: surface.as_str(),
                    expires_at: if note.ttl_secs == 0 {
                        0
                    } else {
                        crate::db::unix_now() + note.ttl_secs
                    },
                    supersedes: supersedes.as_deref(),
                },
                crate::db::unix_now(),
            )?;
            crate::journal::record_result(1, 1, 1, false, 0);
            let line = match outcome {
                crate::db::MemWriteOutcome::Written {
                    rev,
                    superseded_rev,
                } => match superseded_rev {
                    Some(old) => {
                        format!(
                            "ok: '{}' rev {} written (supersedes rev {})",
                            note.key, rev, old
                        )
                    }
                    None => format!("ok: '{}' rev {} written", note.key, rev),
                },
                crate::db::MemWriteOutcome::Duplicate { rev } => {
                    format!(
                        "ok: '{}' unchanged at rev {} (identical content)",
                        note.key, rev
                    )
                }
            };
            Ok((
                format!("{}\n{line}\n", crate::memory::MEMORY_CAVEAT),
                String::new(),
            ))
        }
        CommandRequest::MemQuery {
            terms,
            all,
            kind,
            scope,
            limit,
            include_superseded,
        } => {
            let limit = (*limit).clamp(1, crate::memory::MAX_MEM_LIMIT);
            if *all && !terms.is_empty() {
                return Err(AwError::Usage(
                    "mem query: --all and terms are mutually exclusive".to_string(),
                ));
            }
            if !*all && terms.iter().all(|t| t.trim().is_empty()) {
                return Err(AwError::Usage(
                    "mem query: provide terms or --all".to_string(),
                ));
            }
            // Superseded rows participate in the explicit listing only; the
            // ranked resolver keeps supersession-at-result-time exclusion.
            if *include_superseded && !*all {
                return Err(AwError::Usage(
                    "mem query: --include-superseded requires --all".to_string(),
                ));
            }
            let root = crate::walk::discover_root(root_arg)?;
            let conn = crate::db::open_memory_query(&root)?;
            let now = crate::db::unix_now();
            if *all {
                let rows = crate::db::memory_current_rows(
                    &conn,
                    kind.as_deref(),
                    scope.as_deref(),
                    now,
                    *include_superseded,
                )?;
                // --all order is pure recency (created_at desc), key as
                // tie-break; the SQL already sorts this way.
                let total = rows.len();
                if total == 0 {
                    return Err(mem_empty(if *include_superseded {
                        "memory store has no notes"
                    } else {
                        "memory store has no current notes"
                    }));
                }
                let shown = total.min(limit);
                let mut stdout = String::new();
                stdout.push_str(crate::memory::MEMORY_CAVEAT);
                stdout.push('\n');
                for row in &rows[..shown] {
                    let rendered = if row.superseded_by.is_some() {
                        crate::memory::render_row_tagged(row, " [superseded]")
                    } else {
                        crate::memory::render_row(row)
                    };
                    stdout.push_str(&rendered);
                    stdout.push('\n');
                }
                if total > shown {
                    stdout.push_str(&crate::memory::render_showing(shown, total));
                    stdout.push('\n');
                }
                crate::journal::record_result(shown, shown, total, false, 0);
                let stderr = cap_stderr(total > shown, shown, total);
                Ok((stdout, stderr))
            } else {
                let joined = terms.join(" ");
                let candidates =
                    mem_ranked(&conn, &joined, kind.as_deref(), scope.as_deref(), now)?;
                if candidates.is_empty() {
                    return Err(mem_empty(&format!("no memory matches '{joined}'")));
                }
                let total = candidates.len();
                let shown = total.min(limit);
                let mut stdout = String::new();
                stdout.push_str(crate::memory::MEMORY_CAVEAT);
                stdout.push('\n');
                for cand in &candidates[..shown] {
                    stdout.push_str(&crate::memory::render_row(&cand.row));
                    stdout.push('\n');
                }
                if total > shown {
                    stdout.push_str(&crate::memory::render_showing(shown, total));
                    stdout.push('\n');
                }
                crate::journal::record_result(shown, shown, total, false, 0);
                let stderr = cap_stderr(total > shown, shown, total);
                Ok((stdout, stderr))
            }
        }
        CommandRequest::MemGet { key } => {
            let root = crate::walk::discover_root(root_arg)?;
            let conn = crate::db::open_memory_query(&root)?;
            match crate::db::memory_get(&conn, key)? {
                None => Err(mem_empty(&format!("unknown memory key '{key}'"))),
                Some((row, revs)) => {
                    crate::journal::record_result(1, 1, 1, false, 0);
                    Ok((
                        format!(
                            "{}\n{}\nrevs: {}\n",
                            crate::memory::MEMORY_CAVEAT,
                            crate::memory::render_row(&row),
                            revs
                        ),
                        String::new(),
                    ))
                }
            }
        }
        CommandRequest::MemDelete { key } => {
            let root = crate::walk::discover_root(root_arg)?;
            let mut conn = crate::db::open_memory_mutation(&root)?;
            match crate::db::memory_delete(&mut conn, key)? {
                None => Err(mem_empty(&format!("unknown memory key '{key}'"))),
                Some(revs) => {
                    crate::journal::record_result(
                        revs.max(0) as usize,
                        revs.max(0) as usize,
                        revs.max(0) as usize,
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
        CommandRequest::MemGc => {
            let root = crate::walk::discover_root(root_arg)?;
            let mut conn = crate::db::open_memory_mutation(&root)?;
            let (expired, superseded, durable) =
                crate::db::memory_gc(&mut conn, crate::db::unix_now())?;
            crate::journal::record_result(1, 1, 1, false, 0);
            Ok((
                format!(
                    "{}\nexpired={expired} removed, superseded={superseded} removed, durable={durable} rows\n",
                    crate::memory::MEMORY_CAVEAT
                ),
                String::new(),
            ))
        }
        CommandRequest::MemExport { limit } => {
            let root = crate::walk::discover_root(root_arg)?;
            let conn = crate::db::open_memory_query(&root)?;
            let rows = crate::db::memory_export_rows(&conn, *limit)?;
            if rows.is_empty() {
                return Err(AwError::NoResults(
                    "memory store has no current notes to export".to_string(),
                ));
            }
            crate::journal::record_result(rows.len(), rows.len(), rows.len(), false, 0);
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
    }
}

/// Caveated empty-result error shared by the mem read commands (stdout keeps
/// the caveat line, stderr the diagnostic, exit 1).
fn mem_empty(detail: &str) -> AwError {
    AwError::NoResultsCaveated {
        caveat: crate::memory::MEMORY_CAVEAT,
        detail: detail.to_string(),
    }
}

/// Stderr cap notice matching the house cap contract.
fn cap_stderr(capped: bool, shown: usize, total: usize) -> String {
    if capped {
        format!("[aw] capped at {shown} results ({total} total)\n")
    } else {
        String::new()
    }
}

/// The resolver: FTS candidates capped at CANDIDATE_CAP, ref/tag token
/// overlap, recency decay, fixed-weight fusion (D4-A). Pure over rows the
/// db layer returned; kind/scope/expiry filters apply after the fetch.
fn mem_ranked(
    conn: &rusqlite::Connection,
    joined_terms: &str,
    kind: Option<&str>,
    scope: Option<&str>,
    now: i64,
) -> Result<Vec<crate::memory::Candidate>> {
    let query = crate::memory::mem_query_terms(joined_terms);
    let hits = crate::db::memory_fts_candidates(conn, &query, crate::memory::CANDIDATE_CAP)?;
    if hits.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i64> = hits.iter().map(|(id, _)| *id).collect();
    let rows = crate::db::memory_rows_by_ids(conn, &ids)?;
    let by_id: std::collections::HashMap<i64, crate::db::MemRow> =
        rows.into_iter().map(|row| (row.id, row)).collect();
    let query_tokens: std::collections::BTreeSet<String> = crate::memory::mem_tokens(joined_terms)
        .into_iter()
        .collect();
    let mut candidates = Vec::new();
    for (id, lex_raw) in hits {
        let Some(row) = by_id.get(&id) else {
            continue;
        };
        if row.expires_at != 0 && row.expires_at <= now {
            continue;
        }
        if let Some(k) = kind {
            if row.kind != k {
                continue;
            }
        }
        if let Some(s) = scope {
            if row.scope != s {
                continue;
            }
        }
        let set = crate::memory::mem_ref_tag_tokens(&row.refs, &row.tags);
        let ref_raw = if set.is_empty() {
            0.0
        } else {
            set.intersection(&query_tokens).count() as f64 / set.len() as f64
        };
        let age_days = (now - row.created_at).max(0) as f64 / 86400.0;
        candidates.push(crate::memory::Candidate {
            row: row.clone(),
            lex_raw,
            ref_raw,
            age_days,
            score: 0.0,
        });
    }
    Ok(crate::memory::fuse(candidates))
}

/// Resolve the repo root and open the index read-only (the shared query gate:
/// missing -> 1, corrupt -> 3, skew -> 5).
fn open_query(root_arg: Option<&Path>) -> Result<(std::path::PathBuf, rusqlite::Connection)> {
    let root = crate::walk::discover_root(root_arg)?;
    let conn = crate::db::open_for_query(&root)?;
    Ok((root, conn))
}

/// Read and validate `resolve --stdin` input (CLI-only input shape; the MCP
/// surface passes names as a JSON array). Bounds mirror the MCP dispatch:
/// 1 MiB total, 4096 bytes per line, 1000 retained names, `#` comments.
pub fn read_resolve_stdin() -> Result<Vec<String>> {
    let mut input = String::new();
    std::io::stdin()
        .take(1024 * 1024 + 1)
        .read_to_string(&mut input)?;
    if input.len() > 1024 * 1024 {
        return Err(AwError::Usage("resolve input exceeds 1 MiB".to_string()));
    }
    let mut names = Vec::new();
    for line in input.lines() {
        if line.len() > 4096 {
            return Err(AwError::Usage(
                "resolve line exceeds 4096 bytes".to_string(),
            ));
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            names.push(trimmed.to_string());
        }
    }
    if names.len() > 1000 {
        return Err(AwError::Usage(
            "resolve accepts at most 1000 names".to_string(),
        ));
    }
    if names.is_empty() {
        return Err(AwError::NoResults("no names on stdin".to_string()));
    }
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
    },
    ToolSpec {
        name: "callers",
        description: "Who calls this symbol — direct by default; transitive with depth; pointer list path:line",
        params: &[
            ParamSpec { name: "name", kind: ParamKind::Str, required: true, description: "Symbol name or dotted qualname" },
            DEPTH_PARAM,
            LIMIT_PARAM,
            EXPLAIN_PARAM,
        ],
    },
    ToolSpec {
        name: "callees",
        description: "What does this symbol call — transitive with depth",
        params: &[
            ParamSpec { name: "name", kind: ParamKind::Str, required: true, description: "Symbol name or dotted qualname" },
            DEPTH_PARAM,
            LIMIT_PARAM,
            EXPLAIN_PARAM,
        ],
    },
    ToolSpec {
        name: "path",
        description: "Shortest call path between two symbols — hop call-site pointers path:line over resolved edges; dynamic dispatch may hide paths",
        params: &[
            ParamSpec { name: "from", kind: ParamKind::Str, required: true, description: "Symbol the path starts from" },
            ParamSpec { name: "to", kind: ParamKind::Str, required: true, description: "Symbol the path must reach" },
            ParamSpec { name: "depth", kind: ParamKind::Int { min: 1, max: Some(10) }, required: false, description: "Maximum hops (default 10)" },
        ],
    },
    ToolSpec {
        name: "context",
        description: "One-call symbol context: definition + signature, direct callers, direct callees over resolved edges plus tagged ambiguous candidates",
        params: &[
            ParamSpec { name: "name", kind: ParamKind::Str, required: true, description: "Symbol name or dotted qualname" },
            ParamSpec { name: "limit", kind: ParamKind::Int { min: 1, max: Some(50) }, required: false, description: "Callers/callees shown per section (default 10)" },
            EXPLAIN_PARAM,
        ],
    },
    ToolSpec {
        name: "implementers",
        description: "Who inherits from or implements this symbol — resolved inherits edges only; cross-file impls omitted",
        params: &[
            ParamSpec { name: "name", kind: ParamKind::Str, required: true, description: "Symbol name or dotted qualname" },
            LIMIT_PARAM,
        ],
    },
    ToolSpec {
        name: "imports",
        description: "What does this file import — outbound resolved imports; external/unresolved omitted",
        params: &[
            ParamSpec { name: "file", kind: ParamKind::Str, required: true, description: "Repo-relative path of the indexed file" },
            LIMIT_PARAM,
        ],
    },
    ToolSpec {
        name: "impact",
        description: "Files that call into <file> — one row per caller file with a site count; an empty calls-only result names import/inherit counts; sites=true lists every call site; edges/depth extend the dependency set",
        params: &[
            ParamSpec { name: "file", kind: ParamKind::Str, required: true, description: "Repo-relative path of the indexed file" },
            ParamSpec { name: "sites", kind: ParamKind::Bool, required: false, description: "List every call site instead of one row per file" },
            DEPTH_PARAM,
            ParamSpec { name: "edges", kind: ParamKind::Str, required: false, description: "Edge kinds: calls,imports,inherits,all (default calls)" },
            ParamSpec { name: "context", kind: ParamKind::Bool, required: false, description: "Append the enclosing symbol to each pointer" },
            EXPLAIN_PARAM,
            LIMIT_PARAM,
        ],
    },
    ToolSpec {
        name: "dead",
        description: "Dead-code candidates (no resolved inbound calls/imports/inherits) — candidates, never proof; exclude_tests and path scope filters",
        params: &[
            ParamSpec { name: "exclude_tests", kind: ParamKind::Bool, required: false, description: "Exclude tests/ paths and test_* symbols" },
            PATH_PARAM,
            LIMIT_PARAM,
        ],
    },
    ToolSpec {
        name: "unresolved",
        description: "Per-file unresolved-callee counts, or names=true for path:line kind name sites",
        params: &[
            ParamSpec { name: "names", kind: ParamKind::Bool, required: false, description: "List unresolved sites as path:line kind name" },
            PATH_PARAM,
            LIMIT_PARAM,
        ],
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
    },
    ToolSpec {
        name: "complexity",
        description: "Per-symbol complexity ranking: effective LOC",
        params: &[
            ParamSpec { name: "min_loc", kind: ParamKind::Int { min: 0, max: None }, required: false, description: "Minimum effective lines of code (default 0)" },
            PATH_PARAM,
            LIMIT_PARAM,
        ],
    },
    ToolSpec {
        name: "files",
        description: "Indexed file inventory with symbol/byte counts and generated tag",
        params: &[PATH_PARAM, LIMIT_PARAM],
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
    },
    ToolSpec {
        name: "search",
        description: "Symbol-level search: bm25-ranked FTS5 over split identifier terms",
        params: &[
            ParamSpec { name: "terms", kind: ParamKind::Str, required: true, description: "Space-separated terms; camelCase/snake_case are split and matched" },
            LIMIT_PARAM,
        ],
    },
    ToolSpec {
        name: "stats",
        description: "Index totals: files, symbols, edges, parse errors, sizes",
        params: &[PATH_PARAM],
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
    },
    ToolSpec {
        name: "mem_get",
        description: "Show the current project-memory note for one exact key plus its revision count",
        params: &[ParamSpec {
            name: "key",
            kind: ParamKind::Str,
            required: true,
            description: "The note's stable identity",
        }],
    },
    ToolSpec {
        name: "mem_delete",
        description: "Hard-delete every revision of one project-memory key (its search-index rows go with it)",
        params: &[ParamSpec {
            name: "key",
            kind: ParamKind::Str,
            required: true,
            description: "The note's stable identity",
        }],
    },
    ToolSpec {
        name: "mem_gc",
        description: "Reclaim project-memory disk: purge expired session notes and superseded revisions older than the 30-day retention clock, then report counts",
        params: &[],
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
    },
];

/// The core navigation tools exposed by `aw mcp` without `--all-tools`.
/// OQ1 (owner decision, 2026-11-05): mem_write + mem_query join the default
/// surface — the daily write/query loop; mem_get/mem_delete/mem_gc/mem_export
/// stay behind `--all-tools`.
pub const CORE_TOOLS: &[&str] = &[
    "build",
    "path",
    "context",
    "callers",
    "callees",
    "impact",
    "imports",
    "symbols",
    "mem_write",
    "mem_query",
];

/// Build a `CommandRequest` from MCP tool arguments (untrusted JSON: every
/// field is type-checked here; anything else is a usage error).
pub fn from_tool_args(tool: &str, args: &serde_json::Value) -> Result<CommandRequest> {
    let s = |key: &str| args.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let n = |key: &str, default: usize| -> usize {
        args.get(key)
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(default)
    };
    let b = |key: &str| args.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
    let req = |key: &str| -> Result<String> {
        s(key).ok_or_else(|| AwError::Usage(format!("missing required argument '{key}'")))
    };
    Ok(match tool {
        "build" => CommandRequest::Build {
            force: b("force"),
            max_parse_error_rate: args
                .get("max_parse_error_rate")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.05),
        },
        "callers" => CommandRequest::Callers {
            name: req("name")?,
            limit: n("limit", crate::query::DEFAULT_LIMIT),
            depth: n("depth", 1),
            explain: b("explain"),
        },
        "callees" => CommandRequest::Callees {
            name: req("name")?,
            limit: n("limit", crate::query::DEFAULT_LIMIT),
            depth: n("depth", 1),
            explain: b("explain"),
        },
        "context" => CommandRequest::Context {
            name: req("name")?,
            limit: n("limit", 10),
            explain: b("explain"),
        },
        "implementers" => CommandRequest::Implementers {
            name: req("name")?,
            limit: n("limit", crate::query::DEFAULT_LIMIT),
        },
        "imports" => CommandRequest::Imports {
            file: req("file")?,
            limit: n("limit", crate::query::DEFAULT_LIMIT),
        },
        "path" => CommandRequest::Path {
            from: req("from")?,
            to: req("to")?,
            depth: n("depth", 10),
        },
        "impact" => CommandRequest::Impact {
            file: req("file")?,
            sites: b("sites"),
            depth: n("depth", 1),
            edges: s("edges").unwrap_or_else(|| "calls".to_string()),
            context: b("context"),
            explain: b("explain"),
            limit: n("limit", crate::query::DEFAULT_LIMIT),
        },
        "dead" => CommandRequest::Dead {
            exclude_tests: b("exclude_tests"),
            path: s("path"),
            limit: n("limit", crate::query::DEFAULT_LIMIT),
        },
        "unresolved" => CommandRequest::Unresolved {
            names: b("names"),
            path: s("path"),
            limit: n("limit", crate::query::DEFAULT_LIMIT),
        },
        "symbols" => CommandRequest::Symbols {
            file: req("file")?,
            outline: b("outline"),
            metrics: b("metrics"),
            limit: n("limit", crate::query::DEFAULT_LIMIT),
        },
        "complexity" => CommandRequest::Complexity {
            min_loc: n("min_loc", 0),
            path: s("path"),
            limit: n("limit", crate::query::DEFAULT_LIMIT),
        },
        "files" => CommandRequest::Files {
            path: s("path"),
            limit: n("limit", crate::query::DEFAULT_LIMIT),
        },
        "resolve" => CommandRequest::Resolve {
            names: resolve_names_from_args(args)?,
        },
        "search" => CommandRequest::Search {
            terms: req("terms")?,
            limit: n("limit", crate::query::DEFAULT_LIMIT),
        },
        "stats" => CommandRequest::Stats { path: s("path") },
        "usage" => CommandRequest::Usage {
            quality: b("quality"),
        },
        "mem_write" => CommandRequest::MemWrite {
            key: req("key")?,
            body: req("body")?,
            kind: req("kind")?,
            tags: s("tags"),
            refs: s("refs"),
            scope: s("scope").unwrap_or_else(|| "durable".to_string()),
            ttl: args.get("ttl").and_then(|v| v.as_u64()),
            supersedes: s("supersedes"),
        },
        "mem_query" => CommandRequest::MemQuery {
            terms: s("terms")
                .map(|t| t.split_whitespace().map(str::to_string).collect())
                .unwrap_or_default(),
            all: b("all"),
            kind: s("kind"),
            scope: s("scope"),
            limit: n("limit", crate::memory::DEFAULT_MEM_LIMIT),
            include_superseded: b("include_superseded"),
        },
        "mem_get" => CommandRequest::MemGet { key: req("key")? },
        "mem_delete" => CommandRequest::MemDelete { key: req("key")? },
        "mem_gc" => CommandRequest::MemGc,
        "mem_export" => CommandRequest::MemExport {
            limit: args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize),
        },
        _ => return Err(AwError::Usage(format!("unknown tool '{tool}'"))),
    })
}

/// Validate and collect the `resolve` names array (untrusted input bounds).
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
    if names.len() > 1000
        || names.iter().any(|n| n.len() > 4096)
        || names.iter().map(|n| n.len()).sum::<usize>() > 1024 * 1024
    {
        return Err(AwError::Usage(
            "resolve accepts at most 1000 names, 4096 bytes each, 1 MiB total".to_string(),
        ));
    }
    if names
        .iter()
        .all(|n| n.trim().is_empty() || n.trim().starts_with('#'))
    {
        return Err(AwError::NoResults("no names on stdin".to_string()));
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_specs_cover_every_request_shape() {
        let dispatchable: Vec<&'static str> = [
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
                edges: "calls".into(),
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
