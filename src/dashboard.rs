//! Interactive telemetry, anti-reinvention, and memory dashboard module.
//! Zero-dependency native Rust HTTP 1.1 + SSE server and ANSI query monitor.

pub mod data;
pub mod monitor;
pub mod server;

use std::path::Path;

use crate::error::Result;

pub fn run_dashboard(
    root: Option<&Path>,
    host: &str,
    port: u16,
    open: bool,
    local: bool,
) -> Result<()> {
    server::serve_dashboard(root, host, port, open, local)
}

pub fn run_monitor(
    root: Option<&Path>,
    repo_filter: Option<&str>,
    errors_only: bool,
    local: bool,
) -> Result<()> {
    monitor::stream_monitor(root, repo_filter, errors_only, local)
}
