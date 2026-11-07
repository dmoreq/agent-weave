//! Entry point: parse the CLI, hand it to [`service::execute`], and map the
//! outcome to stdout/stderr and the exit code. Panic-free boundary: a panic
//! aborts with exit 6, never partial stdout.
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
mod memory;
mod parse;
mod query;
mod service;
mod walk;

use clap::Parser;
use error::AwError;

fn main() {
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
    // The MCP server is a protocol loop, not a one-shot command; it journals
    // each tool call itself. Doctor and usage read (not write) the journal.
    let journaled = !no_journal
        && !matches!(
            cli.cmd,
            cli::Cmd::Doctor { .. } | cli::Cmd::Usage { .. } | cli::Cmd::Mcp { .. }
        );
    let started = std::time::Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(cli, no_journal, journaled)
    }));
    let latency_us = started.elapsed().as_micros() as u64;
    let root = walk::discover_root(root_arg.as_deref()).ok();
    let exit = match result {
        Ok(Ok(code)) => code,
        Ok(Err(e)) => {
            eprintln!("{e}");
            e.exit_code()
        }
        Err(panic) => {
            // A closed downstream pipe (e.g. `aw callers x | head -1`) is not
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
            eprintln!("aw: internal: unexpected panic — no output written");
            6
        }
    };
    std::process::exit(exit);
}

/// Execute one command and write its output. Output happens here (inside the
/// panic boundary) so a closed downstream pipe exits 0, not 6 (R6 finding).
fn run(cli: cli::Cli, no_journal: bool, journaled: bool) -> Result<i32, AwError> {
    let (root, cmd) = (cli.root, cli.cmd);
    if let cli::Cmd::Mcp { all_tools } = cmd {
        mcp::serve(root.as_deref(), no_journal, all_tools)?;
        return Ok(0);
    }
    let req = service::CommandRequest::from_cli(cmd)?;
    let exec = service::execute(&req, root.as_deref(), service::Surface::Cli, journaled);
    print!("{}", exec.stdout);
    eprint!("{}", exec.stderr);
    Ok(exec.exit_code)
}
