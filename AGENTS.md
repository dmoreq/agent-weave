# Repository Agent Rules (`aw` / `agent-weave`)

Binding repository conventions for `aw`, a local static code index for Python, TypeScript, JavaScript, and Rust. Read [`docs/designs/2026-10-31_agent-weave-rust-code-index.md`](docs/designs/2026-10-31_agent-weave-rust-code-index.md) before changing schema, resolution semantics, output formats, or exit codes.

## 1. Architecture & Safety Invariants (Non-Negotiable)

- **Consultative Local Tool Only**: Zero runtime network access, zero background daemons, zero file watchers, zero Git hooks, and zero host/agent configuration edits.
- **Strict `.aw/` Write Confinement**:
  - Runtime filesystem writes and deletes may live **only** in [`src/db.rs`](src/db.rs), [`src/doctor.rs`](src/doctor.rs), and [`src/journal.rs`](src/journal.rs) (plus `db_new_path` handling in [`src/build.rs`](src/build.rs)).
  - Files under `src/` must **never** mention agent-configuration tokens (`AGENTS.md`, `CLAUDE.md`, `.claude`, `.codex`, `settings.json`, `keychain`, `credentials`, `hook_file`, `pre-commit`). [`tests/scripts/write_path_check.sh`](tests/scripts/write_path_check.sh) rejects any match.
  - [`tests/scripts/write_path_check.sh`](tests/scripts/write_path_check.sh) scans `src.rglob("*.rs")`, so nested modules stay covered; its negative controls include a nested-subdirectory write.
- **Rust & Dependency Discipline**:
  - Maintain MSRV `1.90` (`edition = "2021"`). Avoid `std::env::set_var` / `remove_var` in production code to keep Rust Edition 2024 compatibility open.
  - Zero `unsafe` code anywhere in `src/` or `tests/`.
  - Keep the synchronous dependency footprint (`clap`, `tree-sitter*`, `rusqlite` bundled, `ignore`, `serde_json`). Do not add async runtimes, HTTP clients, or unapproved crates.
  - Keep [`tests/scripts/allowlist.txt`](tests/scripts/allowlist.txt) synchronized with `Cargo.lock` (`bash tests/scripts/allowlist_check.sh Cargo.lock`).

## 2. Codebase Map

| Subsystem | Files | Responsibility |
|---|---|---|
| CLI & Entrypoint | [`src/cli.rs`](src/cli.rs), [`src/main.rs`](src/main.rs), [`src/error.rs`](src/error.rs) | `clap` subcommands, exit-code mapping, latency/result journaling |
| Discovery & Parsing | [`src/walk.rs`](src/walk.rs), [`src/parse.rs`](src/parse.rs), [`src/hash.rs`](src/hash.rs) | Git-visible / ignore-aware file enumeration, 1 MiB cap, Tree-sitter parsing, FNV-1a hashing |
| Extraction & Resolution | [`src/extract.rs`](src/extract.rs), [`src/extract/walk.rs`](src/extract/walk.rs), [`src/extract/resolve.rs`](src/extract/resolve.rs) | Symbol/signature/metric extraction and static call, import, and inheritance resolution |
| Storage & Build | [`src/build.rs`](src/build.rs), [`src/db.rs`](src/db.rs), [`schema.sql`](schema.sql), [`schema_indexes.sql`](schema_indexes.sql) | Advisory `flock` (`build.lock`), incremental carry-forward, atomic `.aw/index.db.new` replacement |
| Query & Analysis | [`src/query.rs`](src/query.rs), [`src/query/callgraph.rs`](src/query/callgraph.rs), [`src/query/impact.rs`](src/query/impact.rs), [`src/query/analysis.rs`](src/query/analysis.rs), [`src/query/scope.rs`](src/query/scope.rs), [`src/insights.rs`](src/insights.rs), [`src/clones.rs`](src/clones.rs) | Graph traversal, FTS5 symbol search, staleness headers, complexity/clone/hotspot reports |
| Diagnostics & Telemetry | [`src/doctor.rs`](src/doctor.rs), [`src/journal.rs`](src/journal.rs) | Integrity/staleness checks, `--invalidate`, usage/payload JSONL journals (`serde_json`) |
| Agent Surfaces | [`src/mcp.rs`](src/mcp.rs), [`skills/aw-first/SKILL.md`](skills/aw-first/SKILL.md) | Newline-delimited JSON-RPC 2.0 MCP stdio server and agent routing policy |

