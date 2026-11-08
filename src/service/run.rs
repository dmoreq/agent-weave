//! Command dispatch: routes each `CommandRequest` family to its executor
//! and adapts query/graph/traversal results into rendered `ArmOut` text.

use super::*;

pub(super) fn run(
    req: &CommandRequest,
    root_arg: Option<&Path>,
    surface: Surface,
) -> Result<ArmOut> {
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
            let (line, stderr, shown) =
                crate::build::run_build(root_arg, *force, *max_parse_error_rate)?;
            Ok(ArmOut {
                stdout: format!("{line}\n"),
                stderr,
                shown: Some(shown),
            })
        }
        CommandRequest::Doctor { invalidate } => run_doctor(root_arg, *invalidate),
        CommandRequest::Usage { quality, global } => run_usage(root_arg, *quality, *global),
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
        CommandRequest::Context { name, limit } => Ok(ArmOut::from_line_query(
            crate::query::context(&ctx, name, *limit)?,
        )),
        CommandRequest::Imports { file, limit } => emit(crate::query::imports(&ctx, file, *limit)?),
        CommandRequest::Path { from, to, depth } => Ok(ArmOut::from_line_query(
            crate::query::path(&ctx, from, to, *depth)?,
        )),
        CommandRequest::Impact { .. } => run_impact(req, &ctx),
        _ => unreachable!("run_graph_query called with a non-graph request"),
    }
}

/// `callers`/`callees`: one name plus the shared limit/depth bundle.
fn run_traversal(req: &CommandRequest, ctx: &crate::query::QueryCtx<'_>) -> Result<ArmOut> {
    match req {
        CommandRequest::Callers { name, limit, depth } => emit(crate::query::callers(
            ctx,
            name,
            graph_opts(*limit, *depth),
        )?),
        CommandRequest::Callees { name, limit, depth } => emit(crate::query::callees(
            ctx,
            name,
            graph_opts(*limit, *depth),
        )?),
        _ => unreachable!("run_traversal called with a non-traversal request"),
    }
}

/// Shared graph-traversal knobs for `callers`/`callees` (ImpactOpts-style).
fn graph_opts(limit: usize, depth: usize) -> crate::query::GraphQueryOpts {
    crate::query::GraphQueryOpts { limit, depth }
}

/// `impact`: one file plus the extended ImpactOpts bundle.
fn run_impact(req: &CommandRequest, ctx: &crate::query::QueryCtx<'_>) -> Result<ArmOut> {
    let CommandRequest::Impact {
        file,
        sites,
        depth,
        edges,
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
        },
    )?;
    emit(out)
}

/// Inventory, analysis, and reuse queries.
fn run_analysis_query(req: &CommandRequest, root_arg: Option<&Path>) -> Result<ArmOut> {
    let (root, conn) = open_query(root_arg)?;
    let ctx = crate::query::QueryCtx {
        conn: &conn,
        root: &root,
    };
    match req {
        CommandRequest::Unresolved { .. }
        | CommandRequest::Symbols { .. }
        | CommandRequest::Reuse { .. }
        | CommandRequest::Search { .. } => run_line_query(req, &ctx),
        CommandRequest::Stats { path } => {
            let line = crate::insights::stats(&ctx, path.as_deref())?;
            crate::journal::record_result(crate::journal::OutcomeCounts::uniform(1), false, 0);
            Ok(ArmOut {
                stdout: format!("{line}\n"),
                stderr: String::new(),
                shown: Some(1),
            })
        }
        _ => unreachable!("run_analysis_query called with a non-analysis request"),
    }
}

/// Analysis arms that render plain lines (no QueryOutput journaling). The
/// stderr half carries the queries' cap notices (the single cap-notice
/// channel).
fn run_line_query(req: &CommandRequest, ctx: &crate::query::QueryCtx<'_>) -> Result<ArmOut> {
    let out = match req {
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
            limit,
        } => crate::query::symbols(
            ctx,
            file,
            *limit,
            crate::query::SymbolsOpts { outline: *outline },
        )?,
        CommandRequest::Reuse { paths, limit } => crate::query::run_reuse(ctx, paths, *limit)?,
        CommandRequest::Search { terms, limit } => crate::query::search(ctx, terms, *limit)?,
        _ => unreachable!("run_line_query called with a non-line request"),
    };
    Ok(ArmOut::from_line_query(out))
}

/// Memory-store dispatcher. Each command owns its request shape in a small
/// helper; the outer match only selects the command.
fn run_memory(req: &CommandRequest, root_arg: Option<&Path>, surface: Surface) -> Result<ArmOut> {
    match req {
        CommandRequest::MemWrite { .. } => run_mem_write(req, root_arg, surface),
        CommandRequest::MemQuery { .. } => run_mem_query(req, root_arg),
        CommandRequest::MemGet { .. } => run_mem_get(req, root_arg),
        CommandRequest::MemDelete { .. } => run_mem_delete(req, root_arg),
        CommandRequest::MemGc { .. } => run_mem_gc(req, root_arg),
        CommandRequest::MemExport { .. } => run_mem_export(req, root_arg),
        _ => unreachable!("run_memory called with a non-memory request"),
    }
}

fn run_mem_gc(req: &CommandRequest, root_arg: Option<&Path>) -> Result<ArmOut> {
    let CommandRequest::MemGc { global } = req else {
        unreachable!("run_mem_gc called with a non-gc request")
    };
    let mut conn = open_mem_conn(root_arg, *global, MemoryMode::Mutation)?;
    let (expired, superseded, durable) = crate::db::memory_gc(&mut conn, crate::db::unix_now())?;
    crate::journal::record_result(crate::journal::OutcomeCounts::uniform(1), false, 0);
    Ok(ArmOut::plain(
        format!(
            "{}\nexpired={expired} removed, superseded={superseded} removed, durable={durable} rows\n",
            crate::memory::MEMORY_CAVEAT
        ),
        String::new(),
    ))
}
