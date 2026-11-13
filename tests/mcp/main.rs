//! MCP-vs-CLI parity conformance: the same fixture must produce byte-identical
//! result text through the MCP server and the CLI. The default registry is the
//! 13 core navigation tools; `--all-tools` exposes the full registry. Every
//! tool call must journal with surface "mcp".

mod build_telemetry;
mod common;
mod memory;
mod memory_import;
mod memory_touch;
mod memory_window;
mod parity;
mod protocol;
mod reuse_phase;
