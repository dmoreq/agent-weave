# Repository Agent Rules (`eg` / `engram`)

Binding repository conventions for `eg` (`engram`): a deterministic local code graph, real-time anti-reinvention engine, and zero-token persistent memory store for Python, TypeScript, JavaScript, and Rust.

Read the governing specifications before changing schemas, resolution semantics, memory lifecycle, output formats, or exit codes:
- Code Graph & Resolution: [`docs/architecture/code-index.md`](docs/architecture/code-index.md)
- Agent Memory & Retrieval: [`docs/architecture/memory-store.md`](docs/architecture/memory-store.md)

## 1. Architecture & Safety Invariants (Non-Negotiable)

- **Local-First & Zero Outbound Network**:
  - Zero outbound network access, zero background indexing daemons, zero file watchers, zero Git hooks, and zero host/agent configuration edits in Rust runtime code.
  - Loopback HTTP (`127.0.0.1`) is permitted strictly for the explicit user-invoked `eg dashboard` console via standard library `std::net::TcpListener`.
- **Zero Python Operational Requirement**:
  - All user-facing CLI subcommands, MCP stdio server, web dashboard (`eg dashboard`), and terminal streaming monitor (`eg monitor`) must compile into the single native `eg` Rust binary.
  - Never introduce runtime Python dependencies for operational features. Python is restricted strictly to offline evaluation and invariant checks in `tests/scripts/` and `bench/`.
- **Strict `.engram/` and `~/.engram/` Write Confinement**:
  - Runtime filesystem writes and deletes are confined to `<repo>/.engram/` (index, local memory, local journal) and `~/.engram/` / `$ENGRAM_GLOBAL_DIR` (global memory, global journal).
  - Write/delete primitives may live **only** in top-level [`src/db.rs`](src/db.rs), [`src/doctor.rs`](src/doctor.rs), and [`src/journal.rs`](src/journal.rs) (plus `db_new_path` handling in [`src/build.rs`](src/build.rs)).
  - Files under `src/` must **never** mention agent-configuration tokens (`AGENTS.md`, `CLAUDE.md`, `.claude`, `.codex`, `settings.json`, `keychain`, `credentials`, `hook_file`, `pre-commit`). [`tests/scripts/write_path_check.sh`](tests/scripts/write_path_check.sh) scans `src.rglob("*.rs")` and rejects any match or unauthorized write primitive.
- **Rust & Dependency Discipline**:
  - Maintain MSRV `1.90` (`edition = "2021"`). Avoid `std::env::set_var` / `remove_var` in production code to preserve Rust Edition 2024 compatibility.
  - Zero `unsafe` code anywhere in `src/` or `tests/`.
  - Preserve the minimal synchronous dependency footprint (`clap`, `tree-sitter*`, `rusqlite` bundled, `ignore`, `serde_json`). Do not add async runtimes (`tokio`), external HTTP crates, or unapproved dependencies.
  - Keep [`tests/scripts/allowlist.txt`](tests/scripts/allowlist.txt) synchronized with `Cargo.lock` (`bash tests/scripts/allowlist_check.sh Cargo.lock`).
  - **Module Size & File Discipline**: Target 200–500 LOC per source and test file. Decompose oversized modules into cohesive submodules rather than accumulating monolithic files.

## 2. Codebase Map

