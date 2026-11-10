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

### After Upgrading the Binary

A running MCP server keeps the binary image it started with, so an upgrade does
not reach it. Restart the sessions that host `eg mcp` after every install:

```bash
pkill -f 'eg mcp'
```

Hosts respawn the server on the next tool call. Until that happens, usage lines
keep whatever the old build wrote, and one agent can appear under two names in
`eg usage` and the dashboard.

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
| `reuse` | `paths?`, `limit?`, `deep_callee?` | Anti-reinvention duplicate detector; `deep_callee` adds second-hop callee footprints |
| `mem_query` | `terms?`, `all?`, `kind?`, `scope?`, `limit?`, `include_superseded?`, `global?`, `ref?` | Zero-token layered memory search; uses the code index for call-graph expansion when available |
| `usage` | `quality?` | Session telemetry, latency percentiles, and token savings |
| `mem_write` | `key`, `body`, `kind`, `tags?`, `refs?`, `scope?`, `ttl?`, `supersedes?`, `touch?`, `global?` | Persist architectural decisions and gotchas |

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

### Session Tagging

Export `ENGRAM_SESSION` before a work session to attribute its calls and memory
writes to one bucket:

```bash
export ENGRAM_SESSION=refactor-auth-3
```

Without it every call is filed under one synthetic session, so the dashboard's
Session Recall Rate has nothing to divide. The tile shows a dash and asks for the
tag when no call carries one.

### Unattributed Calls

A call that resolves no repository is journaled with `"attribution":"no_root"`
and appears in the dashboard under `(unattributed)`. That bucket is separate from
`workspace` so a repository actually named `workspace` stays readable.

### Repository Identity

A repository is identified by its canonical absolute path, and the dashboard
keys `repo_aggs` by that path while showing the directory name in `display`.
Two checkouts with the same directory name therefore stay apart. A `--root`
that points inside a work tree resolves to the work tree root, so a
subdirectory cannot stand in for a repository; a directory outside any work
tree remains its own repository.

---

## Agent Routing Rules (`engram-first`)

Empirical research in [RepoReuse](https://arxiv.org/abs/2609.35357) (*Ma et al., 2026*) and [SWE-CI](https://arxiv.org/abs/2603.03823) (*Chen et al., 2026*) demonstrates that unguided coding agents degrade severely over multi-turn tasks: repository exploration recall collapses to 35.4%, duplicate code accumulates in 50.8% of tasks, and over 75% of CI maintenance loops suffer regressions without explicit architectural grounding.

To ensure AI models take full advantage of `eg` rather than falling back to slow whole-file reads and reinventing logic, `eg` configures a **3-pillar routing scaffold**:
1. Load the repository agent skill from [`skills/engram-first/SKILL.md`](../skills/engram-first/SKILL.md).
2. The skill enforces **query-first routing** across three non-overlapping pillars:
   - **Pillar 1: Graph Navigation**: Locating callers (`callers`), tracing call chains (`path`), and assessing blast radius (`impact`) before modifying code.
   - **Pillar 2: Anti-Reinvention**: Running `search` before writing new helpers to close the exploration gap via interface cards; running `reuse` (`--deep-callee`) post-edit to prevent duplicate logic.
   - **Pillar 3: Zero-Token Memory**: Querying prior architectural decisions (`mem_query`) before starting tasks; persisting durable learnings and gotchas with AST anchors (`mem_write --refs`) upon completion.

---

## Trust Contract

1. **Navigation Rows**: When an `eg` query exits 0, returned `path:line` pointers are ground truth for indexed static relationships. Do not re-verify existing pointers with broad greps.
2. **Behavioral Edits**: Pointers identify *where* to inspect. Always read the exact targeted source lines before performing edits.
3. **Candidates**: Results from `reuse` and `unresolved` represent candidates for inspection, not definitive refactoring mandates.
4. **Staleness Discipline**: Rebuild the index (`eg build`) after editing source files before trusting further navigation queries.