## 3. Storage, SQL & Output Contracts

- **Schema Evolution**:
  - Evolve [`schema.sql`](schema.sql) and [`schema_indexes.sql`](schema_indexes.sql) additively. Never migrate `.aw/index.db` in place.
  - Any schema mismatch (`PRAGMA user_version != SCHEMA_VERSION`) must exit `5`; `aw build` replaces the database atomically after `fsync`.
  - Keep secondary indexes in [`schema_indexes.sql`](schema_indexes.sql) so fresh builds create them after bulk inserts while preserving identical schema structure across fresh and incremental builds.
- **SQL Safety & Query Plans**:
  - Bind all user-supplied filters (`--path`, symbol names, limits) with SQLite parameters (`?1`, `?2`). Never build `WHERE` clauses via string replacement.
  - Do not force `INDEXED BY` hints when optional filters (such as `--path`) have a more selective index (`symbols_file_path`).
  - Always include deterministic tie-breaker columns (`qualname`, `file_path`, `line_start`) in `ORDER BY`; rowids and `file_id` vary across fresh vs carried builds.
- **Exit Codes**:
  - `0` ok | `1` empty result, missing target, or missing index | `2` invalid CLI usage or ambiguous bare name (`path:line qualname` candidates) | `3` corrupt DB | `4` active build lock or parse-error-rate abort | `5` schema skew | `6` internal or I/O error.

## 4. Cross-Surface Synchronization Gate

When adding, removing, or modifying any subcommand, flag, output format, or crate name, update **every** surface in the same commit:

1. **CLI**: [`src/cli.rs`](src/cli.rs) and [`src/main.rs`](src/main.rs) (write clean `--help` descriptions; never leak internal spec labels like `Amendment N`, `P4`, `P8`, or `CUJ*`).
2. **MCP**: [`src/mcp.rs`](src/mcp.rs) and [`tests/mcp.rs`](tests/mcp.rs).
3. **Agent Skill**: [`skills/aw-first/SKILL.md`](skills/aw-first/SKILL.md) — enforce *query-first* routing strictly on proven high-ROI graph commands (`build`, `path`, `context`, `callers`, `callees`, `impact`, `imports`); keep noisy or human-audit commands (`tests`, `dead`, `complexity`, `hotspots`, `report`, `stats`, `files`) out of mandatory agent routing.
4. **Public Docs**: [`README.md`](README.md), [`docs/getting-started.md`](docs/getting-started.md), [`docs/usage.md`](docs/usage.md), [`docs/cli-reference.md`](docs/cli-reference.md), [`docs/agent-integration.md`](docs/agent-integration.md), and [`CHANGELOG.md`](CHANGELOG.md).
5. **Packaging**: [`Cargo.toml`](Cargo.toml) (`include` list), [`tests/scripts/allowlist.txt`](tests/scripts/allowlist.txt), and [`.github/workflows/`](.github/workflows/).

## 5. Documentation & Public-Release Hygiene

- **Human-Centric Technical Prose**: Explain features in terms of concrete developer and agent workflows (what changes with `aw` vs `grep`/file reads, static boundaries, and fallback steps). Strip internal milestone codes (`P1`–`P8`, `R6d`, `R10`–`R14`, `CUJ*`, `Amendment N`) from user-facing docs, changelogs, and CLI help.
- **Zero Private Leakage**: Never place workstation paths, tilde home paths, internal repository/symbol names, or employer domains in tracked source, tests, or public docs.

## 6. Verification Commands

- **Resolution Changes**: Add both positive and negative assertions (proving what resolves and what must remain unresolved/ambiguous) in [`tests/fixtures.rs`](tests/fixtures.rs).
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
  ```
- **Documentation-Only Gate** (skip Rust test rebuilds):
  - Validate every documented `aw ...` command/flag against [`src/cli.rs`](src/cli.rs) / `aw <subcommand> --help`.
  - Check relative Markdown links and run `git diff --check`.