| Subsystem | Files | Responsibility |
|---|---|---|
| CLI & Entrypoint | [`src/cli.rs`](src/cli.rs), [`src/main.rs`](src/main.rs), [`src/error.rs`](src/error.rs) | `clap` subcommands, exit-code mapping, latency/result journaling |
| Shared Service Layer | [`src/service.rs`](src/service.rs) | Unified request dispatch, CLI/MCP byte-parity formatting, token-savings estimation, tool schemas |
| Discovery & Parsing | [`src/walk.rs`](src/walk.rs), [`src/parse.rs`](src/parse.rs), [`src/hash.rs`](src/hash.rs) | Git-visible / ignore-aware file enumeration, 1 MiB cap, Tree-sitter parsing, FNV-1a hashing |
| Extraction & Resolution | [`src/extract.rs`](src/extract.rs), [`src/extract/walk/`](src/extract/walk/), [`src/extract/resolve.rs`](src/extract/resolve.rs) | Language-specific AST extraction (`python`, `ts_js`, `rust`) and static call, import, and inheritance resolution |
| Storage & Schemas | [`src/build.rs`](src/build.rs), [`src/db.rs`](src/db.rs), [`schema/index.sql`](schema/index.sql), [`schema/indexes.sql`](schema/indexes.sql), [`schema/memory.sql`](schema/memory.sql) | Advisory `flock` (`build.lock`), incremental carry-forward, atomic `.engram/index.db.new` swap, local/global `memory.db` |
| Query, Reuse & Memory | [`src/query.rs`](src/query.rs), [`src/query/callgraph.rs`](src/query/callgraph.rs), [`src/query/impact.rs`](src/query/impact.rs), [`src/query/analysis.rs`](src/query/analysis.rs), [`src/query/reuse.rs`](src/query/reuse.rs), [`src/query/scope.rs`](src/query/scope.rs), [`src/insights.rs`](src/insights.rs), [`src/memory.rs`](src/memory.rs) | Graph traversal, FTS5 symbol search, D1–D4 anti-reinvention detectors, layered local/global memory retrieval |
| Diagnostics & Telemetry | [`src/doctor.rs`](src/doctor.rs), [`src/journal.rs`](src/journal.rs) | Integrity/staleness checks, `--invalidate`, dual-write usage/payload JSONL journals (`.engram/usage.log` & `~/.engram/usage.log`) |
| Native Dashboard & Monitor | [`src/dashboard.rs`](src/dashboard.rs), [`src/dashboard/server.rs`](src/dashboard/server.rs), [`src/dashboard/data.rs`](src/dashboard/data.rs), [`src/dashboard/monitor/`](src/dashboard/monitor/mod.rs), [`src/dashboard/assets/`](src/dashboard/assets/) | Zero-dependency HTTP 1.1 + SSE web server (`eg dashboard`), embedded UI assets, ANSI live stream (`eg monitor`) |
| Agent Surfaces & Setup | [`src/mcp.rs`](src/mcp.rs), [`skills/engram-first/SKILL.md`](skills/engram-first/SKILL.md), [`scripts/install.sh`](scripts/install.sh) | Newline-delimited JSON-RPC 2.0 MCP stdio server, 3-pillar agent routing policy, multi-agent installer |

## 3. Storage, SQL & Output Contracts

- **Dual-Database Schema Evolution**:
  - **Code Index (`.engram/index.db`, `SCHEMA_VERSION = 10`)**: Evolve [`schema/index.sql`](schema/index.sql) and [`schema/indexes.sql`](schema/indexes.sql) additively. Never migrate `.engram/index.db` in place. Any schema mismatch (`PRAGMA user_version != SCHEMA_VERSION`) must exit `5`; `eg build` replaces the database atomically after `fsync`. Keep secondary indexes in [`schema/indexes.sql`](schema/indexes.sql) so fresh builds create them after bulk inserts.
  - **Memory Store (`.engram/memory.db` & `~/.engram/memory.db`, `MEMORY_SCHEMA_VERSION = 2`)**: Defined in [`schema/memory.sql`](schema/memory.sql). Preserves primary agent-authored notes across index rebuilds with dual FTS5 indexes (legacy ASCII + Unicode), revision chains, content-hash deduplication, and JSONL backup (`eg mem export`).
- **CLI & MCP Parity**:
  - Route all shared query, analysis, and memory operations through [`src/service.rs`](src/service.rs) so CLI and MCP outputs remain byte-identical.
- **SQL Safety & Deterministic Ordering**:
  - Bind all user-supplied filters (`--path`, symbol names, limits) with SQLite parameters (`?1`, `?2`). Never construct `WHERE` clauses via string interpolation.
  - Do not force `INDEXED BY` hints when optional filters (such as `--path`) have a more selective index (`symbols_file_path`).
  - Always include deterministic tie-breaker columns (`qualname`, `file_path`, `line_start`) in `ORDER BY`; rowids and `file_id` vary between fresh and incremental builds.
- **Exit Codes**:
  - `0` ok | `1` empty result, missing target, or missing index | `2` invalid CLI usage or ambiguous bare name (`path:line qualname` candidates) | `3` corrupt DB | `4` active build lock | `5` schema skew | `6` internal or I/O error | `7` parse-error-rate abort.

## 4. Dashboard & UI Hygiene

