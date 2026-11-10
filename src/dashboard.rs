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

#[cfg(test)]
mod tip_tests {
    //! The hover-help copy lives in `app.js` and the hooks in `index.html`.
    //! Neither file is compiled, so a key typo would ship a blank tooltip.
    const HTML: &str = include_str!("dashboard/assets/index.html");
    const JS: &str = include_str!("dashboard/assets/app.js");
    const ATTR: &str = "data-tip=\"";

    fn html_keys() -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut rest = HTML;
        while let Some(pos) = rest.find(ATTR) {
            let tail = &rest[pos + ATTR.len()..];
            let end = tail.find('"').unwrap_or(0);
            out.push(tail[..end].to_string());
            rest = &tail[end..];
        }
        out.sort();
        out.dedup();
        out
    }

    fn copy_keys() -> Vec<String> {
        let start = JS.find("const TIP = {").expect("TIP map missing");
        let block = &JS[start..];
        let end = block.find("};").unwrap_or(block.len());
        let mut out: Vec<String> = Vec::new();
        for line in block[..end].lines() {
            let Some(open) = line.find('\'') else {
                continue;
            };
            let Some(close) = line[open + 1..].find('\'') else {
                continue;
            };
            out.push(line[open + 1..open + 1 + close].to_string());
        }
        out.sort();
        out.dedup();
        out
    }

    #[test]
    fn every_hook_has_copy() {
        for key in html_keys() {
            let needle = format!("'{key}':");
            assert!(JS.contains(&needle), "no tooltip copy for {key}");
        }
    }

    #[test]
    fn every_copy_is_hooked() {
        for key in copy_keys() {
            let needle = format!("{ATTR}{key}\"");
            // Dynamic rows build their hooks in app.js template strings, so a
            // key may be attached from either asset.
            assert!(
                HTML.contains(&needle) || JS.contains(&needle),
                "unused tooltip copy: {key}"
            );
        }
    }
}
