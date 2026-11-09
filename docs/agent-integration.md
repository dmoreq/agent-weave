# Agent Integration & Model Context Protocol (MCP)

This document provides setup instructions for integrating `engram` (`eg`) with AI coding agents, including Claude Desktop, Cursor, and Antigravity.

---

## Architecture Overview

`eg` serves AI agents via the Model Context Protocol (MCP) over standard input/output (stdio) using newline-delimited JSON-RPC 2.0.

```text
+-----------------------+                    +------------------------------------+
|                       |  stdio (JSON-RPC)  |          engram (eg)          |
|    AI Coding Agent    | -----------------> |  - In-process SQLite AST Graph     |
| (Claude/Cursor/AntiG) | <----------------- |  - Real-Time Anti-Reinvention      |
|                       |  compact response  |  - Zero-Token Layered Memory       |
+-----------------------+                    +------------------------------------+
```

- **Zero Background Daemons**: The host agent spawns the `eg mcp` process on demand.
- **Zero Runtime Network Access**: Operates 100% locally and offline.
- **Strict Confinement**: Reads repository code; writes exclusively to `.engram/` or `~/.engram/`.

---

## 1-Click Automated Setup

Running `./install.sh` from the repository root automatically discovers and configures all supported active agents on your system (Antigravity, Grok Build, Zcode, OpenCode, Zed, Claude Desktop, and Cursor), installs the `engram-first` skill, and injects routing rules:

```bash
./install.sh
```

---

## Manual Host Configuration

If you prefer to configure agents manually, add `eg` to your agent host's configuration file.

### 1. Claude Desktop
Add to `claude_desktop_config.json` (located at `~/Library/Application Support/Claude/` on macOS or `%APPDATA%\Claude\` on Windows):

```json
{
  "mcpServers": {
    "engram": {
      "command": "eg",
      "args": ["--root", "/path/to/your/repository", "mcp"]
    }
  }
}
```

### 2. Cursor
Add to `.cursor/mcp.json` in your workspace root:

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

### 3. Antigravity / Gemini CLI
Add to `antigravity.json` in your project or global configuration directory:

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

---

## MCP Tools Reference

### Default Core Surface (13 Tools)

By default, `eg mcp` exposes 13 high-ROI tools focused on navigation, anti-reinvention, and persistent memory:

| Tool Name | Parameters | Responsibility |
|---|---|---|
| `build` | `force?`, `max_parse_error_rate?` | Rebuild local AST index (~0.02s incremental) |
| `callers` | `name`, `depth?`, `limit?` | Find incoming caller sites |
| `callees` | `name`, `depth?`, `limit?` | Trace outgoing function calls |
| `path` | `from`, `to`, `depth?` | Shortest call route between symbols |
| `context` | `name`, `limit?` | 1-call definition, callers, and callees |
| `impact` | `file`, `sites?`, `depth?`, `edges?`, `limit?` | Reverse dependency analysis for modified files |
| `imports` | `file`, `limit?` | Outbound resolved imports |
| `symbols` | `file`, `outline?`, `limit?` | File/directory definitions and signatures |
| `search` | `terms`, `limit?` | Discover symbols with interface cards & `[callers=N]` |
| `reuse` | `paths?`, `limit?` | Anti-reinvention duplicate detector |
| `usage` | `quality?` | Session telemetry, latency percentiles, and token savings |
| `mem_write` | `key`, `body`, `kind`, `tags?`, `refs?`, `scope?`, `ttl?`, `supersedes?`, `touch?`, `global?` | Persist architectural decisions and gotchas |
| `mem_query` | `terms?`, `all?`, `kind?`, `scope?`, `limit?`, `include_superseded?`, `global?`, `ref?` | Zero-token layered memory search |

### Extended Surface (`--all-tools`)
Passing `--all-tools` exposes 6 additional memory maintenance tools (19 total): `mem_touch`, `mem_get`, `mem_delete`, `mem_gc`, `mem_export` (JSONL or Markdown), and `mem_import` (reject-on-conflict restore; confined to the project store).

*Note*: `doctor`, `stats`, and `unresolved` are intentionally reserved for human CLI inspection and omitted from MCP stdio dispatch.

---

## Real-Time Response Telemetry

Every MCP tool response appends an execution telemetry footer:

```text
⚡ eg: 1.2ms | ~3.8k tokens saved
```

This footer reflects:
- **Execution Latency**: Wall-clock execution time inside `eg` (consistently sub-2ms for SQLite graph lookups).
- **Estimated Token Savings**: Tokens saved compared to dumping raw file contents to locate the same symbol or call site.

Check aggregate session statistics via the `usage` tool or CLI:

```bash
eg usage --quality
```

---

## Agent Routing Rules (`engram-first`)

To ensure AI models take full advantage of `eg` rather than falling back to slow whole-file reads:
1. Load the repository agent skill from [`skills/engram-first/SKILL.md`](../skills/engram-first/SKILL.md).
2. The skill enforces **query-first routing** for:
   - Locating callers (`callers`)
   - Finding call chains (`path`)
   - Assessing blast radius (`impact`)
   - Discovering reusable helpers (`search`)
   - Checking for code duplication (`reuse`)
   - Retrieving session context and developer preferences (`mem_query`)

---

## Trust Contract

1. **Navigation Rows**: When an `eg` query exits 0, returned `path:line` pointers are ground truth for indexed static relationships. Do not re-verify existing pointers with broad greps.
2. **Behavioral Edits**: Pointers identify *where* to inspect. Always read the exact targeted source lines before performing edits.
3. **Candidates**: Results from `reuse` and `unresolved` represent candidates for inspection, not definitive refactoring mandates.
4. **Staleness Discipline**: Rebuild the index (`eg build`) after editing source files before trusting further navigation queries.
