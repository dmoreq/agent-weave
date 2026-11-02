//! Entry point: subcommand dispatch, build orchestration (full atomic
//! replacement with the per-file fast path), and exit-code mapping.
//! Panic-free boundary: a panic aborts with exit 6, never partial stdout.
#![forbid(unsafe_code)]

mod build;
mod cli;
mod clones;
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
    // git read-only commands must not refresh/rewrite `.git/index` — the
    // safety invariant says no writes outside `.kg/` (R6d A8).
    std::env::set_var("GIT_OPTIONAL_LOCKS", "0");
    // A downstream pipe close is not an internal error; keep stderr quiet
    // and let the catch_unwind arm exit 0 (R6 finding).
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("");
        if !msg.contains("Broken pipe") {
            default_hook(info);
        }
    }));
    let cli = cli::Cli::parse();
    let root_arg = cli.root.clone();
    let no_journal = cli.no_journal;
    let journaled = !no_journal
        && !matches!(
            cli.cmd,
            cli::Cmd::Doctor { .. } | cli::Cmd::Usage { .. } | cli::Cmd::Mcp {}
        );
    if journaled {
        journal::begin("cli", tool_name(&cli.cmd), &journal_args(&cli.cmd));
    }
    let started = Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(cli, no_journal)));
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
            // Contract: dead/clones always carry their caveat on stdout,
            // even for an empty result.
            if let error::KgError::NoResultsCaveated { caveat, .. } = &e {
                println!("{caveat}");
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
        Err(panic) => {
            // A closed downstream pipe (e.g. `kg callers x | head -1`) is not
            // an internal failure: exit quietly like a POSIX filter (R6d A8).
            let msg = panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("");
            if msg.contains("Broken pipe") {
                if journaled {
                    journal::finalize(root.as_deref(), 0, latency_us);
                }
                std::process::exit(0);
            }
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
        cli::Cmd::Context { .. } => "context",
        cli::Cmd::Path { .. } => "path",
        cli::Cmd::Impact { .. } => "impact",
        cli::Cmd::Dead { .. } => "dead",
        cli::Cmd::Unresolved { .. } => "unresolved",
        cli::Cmd::Symbols { .. } => "symbols",
        cli::Cmd::Search { .. } => "search",
        cli::Cmd::Clones { .. } => "clones",
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
        cli::Cmd::Build {
            force,
            max_parse_error_rate,
        } => vec![
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
        cli::Cmd::Path { from, to, depth } => vec![
            ("name", from.clone()),
            ("to", to.clone()),
            ("depth", depth.to_string()),
        ],
        cli::Cmd::Context { name, limit } => {
            vec![("name", name.clone()), ("limit", limit.to_string())]
        }
        cli::Cmd::Impact { file, sites, limit } => vec![
            ("file", file.clone()),
            ("sites", sites.to_string()),
            ("limit", limit.to_string()),
        ],
        cli::Cmd::Dead {
            exclude_tests,
            limit,
        } => vec![
            ("exclude_tests", exclude_tests.to_string()),
            ("limit", limit.to_string()),
        ],
        cli::Cmd::Unresolved { limit } => vec![("limit", limit.to_string())],
        cli::Cmd::Symbols {
            file,
            outline,
            limit,
        } => {
            vec![
                ("file", file.clone()),
                ("outline", outline.to_string()),
                ("limit", limit.to_string()),
            ]
        }
        cli::Cmd::Search { terms, limit } => {
            vec![("terms", terms.clone()), ("limit", limit.to_string())]
        }
        cli::Cmd::Clones { min_lines, limit } => {
            vec![
                ("min_lines", min_lines.to_string()),
                ("limit", limit.to_string()),
            ]
        }
        cli::Cmd::Stats {} => vec![],
        cli::Cmd::Hotspots { limit } => vec![("limit", limit.to_string())],
        cli::Cmd::Report {} => vec![],
        cli::Cmd::Doctor { invalidate } => vec![("invalidate", invalidate.to_string())],
        cli::Cmd::Usage { .. } => vec![],
        cli::Cmd::Mcp {} => vec![],
    }
}

fn run(cli: cli::Cli, no_journal: bool) -> Result<()> {
    let root_arg = cli.root.as_deref();
    match cli.cmd {
        cli::Cmd::Build {
            force,
            max_parse_error_rate,
        } => {
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
        cli::Cmd::Context { name, limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in query::context(&conn, &root, &name, limit)? {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Path { from, to, depth } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in query::path(&conn, &root, &from, &to, depth)? {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Impact { file, sites, limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            let out = query::impact(&conn, &root, &file, limit, sites)?;
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
        cli::Cmd::Dead {
            exclude_tests,
            limit,
        } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            let out = query::dead(&conn, &root, limit, exclude_tests)?;
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
        cli::Cmd::Mcp {} => mcp::serve(root_arg, no_journal),
        cli::Cmd::Unresolved { limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in query::unresolved(&conn, limit)? {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Symbols {
            file,
            outline,
            limit,
        } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in query::symbols(&conn, &root, &file, outline, limit)? {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Search { terms, limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in query::search(&conn, &root, &terms, limit)? {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Clones { min_lines, limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in query::clones(&conn, &root, min_lines, limit)? {
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
