# engram (`eg`)

[![CI](https://github.com/dmoreq/engram/actions/workflows/ci.yml/badge.svg)](https://github.com/dmoreq/engram/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue.svg)](#license)
[![Rust 1.90+](https://img.shields.io/badge/rust-1.90%2B-orange.svg)](docs/getting-started.md#requirements)
[![Offline](https://img.shields.io/badge/runtime-100%25%20offline-success.svg)](#security-and-confinement)
[![MCP](https://img.shields.io/badge/protocol-MCP%202025--06--18-purple.svg)](docs/agent-integration.md)

Local deterministic code graph, real-time anti-reinvention engine, and zero-token persistent memory for AI coding agents.

Single native binary. Zero background daemons. Zero runtime network access. Zero writes outside `.engram/`.

---

## The 3 Core USPs

```text
+-----------------------------------------------------------------------------------+
|                                    engram (eg)                                    |
| Single native binary | 0 daemons | 0 network requests | Strict .engram/           |
+-----------------------------------------------------------------------------------+
        |                                    |                                    |
        v                                    v                                    v
+-----------------------+  +-----------------------------------+  +-----------------------+
|        USP 1          |  |               USP 2               |  |         USP 3         |
|  Deterministic Graph  |  |      Anti-Reinvention Engine      |  |  Zero-Token Layered   |
|      Navigation       |  |   (arXiv:2609.35357 RepoReuse)    |  |        Memory         |
|                       |  |                                   |  | (arXiv:2607.29377)    |
| • < 2ms query latency |  | • Pre-write: eg search interface  |  |                       |
| • 0.02s rebuild       |  |   cards with [callers=N]          |  | • 0 LLM tokens tax    |
| • -27.3% input tokens |  | • Post-edit: instant build hints  |  | • Local: repo context |
| • 98.6% recall        |  | • Calibrated detector (Cdup >= 80%)|  | • Global: dev profile |
+-----------------------+  +-----------------------------------+  +-----------------------+
```

### 1. Sub-2ms Deterministic Code Navigation (Zero Daemon, Zero Token Waste)
Large Language Models waste context windows dumping whole source files to find call sites. `eg` parses Python, TypeScript, JavaScript, and Rust into a local SQLite AST graph:
- **Sub-2ms query latency** across multi-thousand file codebases.
- **0.02s–0.03s incremental rebuilds** via content-addressed AST hashing.
- **Empirical impact**: **98.6% recall** (vs 81.9% grep-only), **-27.3% input tokens**, and **-57% to -86% tool calls**.
- **Deterministic pointers**: `path:line qualname` outputs allow agents to inspect exact slices instead of burning tokens on file scans.

### 2. Real-Time Anti-Reinvention Engine (Prevents Duplicate Code & Exploration Collapse)
Unguided AI coding agents duplicate existing logic at alarming rates (up to 50.8% duplicate code by Turn 5 under standard multi-turn benchmarks, as documented in RepoReuse, [arXiv:2609.35357](https://arxiv.org/abs/2609.35357)). `eg` provides two-moment protection:
- **Pre-write Discovery**: `eg search` returns zero-body Concise Interface Cards ranked by `BM25 + log(1 + callers)` to surface canonical functions before agents author new code.
- **Post-edit Defense**: Incremental `eg build` emits instant stderr hints (< 1ms), while `eg reuse` runs four calibrated detectors ($C_{dup}$ callee footprint $\ge 80\%$, interface token overlap $\ge 60\%$, client bypass, and 1-caller wrappers).

### 3. Zero-Token Code-Anchored Project & Global Memory (Unified Single Binary)
External memory MCP servers introduce an expensive "Memory Tax" through prompt-injected summaries and LLM-based extractions. Based on Zero-Mem ([arXiv:2607.29377](https://arxiv.org/abs/2607.29377)), `eg` implements zero-token persistent memory:
- **0 LLM tokens consumed**: Pure SQLite FTS5 BM25 + deterministic recency/ref score fusion in under 2ms.
- **Layered storage**: Local `.engram/memory.db` for repository decisions, gotchas, and task handoffs; global `~/.engram/memory.db` (`--global`) for developer preferences and cross-project conventions.
- **Code-anchored references**: Notes anchor directly to AST symbols via `--refs`. When code refactors invalidate an anchor, `eg` flags `[stale-ref]` automatically.
- **Unified binary**: Replaces separate memory servers (`ai-memory`) with a single toolchain.

---

## Workflow Comparison Matrix

| Metric / Capability | Grep / Whole-File Reads | Language Server Protocol (LSP) | `engram` (`eg`) |
|---|---|---|---|
| **Query Latency** | 100ms–5000ms | 50ms–2000ms (daemon startup) | **< 2ms** (in-process SQLite) |
| **Incremental Rebuild** | N/A (re-scans every time) | Continuous background indexing | **0.02s** on edit |
| **Token Consumption** | High (dumps whole files) | High (verbose JSON structures) | **-27.3% input tokens** (compact `path:line`) |
| **Tool Call Volume** | 5–15 calls per lookup | 3–8 protocol exchanges | **1–2 targeted queries** (-57% to -86%) |
| **Multi-Hop Recall** | 81.9% (text search misses) | Variable across dynamic edges | **98.6% deterministic graph** |
| **Anti-Reinvention** | None (silent duplicates) | None | **Pre-write cards + post-edit hints** |
| **Cross-Session Memory** | None (wiped on reset) | None | **Zero-token local + global memory** |
| **Runtime Overhead** | High CPU during search | Persistent background daemon | **Zero daemons, zero background threads** |

---

## 60-Second Quickstart

### Installation
Build from source and configure all active AI agents automatically:

```bash
git clone https://github.com/dmoreq/engram.git
cd engram
./install.sh
```

Or install binary only via Cargo:
```bash
cargo install --path . --locked
```

Confirm the installation:
```bash
eg --version
```

### Quick Walkthrough

```bash
# 1. Build the local index
eg build

# 2. USP 2: Discover canonical interfaces before writing code
eg search "read bounded"
# Output: src/walk.rs:42 fn read_bounded(path: &Path, max_bytes: usize) -> Result<Vec<u8>> [callers=14]

# 3. USP 1: Inspect surroundings and callers in sub-2ms
eg context read_bounded
eg callers read_bounded --depth 2

# 4. USP 2: Verify zero duplicate functions after edits
eg reuse
# Optional: extend D1 to second-hop callees for wrappers through a helper
eg reuse --deep-callee

# 5. USP 3: Persist cross-project developer preferences (0 tokens)
eg mem write dev-profile --kind decision --global \
  --body "Preferred stack: Rust. Coding style: Minimalist. Reject unrequested dependencies."

# 6. USP 3: Persist a code-anchored repository decision
eg mem write resolver-strategy --kind decision \
  --body "Resolver maintains explicit confidence tiers. Ambiguous calls remain dst_id NULL." \
  --refs src/extract/resolve.rs
```

---

## Core Command Reference

The 10 core command surfaces cover complete code navigation, anti-reinvention, and memory workflows:

| Subcommand | Primary Purpose | Key Output / Behavior |
|---|---|---|
| `eg build` | Compile or incrementally update AST index | Rebuilds in ~0.02s; atomic swap |
| `eg callers <symbol>` | Find incoming callers (direct or transitive) | `path:line qualname [depth=N]` |
| `eg callees <symbol>` | Trace outgoing function calls | Resolved call targets by depth |
| `eg context <symbol>` | 1-call definition, callers, and callees | Consolidated symbol surroundings |
| `eg path <from> <to>` | Find shortest call route between symbols | Exact hop sequence with call sites |
| `eg impact <file>` | Identify files calling into target file | File grouping with caller counts |
| `eg imports <file>` | Outbound resolved imports for a file | Module import targets |
| `eg search <terms>` | BM25 + caller-weighted symbol discovery | Interface cards with `[callers=N]` |
| `eg symbols <path>` | File/directory outline with signatures | `path:line kind qualname (signature)` |
| `eg reuse [paths...]` | Detect duplicate functions and wrappers | 4 calibrated anti-reinvention detectors |

### Platform & Maintenance Commands

- `eg mem write|query|get|delete|export|import|gc`: Zero-token project and global memory management (Markdown/JSONL export and reject-on-conflict import).
- `eg doctor [--invalidate]`: Verify index integrity, staleness, or reset derived data.
- `eg stats [--path <dir>]`: Build-time summary of files, symbols, edges, and index bytes.
- `eg unresolved [--names]`: Inspect where static resolution could not resolve call sites.
- `eg usage [--global] [--quality]`: View latency distribution, token savings, and query success telemetry (local or machine-wide).
- `eg dashboard [--port 7890] [--open]`: Native live streaming Web SSE dashboard (Telemetry, Anti-Reinvention, Memory).
- `eg monitor [--repo <NAME>] [--errors-only]`: Real-time terminal ANSI query event stream.
- `eg-dashboard`: Convenience launcher for native web dashboard.
- `eg mcp`: Start the Model Context Protocol stdio server.

See the complete [CLI Reference](docs/cli-reference.md) for flag details and exit codes.

---

## Agent Integration (MCP)

`eg` integrates seamlessly with AI coding agents via the Model Context Protocol (MCP).

### 1-Click Host Configurations

#### Claude Desktop (`claude_desktop_config.json`)
```json
{
  "mcpServers": {
    "engram": {
      "command": "eg",
      "args": ["--root", "/absolute/path/to/repository", "mcp"]
    }
  }
}
```

#### Cursor (`.cursor/mcp.json`)
```json
{
  "mcpServers": {
    "engram": {
      "command": "eg",
      "args": ["--root", "${workspaceFolder}", "mcp"]
    }
  }
}
```

#### Antigravity / Gemini CLI (`antigravity.json`)
```json
{
  "mcpServers": {
    "engram": {
      "command": "eg",
      "args": ["--root", ".", "mcp"]
    }
  }
}
```

### Real-Time Performance & Telemetry Footer
Every MCP response includes a compact execution summary:
```text
⚡ eg: 1.2ms | ~3.8k tokens saved
```
Verify aggregate session savings at any time:
```bash
eg usage --quality
```

For complete agent routing instructions and the `engram-first` skill, see [Agent Integration](docs/agent-integration.md).

---

## Security and Confinement

- **Zero Runtime Network Access**: `eg` makes no network connections during operation.
- **Strict Filesystem Confinement**: All runtime writes and deletes are confined strictly to `.engram/` inside the target repository root, plus `~/.engram/` for global memory. `eg` never edits source files, git configuration, hooks, or agent settings.
- **Zero Background Daemons**: No background processes, file watchers, or persistent sockets. Every command runs synchronously and exits immediately.
- **Deterministic and Audited**: Zero `unsafe` Rust code in `src/` or `tests/`. Dependencies are pinned and strictly allowlisted.

See [Security Policy](SECURITY.md) for full audit parameters.

---

## Documentation

- [Getting Started](docs/getting-started.md): Installation and first 5 minutes.
- [Usage Guide](docs/usage.md): Developer workflows, anti-reinvention scenarios, and handoffs.
- [CLI Reference](docs/cli-reference.md): Full flag specifications, exit codes, and output contracts.
- [Agent Integration](docs/agent-integration.md): MCP setup and routing rules.
- [Concepts](docs/concepts.md): Graph data model, confidence tiers, and resolution boundaries.
- [Documentation Index](docs/README.md): Complete repository documentation map.

---

## License

Dual-licensed under either:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
