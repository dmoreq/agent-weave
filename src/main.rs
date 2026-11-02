//! Entry point: subcommand dispatch, build orchestration (full atomic
//! replacement with the per-file fast path), and exit-code mapping.
//! Panic-free boundary: a panic aborts with exit 6, never partial stdout.
#![forbid(unsafe_code)]

use std::io::Read;

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
        cli::Cmd::Implementers { .. } => "implementers",
        cli::Cmd::Imports { .. } => "imports",
        cli::Cmd::Tests { .. } => "tests",
        cli::Cmd::Path { .. } => "path",
        cli::Cmd::Impact { .. } => "impact",
        cli::Cmd::Dead { .. } => "dead",
        cli::Cmd::Unresolved { .. } => "unresolved",
        cli::Cmd::Symbols { .. } => "symbols",
        cli::Cmd::Complexity { .. } => "complexity",
        cli::Cmd::Api { .. } => "api",
        cli::Cmd::Resolve { .. } => "resolve",
        cli::Cmd::Files { .. } => "files",
        cli::Cmd::Search { .. } => "search",
        cli::Cmd::Clones { .. } => "clones",
        cli::Cmd::Stats {} => "stats",
        cli::Cmd::Hotspots { .. } => "hotspots",
        cli::Cmd::Report { .. } => "report",
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
        cli::Cmd::Implementers { name, limit } => {
            vec![("name", name.clone()), ("limit", limit.to_string())]
        }
        cli::Cmd::Imports { file, limit } => {
            vec![("file", file.clone()), ("limit", limit.to_string())]
        }
        cli::Cmd::Tests { name, depth, limit } => vec![
            ("name", name.clone()),
            ("depth", depth.to_string()),
            ("limit", limit.to_string()),
        ],
        cli::Cmd::Impact {
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
        cli::Cmd::Dead {
            exclude_tests,
            exclude_generated,
            path,
            limit,
        } => vec![
            ("exclude_tests", exclude_tests.to_string()),
            ("exclude_generated", exclude_generated.to_string()),
            ("path", path.clone().unwrap_or_default()),
            ("limit", limit.to_string()),
        ],
        cli::Cmd::Unresolved { names, path, limit } => vec![
            ("names", names.to_string()),
            ("path", path.clone().unwrap_or_default()),
            ("limit", limit.to_string()),
        ],
        cli::Cmd::Symbols {
            file,
            outline,
            metrics,
            limit,
        } => {
            vec![
                ("file", file.clone()),
                ("outline", outline.to_string()),
                ("metrics", metrics.to_string()),
                ("limit", limit.to_string()),
            ]
        }
        cli::Cmd::Complexity {
            min_cyc,
            min_loc,
            min_branches,
            min_nest,
            path,
            exclude_generated,
            limit,
        } => vec![
            ("min_cyc", min_cyc.to_string()),
            ("min_loc", min_loc.to_string()),
            ("min_branches", min_branches.to_string()),
            ("min_nest", min_nest.to_string()),
            ("path", path.clone().unwrap_or_default()),
            ("exclude_generated", exclude_generated.to_string()),
            ("limit", limit.to_string()),
        ],
        cli::Cmd::Api {
            path,
            include_private,
            exclude_generated,
            limit,
        } => vec![
            ("path", path.clone()),
            ("include_private", include_private.to_string()),
            ("exclude_generated", exclude_generated.to_string()),
            ("limit", limit.to_string()),
        ],
        cli::Cmd::Resolve { .. } => vec![("candidate_limit", "50".to_string())],
        cli::Cmd::Files { path, limit } => vec![
            ("path", path.clone().unwrap_or_default()),
            ("limit", limit.to_string()),
        ],
        cli::Cmd::Search {
            terms,
            kind,
            path,
            exact,
            limit,
        } => vec![
            ("terms", terms.clone()),
            ("kind", kind.clone().unwrap_or_default()),
            ("path", path.clone().unwrap_or_default()),
            ("exact", exact.to_string()),
            ("limit", limit.to_string()),
        ],
        cli::Cmd::Clones {
            min_lines,
            path,
            exclude_generated,
            limit,
        } => vec![
            ("min_lines", min_lines.to_string()),
            ("path", path.clone().unwrap_or_default()),
            ("exclude_generated", exclude_generated.to_string()),
            ("limit", limit.to_string()),
        ],
        cli::Cmd::Stats {} => vec![],
        cli::Cmd::Hotspots {
            limit,
            path,
            exclude_generated,
        } => vec![
            ("limit", limit.to_string()),
            ("path", path.clone().unwrap_or_default()),
            ("exclude_generated", exclude_generated.to_string()),
        ],
        cli::Cmd::Report {
            path,
            exclude_generated,
        } => vec![
            ("path", path.clone().unwrap_or_default()),
            ("exclude_generated", exclude_generated.to_string()),
        ],
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
        cli::Cmd::Implementers { name, limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            let out = query::implementers(&conn, &root, &name, limit)?;
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
        cli::Cmd::Imports { file, limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            let out = query::imports(&conn, &root, &file, limit)?;
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
        cli::Cmd::Tests { name, depth, limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            let out = query::test_impact(&conn, &root, &name, depth, limit)?;
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
        cli::Cmd::Path { from, to, depth } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in query::path(&conn, &root, &from, &to, depth)? {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Impact {
            file,
            sites,
            depth,
            edges,
            context,
            explain,
            limit,
        } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            let out = query::impact(
                &conn, &root, &file, limit, sites, depth, &edges, context, explain,
            )?;
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
            exclude_generated,
            path,
            limit,
        } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            let out = query::dead(
                &conn,
                &root,
                limit,
                exclude_tests,
                exclude_generated,
                path.as_deref(),
            )?;
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
        cli::Cmd::Unresolved { names, path, limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            let lines = if names {
                query::unresolved_names(&conn, &root, path.as_deref(), limit)?
            } else {
                query::unresolved(&conn, path.as_deref(), limit)?
            };
            for line in lines {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Symbols {
            file,
            outline,
            metrics,
            limit,
        } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in query::symbols(&conn, &root, &file, outline, metrics, limit)? {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Complexity {
            min_cyc,
            min_loc,
            min_branches,
            min_nest,
            path,
            exclude_generated,
            limit,
        } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in query::complexity(
                &conn,
                min_cyc,
                min_loc,
                min_branches,
                min_nest,
                path.as_deref(),
                exclude_generated,
                limit,
            )? {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Api {
            path,
            include_private,
            exclude_generated,
            limit,
        } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in query::api(
                &conn,
                &root,
                &path,
                include_private,
                exclude_generated,
                limit,
            )? {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Resolve { stdin } => {
            if !stdin {
                return Err(crate::error::KgError::Usage(
                    "resolve requires --stdin".to_string(),
                ));
            }
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input)?;
            if input.len() > 1024 * 1024 {
                return Err(crate::error::KgError::Usage(
                    "resolve input exceeds 1 MiB".to_string(),
                ));
            }
            let mut names = Vec::new();
            for line in input.lines() {
                if line.len() > 4096 {
                    return Err(crate::error::KgError::Usage(
                        "resolve line exceeds 4096 bytes".to_string(),
                    ));
                }
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('#') {
                    names.push(trimmed.to_string());
                }
            }
            if names.len() > 1000 {
                return Err(crate::error::KgError::Usage(
                    "resolve accepts at most 1000 names".to_string(),
                ));
            }
            if names.is_empty() {
                return Err(crate::error::KgError::NoResults(
                    "no names on stdin".to_string(),
                ));
            }
            crate::journal::begin(
                "cli",
                "resolve",
                &[
                    ("count", names.len().to_string()),
                    ("candidate_limit", "50".to_string()),
                ],
            );
            let (lines, exit) = query::resolve_names(&conn, &names)?;
            for line in &lines {
                println!("{line}");
            }
            journal::record_result(lines.len(), lines.len(), lines.len(), false, 0);
            if exit != 0 {
                journal::finalize(Some(&root), exit, 0);
                std::process::exit(exit);
            }
            Ok(())
        }
        cli::Cmd::Files { path, limit } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in query::files(&conn, path.as_deref(), limit)? {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Search {
            terms,
            kind,
            path,
            exact,
            limit,
        } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in query::search(
                &conn,
                &root,
                &terms,
                kind.as_deref(),
                path.as_deref(),
                exact,
                limit,
            )? {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Clones {
            min_lines,
            path,
            exclude_generated,
            limit,
        } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in query::clones(
                &conn,
                &root,
                min_lines,
                path.as_deref(),
                exclude_generated,
                limit,
            )? {
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
        cli::Cmd::Hotspots {
            limit,
            path,
            exclude_generated,
        } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in insights::hotspots(&conn, &root, limit, path.as_deref(), exclude_generated)?
            {
                println!("{line}");
            }
            Ok(())
        }
        cli::Cmd::Report {
            path,
            exclude_generated,
        } => {
            let root = walk::discover_root(root_arg)?;
            let conn = db::open_for_query(&root)?;
            for line in insights::report(&conn, &root, path.as_deref(), exclude_generated)? {
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
