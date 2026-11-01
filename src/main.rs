//! Entry point: subcommand dispatch, build orchestration (full atomic
//! replacement with the per-file fast path), and exit-code mapping.
//! Panic-free boundary: a panic aborts with exit 6, never partial stdout.
#![forbid(unsafe_code)]

mod build;
mod cli;
mod db;
mod doctor;
mod error;
mod extract;
mod hash;
mod insights;
mod journal;
mod mcp;
mod parse;
mod query;
mod walk;


use clap::Parser;
use std::time::Instant;

use error::Result;

fn main() {
    let cli = cli::Cli::parse();
    let root_arg = cli.root.clone();
    let no_journal = cli.no_journal;
    let journaled = !no_journal
        && !matches!(cli.cmd, cli::Cmd::Doctor { .. } | cli::Cmd::Usage { .. } | cli::Cmd::Mcp {});
    if journaled {
        journal::begin("cli", tool_name(&cli.cmd), &journal_args(&cli.cmd));
    }
    let started = Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(cli)));
    let latency_us = started.elapsed().as_micros() as u64;
    let root = walk::discover_root(root_arg.as_deref()).ok();
    match result {
        Ok(Ok(())) => {
            if journaled {
                journal::finalize(root.as_deref(), 0, latency_us);
            }
        }
        Ok(Err(e)) => {
            if journaled {
                journal::finalize(root.as_deref(), e.exit_code(), latency_us);
            }
            // CUJ4: ambiguous-name candidates belong on stdout, message on stderr.
            if let error::KgError::Ambiguous { candidates, .. } = &e {
                for (path, line, qualname) in candidates {
                    println!("{path}:{line} {qualname}");
                }
            }
            eprintln!("{e}");
            std::process::exit(e.exit_code());
        }
        Err(_panic) => {
            if journaled {
                journal::finalize(root.as_deref(), 6, latency_us);
            }
            eprintln!("kg: internal: unexpected panic — no output written");
            std::process::exit(6);
        }
    }
}

fn tool_name(cmd: &cli::Cmd) -> &'static str {
    match cmd {
        cli::Cmd::Build { .. } => "build",
        cli::Cmd::Callers { .. } => "callers",
        cli::Cmd::Callees { .. } => "callees",
        cli::Cmd::Impact { .. } => "impact",
        cli::Cmd::Dead { .. } => "dead",
        cli::Cmd::Unresolved { .. } => "unresolved",
        cli::Cmd::Symbols { .. } => "symbols",
        cli::Cmd::Stats {} => "stats",
        cli::Cmd::Hotspots { .. } => "hotspots",
        cli::Cmd::Report {} => "report",
        cli::Cmd::Doctor { .. } => "doctor",
        cli::Cmd::Usage { .. } => "usage",
        cli::Cmd::Mcp {} => "mcp",
    }
}

fn journal_args(cmd: &cli::Cmd) -> Vec<(&'static str, String)> {
    match cmd {
        cli::Cmd::Build { force, max_parse_error_rate } => vec![
            ("force", force.to_string()),
            ("threshold", max_parse_error_rate.to_string()),
        ],
        cli::Cmd::Callers { name, limit } => {
            vec![("name", name.clone()), ("limit", limit.to_string())]
        }
        cli::Cmd::Callees { name, limit, depth } => vec![
            ("name", name.clone()),
            ("depth", depth.to_string()),
            ("limit", limit.to_string()),
        ],
        cli::Cmd::Impact { file, limit } => {
            vec![("file", file.clone()), ("limit", limit.to_string())]
        }
        cli::Cmd::Dead { limit } => vec![("limit", limit.to_string())],
        cli::Cmd::Unresolved { limit } => vec![("limit", limit.to_string())],
        cli::Cmd::Symbols { file, limit } => {
            vec![("file", file.clone()), ("limit", limit.to_string())]
        }
        cli::Cmd::Stats {} => vec![],
        cli::Cmd::Hotspots { limit } => vec![("limit", limit.to_string())],
        cli::Cmd::Report {} => vec![],
        cli::Cmd::Doctor { invalidate } => vec![("invalidate", invalidate.to_string())],
        cli::Cmd::Usage { .. } => vec![],
        cli::Cmd::Mcp {} => vec![],
    }
}

fn run(cli: cli::Cli) -> Result<()> {
    let root_arg = cli.root.as_deref();
    match cli.cmd {
        cli::Cmd::Build { force, max_parse_error_rate } => {
            let line = build::run_build(root_arg, force, max_parse_error_rate)?;
            println!("{line}");
            Ok(())
        }
        cli::Cmd::Callers { name, limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            let out = query::callers(&conn, &root, &name, limit)?;
            journal::record_result(
                out.pointers.len(),
                out.shown,
                out.total,
                out.stale,
                out.pointers.iter().filter(|p| !p.tag.is_empty()).count(),
            );
            query::print_output(&out);
            Ok(())
        }
        cli::Cmd::Callees { name, limit, depth } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            let out = query::callees(&conn, &root, &name, limit, depth)?;
            journal::record_result(
                out.pointers.len(),
                out.shown,
                out.total,
                out.stale,
                out.pointers.iter().filter(|p| !p.tag.is_empty()).count(),
            );
            query::print_output(&out);
            Ok(())
        }
        cli::Cmd::Impact { file, limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            let out = query::impact(&conn, &root, &file, limit)?;
            journal::record_result(
                out.pointers.len(),
                out.shown,
                out.total,
                out.stale,
                out.pointers.iter().filter(|p| !p.tag.is_empty()).count(),
            );
            query::print_output(&out);
            Ok(())
        }
        cli::Cmd::Dead { limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            let out = query::dead(&conn, &root, limit)?;
            journal::record_result(
                out.pointers.len(),
                out.shown,
                out.total,
                out.stale,
                out.pointers.iter().filter(|p| !p.tag.is_empty()).count(),
            );
            query::print_output(&out);
            Ok(())
        }
        cli::Cmd::Usage { verify, limit } => {
            let root = walk::discover_root(root_arg)?;
            if verify {
                let conn = db::open_for_query(&root)?;
                for line in journal::verify(&root, &conn, limit)? {
                    println!("{line}");
                }
            } else {
                for line in journal::summary(&root)? {
                    println!("{line}");
                }
            }
            Ok(())
        }
        cli::Cmd::Mcp {} => mcp::serve(root_arg),
        cli::Cmd::Unresolved { limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in query::unresolved(&conn, limit)? {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Symbols { file, limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in query::symbols(&conn, &root, &file, limit)? {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Stats {} => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            println!("{}", insights::stats(&conn, &root)?);
            Ok(())
        }
        cli::Cmd::Hotspots { limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in insights::hotspots(&conn, &root, limit)? {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Report {} => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in insights::report(&conn, &root)? {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Doctor { invalidate } => {
            let root = walk::discover_root(root_arg)?;
            if invalidate {
                doctor::invalidate(&root)
            } else {
                doctor::report(&root)
            }
        }
    }
}