- **Embedded Offline-First Assets**: Web dashboard assets under [`src/dashboard/assets/`](src/dashboard/assets/) (`index.html`, `style.css`, `app.js`) are compiled into the binary via `include_str!`. Keep them vanilla HTML/CSS/JS with zero external CDNs, zero external web fonts, and zero frontend build steps.
- **Visual & Terminal Discipline**: Follow Zen Minimalism (Shibumi Utilitarian light/dark design tokens). Maintain zero emojis and zero Unicode sparkline/bar characters across the web UI, CLI output, and ANSI monitor.

## 5. Cross-Surface Synchronization Gate

When adding, removing, or modifying any subcommand, flag, MCP tool, output format, or crate name, update **every** affected surface in the same commit:

1. **CLI & Service Dispatch**: [`src/cli.rs`](src/cli.rs), [`src/main.rs`](src/main.rs), and [`src/service.rs`](src/service.rs) (write clean `--help` descriptions; never leak internal spec labels like `Amendment N`, `P4`, `P8`, or `CUJ*`).
2. **MCP**: [`src/mcp.rs`](src/mcp.rs), [`src/service.rs`](src/service.rs) (`tool_specs` / `from_tool_args`), and [`tests/mcp/`](tests/mcp/main.rs).
3. **Agent Skill**: [`skills/engram-first/SKILL.md`](skills/engram-first/SKILL.md) and [`scripts/install.sh`](scripts/install.sh) — enforce the 3-pillar agent workflow:
   - *Pillar 1 (Graph Navigation)*: `build`, `callers`, `callees`, `path`, `context`, `impact`, `imports`, `symbols`.
   - *Pillar 2 (Anti-Reinvention)*: Pre-write `search` (prioritizing `[callers=N]`) and post-edit `reuse`.
   - *Pillar 3 (Project Memory)*: Pre-task `mem query` and post-fix `mem write`.
   - Keep human-audit commands (`tests`, `dead`, `complexity`, `hotspots`, `report`, `stats`, `files`) out of mandatory agent routing.
4. **Public Docs**: [`README.md`](README.md), [`docs/getting-started.md`](docs/getting-started.md), [`docs/usage.md`](docs/usage.md), [`docs/cli-reference.md`](docs/cli-reference.md), [`docs/agent-integration.md`](docs/agent-integration.md), and [`CHANGELOG.md`](CHANGELOG.md).
5. **Packaging**: [`Cargo.toml`](Cargo.toml) (`include` list), [`tests/scripts/allowlist.txt`](tests/scripts/allowlist.txt), [`scripts/install.sh`](scripts/install.sh), and [`.github/workflows/`](.github/workflows/).

## 6. Documentation & Public-Release Hygiene

- **Human-Centric Technical Prose**: Explain features in terms of concrete developer and agent workflows (what changes with `eg` vs `grep`/file reads, static boundaries, and fallback steps). Strip internal milestone codes (`P1`–`P8`, `R6d`, `R10`–`R14`, `CUJ*`, `Amendment N`) from user-facing docs, changelogs, and CLI help.
- **Zero Private Leakage**: Never place workstation paths, tilde home paths, internal repository/symbol names, or employer domains in tracked source, tests, or public docs.

## 7. Verification Commands

- **Resolution Changes**: Add both positive and negative assertions (proving what resolves and what must remain unresolved/ambiguous) in [`tests/fixtures/`](tests/fixtures/main.rs).
- **Targeted Iteration** (run only affected tests while editing):
  ```sh
  cargo test --locked --lib <test_name>
  cargo test --locked --test fixtures <test_name>
  cargo test --locked --test cli <test_name>
  cargo test --locked --test mcp <test_name>
  ```
- **Final Pre-Merge Gate (Runtime Code Changes)**:
  ```sh
  cargo fmt --all -- --check
  cargo clippy --all-targets -- -D warnings
  cargo test --locked
  bash tests/scripts/allowlist_check.sh Cargo.lock
  bash tests/scripts/write_path_check.sh
  bash tests/scripts/hygiene_check.sh
  ```
- **Pre-Commit Hook Installation & Targeted Runs**:
  ```sh
  pre-commit install --hook-type pre-commit --hook-type pre-push
  pre-commit run --files <changed_files>
  ```
- **Documentation-Only Gate** (skip Rust test rebuilds):
  - Validate every documented `eg ...` command/flag against [`src/cli.rs`](src/cli.rs) / `eg <subcommand> --help`.
  - Check relative Markdown links and run `git diff --check`.
