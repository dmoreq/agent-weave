//! MCP-vs-CLI parity conformance: the same fixture must produce byte-identical
//! result text through the MCP server and the CLI. The default registry is the
//! eight core navigation tools; `--all-tools` exposes all 21. Every tool call
//! must journal with surface "mcp".

mod build_telemetry;
mod common;
mod memory;
mod parity;
mod protocol;
