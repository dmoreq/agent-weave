//! Entry point: parse the CLI, hand it to [`service::execute`], and map the
//! outcome to stdout/stderr and the exit code. Panic-free boundary: a panic
//! aborts with exit 6, never partial stdout.
#![forbid(unsafe_code)]

use agent_weave::{cli, error, journal, mcp, service, walk};

use clap::Parser;
use error::AwError;

fn main() {
    quiet_pipe_hook();
    let cli = cli::Cli::parse();
    let root_arg = cli.root.clone();
    let no_journal = cli.no_journal;
    // The MCP server is a protocol loop, not a one-shot command; it journals
    // each tool call itself. Doctor and usage read (not write) the journal.
    let journaled = !no_journal
        && !matches!(
            cli.cmd,
            cli::Cmd::Doctor { .. }
                | cli::Cmd::Usage { .. }
                | cli::Cmd::Mcp { .. }
                | cli::Cmd::Dashboard { .. }
                | cli::Cmd::Monitor { .. }
        );
    let started = std::time::Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(cli, no_journal, journaled)
    }));
    let latency_us = started.elapsed().as_micros() as u64;
    let root = walk::discover_root(root_arg.as_deref()).ok();
    let exit = exit_code_of(result, root.as_deref(), latency_us, journaled);
    std::process::exit(exit);
}

/// A downstream pipe close is not an internal error; keep stderr quiet and
/// let the catch_unwind arm exit 0 (R6 finding).
fn quiet_pipe_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !panic_text(info.payload()).contains("Broken pipe") {
            default_hook(info);
        }
    }));
}

/// Payload text for both panic surfaces (pipe hook and the catch_unwind arm):
/// `String` and `&str` payloads carry the message, others render empty.
fn panic_text(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("")
}

/// Map the run outcome to an exit code. A closed downstream pipe
/// (`aw callers x | head -1`) exits 0 quietly like a POSIX filter (R6d A8).
fn exit_code_of(
    result: std::thread::Result<Result<i32, AwError>>,
    root: Option<&std::path::Path>,
    latency_us: u64,
    journaled: bool,
) -> i32 {
    match result {
        Ok(Ok(code)) => code,
        Ok(Err(e)) => {
            eprintln!("{e}");
            e.exit_code()
        }
        Err(panic) => {
            let msg = panic_text(&panic);
            if msg.contains("Broken pipe") {
                if journaled {
                    journal::finalize(root, 0, latency_us);
                }
                std::process::exit(0);
            }
            if journaled {
                journal::finalize(root, 6, latency_us);
            }
            eprintln!("aw: internal: unexpected panic — no output written");
            6
        }
    }
}

/// Execute one command and write its output. Output happens here (inside the
/// panic boundary) so a closed downstream pipe exits 0, not 6 (R6 finding).
fn run(cli: cli::Cli, no_journal: bool, journaled: bool) -> Result<i32, AwError> {
    let (root, cmd) = (cli.root, cli.cmd);
    if let cli::Cmd::Mcp { all_tools } = cmd {
        mcp::serve(root.as_deref(), no_journal, all_tools)?;
        return Ok(0);
    }
    if let cli::Cmd::Dashboard {
        port,
        host,
        open,
        local,
        global: _,
    } = cmd
    {
        agent_weave::dashboard::run_dashboard(root.as_deref(), &host, port, open, local)?;
        return Ok(0);
    }
    if let cli::Cmd::Monitor {
        repo,
        errors_only,
        local,
    } = cmd
    {
        agent_weave::dashboard::run_monitor(root.as_deref(), repo.as_deref(), errors_only, local)?;
        return Ok(0);
    }
    let req = service::CommandRequest::from_cli(cmd)?;
    let exec = service::execute(&req, root.as_deref(), service::Surface::Cli, journaled);
    print!("{}", exec.stdout);
    eprint!("{}", exec.stderr);
    Ok(exec.exit_code)
}
