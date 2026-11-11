# Changelog

All notable changes to engram are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning is
[Calm SemVer](https://calver.org) 0.x — breaking contract
changes bump the minor while 0.x is unstable.

## Reading this history

- Start with [getting started](docs/getting-started.md) for installation and rebuild steps, not historical rollout instructions.
- Entries describe their dated implementation snapshot. Later entries can supersede earlier schemas, limits, and output formats.
- Historical measurements and review verdicts below are internally reported, not new verification of this checkout.
  Original internal evidence is unavailable in the public distribution; this history does not make it publicly reproducible.
- Read [evaluation](docs/evaluation.md) for the public evidence summary and its limitations, not replacement original evidence.
- Internal evaluation corpus names are neutralized as `corpus-a` and `corpus-b`.

## [Unreleased]

## [1.0.3] - 2026-11-11

### Changed
- Reuse candidates now carry per-candidate usage: each `eg reuse` run
  journals the surfaced `symbol->canonical` pairs (capped at 25) in
  `args.candidates`, and the dashboard aggregates `reuse_count` and
  `last_reused_ts` from the retained journal. Tab 02 cards show
  "surfaced N× · last used <date>" (or "never surfaced in retained journal").
- Both live feeds (Recent Queries on tab 01, Pre-Write Discovery on tab 02)
  show only the newest ten rows, with a "Showing X of Y" counter; the full
  journal stays queryable through the CLI.
- Command lists now say what the tool did in plain words ("Find who calls
  _read", "Look for duplicate code in every file") instead of raw JSON or
  "(all paths)"; the raw arguments stay available in the cell tooltip.
- The Anti-Reinvention filter bar no longer carries a free-text search box;
  repo, detector, and view filters remain.
- Memory and reuse lists collapse to a view selector: Memory defaults to the
  10 notes with the latest activity (created, verified, or recalled), with
  Top 3, Most used, and All available; Reuse defaults to the 10 candidates by
  canonical call-site usage, with Top 3 and All. Each list shows
  "Showing X of Y", and memory cards now carry `created`, `recalled N×`,
  `last used`, and `verified` timestamps instead of a bare recall badge.
- The dashboard now renders only server payloads and tells the truth about
  every number. The client-side aggregate mirror (including its 10x-off token
  coefficients) is deleted; KPIs refresh from a bounded scheduler instead. The
  Session Recall KPI counts tagged sessions only, per-tool savings are
  success-only (`sum(tool saved) == saved_tokens`), D3/D4 carry an evidence
  label instead of a fabricated 100% overlap, and payloads declare
  `schema_version: 2`, `generated_at`, `window: "retained_journal"`, and
  `*_estimated` flags. Memory payloads are `{notes, degraded, errors}` so an
  unreadable store is visible instead of rendering as empty. The monitor TUI
  reads the canonical tool keys (real p50/OK%/saved), reports its real version,
  and shows "—" for recall when nothing is tagged.
- Interaction fixes: agent drill-down can no longer blank the console
  (`switchTab` allowlists its keys), filters survive SSE refetches, selection
  is keyed by stable ids, and a log shrink or rotation emits a named SSE
  `reset` event instead of replaying the journal. Memory Verify (formerly
  Touch Anchor) addresses one store by canonical `memory.db` path, requires
  the displayed revision (409 on skew, applied atomically), and refreshes a
  new `anchored_at` column (memory schema v2, additive) instead of rewriting
  `created_at`. Refs in stores without an index are `unverified`, not broken.
- Security: the dashboard binds loopback only, rejects foreign `Host` headers,
  sends CSP/nosniff/frame-ancestors headers, refuses cross-origin or
  non-JSON Touch requests, removes the wildcard CORS header from `/events`,
  caps connections/headers/journal lines, and contains snippet paths to the
  repository root. All inline event handlers are gone in favor of delegated
  listeners.
- Performance: the reuse detector scan is cached on index mtimes (2.2 s cold,
  ~2 ms warm), journal parsing is cached per log generation, snippets read
  only the requested window, and `/api/reuse` is fetched only when its tab is
  active.
- Accessibility: full tabs pattern (ARIA state, roving tabindex, arrow keys),
  keyboard-activatable rows and cards, live-region announcements for state
  transitions, visible focus rings, reduced-motion guards, AA contrast fixes,
  and truncation for long refs/args.

### Fixed
- Integration-test fixtures now delete their temporary repositories on drop.
  A full `cargo test` run previously left ~6,000 `eg-*` directories (933 MB) in
  the system temp dir; the MCP suite left another eleven per run.
- Usage entries of deleted scratch repositories no longer reach the dashboard or
  `eg usage`. The staleness predicate originally keyed on the linked-worktree
  flag, which benchmark temp roots never set; benchmark builds now pass
  `--no-journal` so they stop writing these lines at all.
- Deleted repositories now vanish from every telemetry surface. One staleness
  predicate decides it all: an entry whose absolute repo root is no longer a
  real directory is dropped by every reader (CLI global summary, dashboard APIs,
  Recent Queries, the SSE stream, and `eg monitor`'s live feed) and purged from
  `usage.log` on the next `eg dashboard` or `eg doctor` run. A path that has
  been replaced by a symlink counts as gone, since the journal records
  canonical paths and the symlink aliases a different repository. The same
  rule retires the `(unattributed)` bucket: identityless legacy lines and
  relative-path test artifacts are pruned and skipped, and an invocation that
  resolves no repository is no longer journaled at all, so totals contain only
  live, attributed calls. The active-worktree KPI can no longer count deleted
  worktrees, and dashboard responses send `Cache-Control: no-store` so a
  long-lived tab never reuses a stale asset bundle.
- Agent attribution now recognizes the calling CLI. `CLAUDECODE`,
  `CODEX_TOOL_CALL_ID`, `ZCODE_MANAGED_PACKAGE`, `ZCODE_APP_VERSION`,
  `ZCODE_PROCESS_LABEL`, `OPENCODE_CLIENT`, and `ZED_ENVIRONMENT=worktree-shell`
  are read as markers, with agent markers taking precedence over editor
  markers; `ENGRAM_AGENT` still overrides every marker. `TERM_PROGRAM=zed` is
  deliberately ignored, since Zed sets it for terminals a person opens too.
  Caller names are canonicalized, so `antigravity-client`, `claude`, and
  `CLAUDE_CODE` resolve to one slug each. The dashboard tags the `user`
  fallback with hover help: no marker was detected, so the invoker may be a
  human terminal or an unidentified agent.
- Anti-Reinvention tab: the Semantic Judge tile now reports the real judged-run
  count instead of always zero, the detector card states the implemented
  thresholds (D1 80%, D2 60%), and the excess-code subtitle marks the figure as
  an extrapolation from sampled candidates.
- `eg build` refuses the filesystem root with a usage error (exit 2) instead of
  trying to create `/.engram` and failing later with an opaque I/O error. An MCP
  client that reports a `rootUri` of `/` used to reach the second case.
- `eg mem` commands reject an explicit `--root` that cannot resolve, instead of
  silently falling back to the global store.
- D2 no longer reports two functions that merely share a signature. Similarity
  is measured on the symbol names instead of the signature-dominated token bag,
  and numbered siblings in one file (`find_best_d1` / `find_best_d2`) are treated
  as a family. This removes the `check_d3`/`check_d4` and `find_best_d1`/
  `find_best_d2` classes of sibling pair.
- Usage lines for calls that resolved no repository are marked
  `"attribution":"no_root"`, and the dashboard files them under `(unattributed)`
  instead of `workspace`, so a repository actually named `workspace` stays
  readable.
- `eg` resolves the project root once per run and hands that path to the arm and
  to the journal. Previously both resolved separately, so a command could succeed
  and still be logged without a repository.
- One repository identity replaces the per-surface guesses. `src/repo.rs`
  resolves a canonical root, a subdirectory now resolves to its work tree root
  instead of inventing a repository named after the directory, and the journal
  writes the canonical path. The dashboard keys `repo_aggs` by that path and
  shows the directory name in `display`, so two checkouts sharing a name stay
  apart. The `"workspace"` and `"repo"` fallbacks in the journal, dashboard,
  monitor, and reuse panel are gone.

### Added
- `eg usage` prints a `Savings basis` line splitting the token estimate by tool
  family. The reuse coefficient dominates that estimate, and the single total no
  longer reads as measured savings.
- `eg reuse --judge` explains a no-op run: when no candidate falls in the 45-75%
  band it says so, instead of returning silently as if a verdict had been given.

### Changed
- The pre-rename `aw` compatibility layer is gone. `AW_AGENT`, `AW_SESSION`,
  `AW_ARM`, `AW_NO_JOURNAL`, and `AW_GLOBAL_DIR` are no longer read, `.aw/` is
  no longer skipped during discovery, and the one-shot migration of
  `.aw/memory.db` and `~/.aw/memory.db` is removed with its test. Migrate a
  store that only exists under `.aw/` before upgrading; this release does not
  read it.

## [1.0.2] - 2026-11-10

### Added
- `eg reuse --judge`: sends candidates with 45-75% overlap to a local `eg-judge`
  binary and appends its verdict. Reads `EG_JUDGE_BIN` and `EG_JUDGE_MODEL`; a
  missing binary, missing checkpoint, timeout, or malformed answer leaves the
  static output byte-identical and prints one stderr note. Exposed on the MCP
  `reuse` tool as `judge`.
- Usage log field `"judged": true` on invocations that dispatched `eg-judge`
  successfully. The dashboard shows a `judge` badge on those rows and a
  "Semantic Judge" counter in the Reuse panel.

### Changed
- Companion repository `engram-laya` renamed to `engram-judge`; binary
  `eg-laya` renamed `eg-judge`, and the checkpoint variables are now
  `EG_JUDGE_BIN` / `EG_JUDGE_MODEL` with no legacy alias.

### Fixed
- Fixed clippy `unneeded_wildcard_pattern` warning in `src/query/analysis/unresolved.rs`.

### Documentation
- Grounded canonical specs and guides in peer-reviewed research papers (RepoReuse, Zero-Mem, Agent Zero Memory, Agent-Native Memory Systems).
- Removed 185 non-shipping historical baseline snapshots and archive drafts from `docs/archive/` and `docs/baselines/`.
- Updated cross-references across `CONTRIBUTING.md`, `AGENTS.md`, and GitHub templates to canonical architecture paths.

## [1.0.1] - 2026-11-09

### Added
- `eg reuse --deep-callee`: extends the D1 callee footprint to second-hop
  callees (weight `0.5`), catching wrappers that forward through an
  intermediate helper.
- `eg mem write` near-duplicate guard: at 80% Jaccard token similarity the
  write appends an advisory hint naming the existing key and suggesting
  `--supersedes`. The note is still written.
- `eg mem query` call-graph expansion: when `.engram/index.db` is readable,
  query terms seed a bounded `calls` traversal (depth <= 3, decay
  `0.5^depth`) so notes anchored to downstream callees surface. A missing
  index falls back to plain FTS5 ranking.

### Changed
- `eg reuse` builds inverted posting lists for callees, token pairs, and raw
  clients above 1,000 symbols, replacing the per-target corpus scan with
  posting-list candidates. Output is unchanged; a parity unit test compares
  the indexed and flat detector results.
- `eg reuse` folds caller counts from the call edges it already loads instead
  of running a correlated count subquery per symbol.
- `eg reuse` skips the unresolved-call span index when the index has no
  unresolved call rows.

## [1.0.0] - 2026-11-09

### Highlights & Official Release

- **Official Open-Source Release**: `engram` (`eg`) reaches v1.0.0 with unified single native binary architecture.
- **USP 1: Sub-2ms Deterministic Graph Navigation**: Tree-sitter AST indexing for Python, TypeScript, JavaScript, and Rust with SQLite FTS5 search and content-addressed incremental builds.
- **USP 2: Real-Time Anti-Reinvention Engine**: Pre-write interface discovery with caller weighting and post-edit duplicate detection across 4 calibrated detectors.
- **USP 3: Zero-Token Layered Memory**: Code-anchored persistent project and global memory with SQLite BM25 + recency/ref fusion.
- **Native Web Dashboard & TUI Monitor**: In-process HTTP SSE dashboard and terminal streaming monitor with zero Python dependencies.
- **Model Context Protocol (MCP)**: Conforming stdio server for Claude, Cursor, Antigravity, Grok, Zcode, and OpenCode.
- **Security & Confinement**: Strict write confinement to `.engram/` and `~/.engram/`, rejection of Git-tracked repository state, POSIX 0700/0600 modes.

### Changed — project rename: `engram` / `eg` (2026-11-09)

- The package is now `engram` and the installed command is `eg`. The crate
  compiles into one native binary with every subcommand, including
  `eg dashboard` and `eg monitor`.
- Confinement directories move to `.engram/` and `~/.engram/`. A pre-rename
  `.aw/memory.db` or `~/.aw/memory.db` is migrated automatically on the first
  memory read or write when the new store is absent; the legacy file is copied,
  never moved, so a downgrade keeps working. Indexes are not migrated: `eg build`
  rebuilds them.
- Environment variables use the `ENGRAM_` prefix. `ENGRAM_NO_JOURNAL`,
  `ENGRAM_SESSION`, `ENGRAM_GLOBAL_DIR`, and `ENGRAM_AGENT` resolve the legacy
  `AW_*` spelling as a fallback, so existing privacy opt-outs and global-store
  overrides keep applying.
- The MCP server identifies itself as `engram`, the KPI footer reads `⚡ eg:`,
  and CLI output prefixes read `eg: ...`. The routing skill moves to
  `skills/engram-first`, and the installer provides clean configuration for all running agent hosts.
- Security hardening: repo-local `.engram/` state that is tracked in Git is
  rejected with exit 2 (`eg: security: ...`) to block committed memory
  injection, `~/.engram/` is created `0700` with `0600` stores and journals on
  Unix, and `eg doctor --invalidate` unlinks symlinked entries instead of
  traversing them.

### Added — portable memory exchange (2026-11-09)

- **`eg mem export --format markdown`**: renders current notes as fenced
  Markdown (`@@ engram-note <slug> @@`, one-line JSON front-matter, body,
  matching closer) so durable memory can be reviewed in pull requests and
  versioned in Git. JSONL stays the default and keeps its restore contract.
- **`eg mem import <path>`**: symmetric restore from a Markdown or JSONL export,
  or stdin (`-`). The command parses at the service boundary, validates every
  note, and writes the batch in one transaction. Imported rows carry
  `source=import` and render `[imported]`.
- **Reject-on-conflict import**: a key whose local content differs is skipped
  and the command exits 2 unless `--force` (alias `--overwrite`) opens a new
  revision. Identical content is a no-op.
- **`procedure` note kind**: the closed taxonomy is now `decision | gotcha |
  fact | task | procedure`, with dashboard badge/filter support.
- **MCP**: `mem_import` joins the `--all-tools` surface (repo-relative path,
  project store only); `mem_export` gains a `format` argument. CLI and MCP
  success payloads stay byte-identical.

## [0.5.3] - 2026-11-09

### Added

- **Memory touch & anchor freshness refresh.** Added `aw mem touch <key>` subcommand and `mem_touch` MCP tool to refresh note timestamps without creating new revisions. Added `--touch` flag to `aw mem write` to refresh timestamps when content hash matches.
- **Reference filter & empty query normalization.** Added `--ref <path_or_symbol>` parameter to `aw mem query` and `mem_query` tool to filter candidate notes by exact code reference target. Normalized empty string terms in `mem_query` to fall back to listing current notes (`effective_all = true`) with exit code 0.
- **Broken vs. modified reference distinction.** Clarified stale reference statuses by distinguishing between missing targets (`[stale-ref] [broken-ref]`) and targets modified after note authoring (`[stale-ref] [modified-ref]`).
- **Ambient memory co-retrieval and guard hints.** `aw context <name>` now co-retrieves active gotchas and decisions referencing the target symbol. `aw impact <file>` displays warnings when modified files are guarded by architectural decisions. `aw build` emits non-invasive stderr notes during incremental builds when modified files touch guarded areas.
- **Three-pillar dashboard redesign & redundancy elimination.** Aligned web dashboard tabs 1:1 with the three core USPs: Tab 01 strictly owns Code Graph Navigation (graph metrics, sub-ms latency, avoided reads, graph SSE stream); Tab 02 strictly owns the Anti-Reinvention Engine (D1–D4 detectors, snippet diff triage, pre-write helper discovery feed); Tab 03 strictly owns Zero-Token Project Memory (Read:Write ratio, Session Recall Rate, Anchor Currency Rate, Guarded Hotspots, and Action Center with 1-click Touch).
- **Dashboard Action Center with 1-click Touch.** Web dashboard Action Center alerts on broken and modified references with 1-click touch action calling `POST /api/memory/touch`.

## [0.5.2] - 2026-11-09

### Added

- **Global telemetry default with backlog streaming.** `aw dashboard` and `aw monitor` now default to machine-wide global telemetry (`~/.aw/usage.log`), providing cross-repository visibility by default. Added `--local` flag to scope down to the current workspace (`.aw/usage.log`).
- **Initial backlog replay in `aw monitor`.** Terminal query monitor pre-loads and streams the 15 most recent invocations on launch, eliminating blank screen confusion before live events arrive.
- **Agent caller attribution & tagging.** Invocations are automatically attributed to calling agents via MCP handshake `clientInfo.name` (e.g. `antigravity`), CLI environment variables (`AW_AGENT`, `ANTIGRAVITY_AGENT`), or fallback to `user`. Added dedicated `AGENT` column on terminal monitor and colored badge tags (`.agent-tag`) on the web dashboard.
- **Workspace & Agent workload comparative analytics charts.** Interactive horizontal comparative bar charts on the web dashboard for workspace distribution and agent attribution, tracking call volume, percentage share, reliability %, tokens saved, and last active timestamp with 1-click cross-filtering.
- **Automated orphaned worktree pruning.** Dual-layer cleanup for temporary or deleted Git worktrees: runtime filter immediately excludes missing worktree paths from metrics and charts, and physical disk pruning (`prune_deleted_worktrees_all`) purges orphaned records from `usage.log` during `aw dashboard` startup and `aw doctor`.

## [0.5.1] - 2026-11-08

### Fixed

- **Memory retrieval via code-anchored `--refs`.** `aw mem query` now indexes and retrieves candidate notes directly by `--refs` tokens even when the search query term does not appear in `key`, `tags`, or `body`, eliminating candidate drops for AST entity lookups.
- **Automated `[stale-ref]` verification across CLI and MCP.** `aw mem query` and `aw mem get` now automatically verify referenced files and symbols against `.aw/index.db`. If an anchor symbol/file has been removed or modified since the note was authored (`mtime > created_at`), the note is flagged with `[stale-ref]`, matching the contract documented in `README.md` and `docs/usage.md`.
- **MCP `build` incremental reuse hint delivery.** `aw build` over MCP now forwards `stderr` diagnostic hints (including `aw: reuse-hint:`) within the tool call response text when build succeeds (exit code 0), ensuring coding agents on MCP receive real-time anti-reinvention alerts.
- **Targeted incremental hint filtering.** `incremental_reuse_hints` now inspects `resolution.existing_ids` to evaluate only genuinely new or modified symbols, eliminating false-positive reuse alerts on carried symbols in dirty closures.
- **Intra-file duplicate detection in `aw reuse`.** Detectors D1 (callee containment), D2 (interface token overlap), and D3 (raw client bypass) now detect duplicate helper functions authored within the same file (protecting against multi-turn intra-file duplication), while ignoring overlapping/enclosing AST spans.
- **Symmetric D1 containment.** Callee footprint containment in D1 now uses symmetric containment $\max\left(\frac{|C(t) \cap C(c)|}{|C(c)|}, \frac{|C(t) \cap C(c)|}{|C(t)|}\right) \ge 0.80$ guarded by a size ratio ($2 \times \min \ge \max$) to catch both subset and superset duplicates.

### Added

- **Native Rust web dashboard: `aw dashboard`.** Pure Rust HTTP 1.1 +
  Server-Sent Events (SSE) server implemented with `std::net`, requiring zero
  Python and zero external web/async crates. Serves embedded static assets
  (`assets/index.html`, `style.css`, `app.js`) with a responsive 3-tab layout:
  1. *Telemetry & Operations*: QPS, p50/p95 latency, token savings, active repo
     breakdowns, and live-streaming error triage.
  2. *Anti-Reinvention Inspector*: Direct engine-driven D1–D4 duplicate
     detection with side-by-side source code preview panels.
  3. *Agent Memory Inspector*: Interactive layered local/global memory notes
     with real-time recall counts and verified index references.
  Accessible via `aw dashboard [--port 7890] [--global] [--open]`.
- **Native Rust terminal streaming monitor: `aw monitor`.** Real-time ANSI query
  event streaming monitor tailing `usage.log` with zero external dependencies.
  Supports repository filtering (`--repo <NAME>`) and error triage (`--errors-only`).
- **Global telemetry logging & multi-repo dashboard.** Dual-write telemetry to
  `<repo>/.aw/usage.log` and global `~/.aw/usage.log` (configurable via
  `AW_GLOBAL_DIR`), tracking repository attribution and Git worktree flags.
  Added `aw usage --global` (CLI and MCP) for machine-wide performance,
  active-repo breakdowns, and recent error triage. Updated `scripts/aw-dashboard`
  wrapper to invoke `exec aw dashboard --global "$@"`.

### Removed

- **Deprecated Python scripts.** Removed `scripts/aw-monitor` and
  `tests/scripts/render_dashboard.py`, fully standardizing on native Rust
  `aw dashboard` and `aw monitor`.
- **Documentation standardization & 3 USPs overhaul.** Complete public GitHub
  showcase landing page in `README.md` with badges, 3 USPs deep-dive (sub-2ms
  deterministic navigation, real-time anti-reinvention engine based on RepoReuse
  `arXiv:2609.35357`, and zero-token layered memory based on Zero-Mem
  `arXiv:2607.29377`), comparison matrix, 60-second quickstart, and 1-click MCP
  configs. Aligned task-oriented guides (`docs/getting-started.md`,
  `docs/usage.md`), CLI reference for 10 core commands matching schema v10
  (`docs/cli-reference.md`), agent integration and real-time telemetry
  (`docs/agent-integration.md`), sitemap (`docs/README.md`), and updated agent
  routing policy (`skills/aw-first/SKILL.md`).
- **Cross-project global memory: `aw mem --global`.** aw now supports a global
  memory store in `~/.aw/memory.db` (`$AW_GLOBAL_DIR/memory.db`) for developer
  profiles, personal preferences, and cross-project conventions. Transparent
  layered queries (`aw mem query`) simultaneously retrieve from both local and
  global stores with `[global]` badges and local key shadowing. Operations can
  be targeted explicitly to the global store via `--global` across write, query,
  get, delete, export, and gc, with full MCP parity.
- **Project memory: `aw mem write/query/get/delete/export/gc`.** aw now keeps
  agent-authored project notes (decision/gotcha/fact/task) in
  `.aw/memory.db`, a separate SQLite store with its own FTS5 index, revision
  chains, content-hash idempotent replay, and a deterministic resolver
  (BM25 + ref/token overlap + recency fusion). Notes are candidates, never
  proof: every mem command opens with the caveat line. Bodies cap at 8 KiB;
  the journal carries `body_len`, never body text. Design:
  `docs/architecture/memory-store.md`.
- **`aw mem export` JSONL backup.** stdout is pure JSONL (the caveat goes to
  stderr); restore = `mem write` replay, idempotent by content hash. This is
  the recovery path for the primary memory store.
- **MCP `mem_write` + `mem_query` join the default tool set** (owner decision
  OQ1); `mem_get`, `mem_delete`, `mem_gc`, and `mem_export` sit behind
  `--all-tools` (17 tools total, 13 by default). The `aw-first` skill gains
  a non-mandatory "Project memory" section.
- **Memory lifecycle (Phase 2, OQ2 default TTL 7 days).** `--scope session`
  expires after a TTL (`--ttl <SECS>`, default 604800 = 7 days) and drops out
  of every read once expired; `aw mem gc` now purges expired session notes
  (with their search-index rows) and superseded revisions older than 30 days
  clocked by `superseded_at`, printing
  `expired=N removed, superseded=M removed, durable=K rows`.
  `mem write --supersedes <key>` records a cross-key replacement in the same
  transaction (unknown target exits 1, same key exits 2);
  `mem query --all --include-superseded` lists replaced revisions tagged
  `[superseded]`.
- **Memory measurement (Phase 3).** `tests/scripts/mem_recall.py` (stdlib
  Python) measures `aw mem query` recall against the frozen 30-note/20-query
  oracle `tests/fixtures/memory/oracle-v2.json` with a Wilson 95% interval
  and a seeded negative control (gate: recall >= 0.8, shuffled mapping below
  the gate). First run: recall@5 = 1.000 [0.839, 1.000], negative control
  0.050 — see [Evaluation](docs/evaluation.md). Weight tuning stays gated on
  a sub-threshold oracle run.

### Changed (breaking)

- **`aw doctor --invalidate` keeps `.aw/memory.db`.** The reset removes the
  derived state (index, journals, locks, orphans) and preserves the primary
  memory store and its crash-recovery sidecars; `rm -rf .aw/` remains the
  one-command exit that discards memory too. `--help` documents the new scope.
  Graft-learning Decision B1 (`aw note add`) is superseded by this store
  (owner decision OQ6).

### Fixed

- **`aw resolve --stdin` journals non-zero batch exits.** A missing or
  ambiguous name returns exit 1/2 through the batch-output path, which now
  finalizes the pending journal line; `aw usage --quality` counts those
  invocations instead of dropping them.

## [0.5.0] - 2026-11-05

### Changed (breaking)

- **Schema v9; rebuild required.** An older index exits `5` on every query;
  `aw build` replaces it atomically. Removed: the `clone_groups` table and its
  five-line shingling pass, `symbols.cyclomatic/branches/nesting`,
  `files.loc/cyclomatic`, and the five-column `symbols_complexity_idx`
  (replaced by an `effective_loc` ranking key). A fresh build is smaller and
  skips the shingle pass.
- **`aw complexity` ranks by effective LOC only.** `--min-cyc`,
  `--min-branches`, and `--min-nest` are gone; rows print `path:line loc=N
  qualname`. `aw symbols --metrics` appends only `loc=`.
- **`aw clones`, `aw hotspots`, `aw report`, and `aw tests` are removed**, along
  with their MCP tools. Clone detection, git-log churn scraping (including the
  hardcoded fix-keyword list), the four-section report, and static test-reach
  candidates are outside the call-graph navigation core. `dead
  --exclude-tests` keeps the `is_test` classification.
- **Search no longer applies a morphology fallback.** A zero-hit query is not
  retried with derived stems (`rotation` -> `rotate`); prefix matching over
  identifier tokens is the documented behavior.
- **One dispatch layer for CLI and MCP.** `src/service.rs` owns root
  resolution, index opening, journaling, output rendering, and exit-code
  mapping. The MCP tool schemas and required-argument checks render from one
  declarative tool table, so the two surfaces cannot drift.
- **The usage journal line is serialized with serde_json.** The JSONL file
  keeps the same keys and format. The journal no longer refuses a symlinked,
  hardlinked, or FIFO `usage.log`: an append follows the path as given, so a
  repository that ships such a link can redirect the journal or block the
  writer. Treat repository-controlled `.aw/` entries as untrusted.
- **AST traversal uses an explicit work stack.** Nesting depth is bounded by
  the 1 MiB file cap, not the thread stack, so parse workers run on the default
  thread stack (the former 64 MB stack forcing is gone). Depth is no longer
  truncated at a nesting ceiling.
- **One build lock.** `RootLock` (a flock on the repository directory) is
  removed; `BuildLock` on `.aw/build.lock` is the single lock identity for
  builds and `doctor --invalidate`.

### Added

- `--explain` for `aw callers`, `aw callees`, and `aw context` appends the
  stored edge metadata: kind, confidence, site line, destination, grouped
  edge count, and a typed-receiver hint when a `type_ref` edge exists. It
  reports ambiguous rows as ambiguous and states that the candidate set is not
  stored. The flag is opt-in; default output is byte-identical and no schema
  change is required.

- `aw usage --quality` adds the empty-result split (`empty_ok` for exit 0,
  `empty_error` for non-zero), the capped-result count (`total > shown`), and
  the `empty_ok` percentage. It reads only fields already written to
  `.aw/usage.log`; the default summary and the journal schema are unchanged.

- `symbols.is_test` (schema v7 -> v8): test classification computed once at
  extraction from test paths (`tests/`, `test/`, `__tests__/`), test-shaped
  names (`test_*`, `test` + uppercase) and files (`test_*.py`, `*_test.py`,
  `*_test.go`, `*_test.dart`, `*.test.*`, `*.spec.*`), Rust `#[test]` and
  `#[cfg(test)]` ancestry, and TS `describe`/`it` scope. `aw tests`, the
  `dead --exclude-tests` filter, and the build-time `test_symbol_count`
  counter now read the column instead of repeating name-shape SQL, so
  Rust-attribute test functions (for example `fnv1a64_known_vectors`) are
  recognized without `test_*` names. Assert-family macro bodies
  (`assert!`, `assert_eq!`, `assert_ne!`, `debug_assert!`) now contribute
  call edges from their token trees, bounded per site.

**User impact:** `aw tests` finds attribute-only Rust test fns; `dead
--exclude-tests` drops the same rows the v7 heuristics dropped plus
attribute-classified tests. Callers that parse output see no shape changes.
**Required action:** run `aw build` to replace v7 indexes with schema v8.
Query opens on v7 exit 5.

- Typed Python receiver calls (`x.m()` with `x: cls`) now also emit an
  internal `type_ref` edge to the receiver's class. The incremental fast
  path's dirty closure follows that edge, so a method added on an ancestor
  class re-extracts carried callers and their MRO answer matches
  `aw build --force` instead of staying stale. The edge participates in the
  gap queries and in `aw dead` (a class referenced only through an
  annotation is not dead); `impact --edges` and callers/callees traversal
  keep their existing kinds. Output shapes are unchanged; edge totals grow
  by one per typed receiver call site.

- `aw impact <file>` in the default calls-only mode no longer answers a bare
  `0 impacting files found` when the target file has importers or
  inheritors. The empty result now carries the calls-only caveat header and
  a detail line with the counts, for example `0 impacting files found
  (calls only); 1 file(s) import and 0 file(s) inherit from this file —
  rerun with --edges imports,inherits`. Exit code stays 1. Consumers that
  match the old empty stderr byte-for-byte must accept the new
  no-results detail; non-empty results are byte-identical.

## [0.4.0] - 2026-11-05

**User impact:** faster fresh builds, broader dependency navigation, and more reliable incremental resolution.
**Required action:** run `aw build` to replace older indexes with schema v7. Query opens on v6 exit 5.
Consumers that parse output must review the query-shape changes listed below before upgrading.

### Removed

- `aw usage --verify` and its `--limit` flag, and the MCP `usage` tool's
  `verify`/`limit` arguments. `aw usage` (CLI and MCP) summarizes the journal
  only.
- The result-payload journal `.aw/usage-results.jsonl`: `aw` 0.4 no longer
  writes or reads pointer payloads. A file left by 0.3 stays on disk and can
  be deleted manually.

### Fixed — re-review hardening (2026-11-04)

- Incremental builds re-apply the inbound dirty closure after gap healing,
  invalidate ambiguous sites when a candidate is removed or renamed, and choose
  duplicate import candidates deterministically (`AMBIGUOUS` instead of an
  insertion-order pick). `load_carried` queries are chunked, and file reads are
  bounded at 1 MiB even if a file grows after discovery. Incremental and forced
  builds should agree on normalized edge sets.
- Source discovery rejects a tracked path whose ancestor is a symlink outside the
  repository root.
- Resolution fixes: assignment and arrow-parameter shadowing, nested
  `self`/`cls`/`this` parameters, ordered bindings at each call site,
  annotation-as-initial-binding, TS type annotations and `new C()`, Rust struct
  literals and `Self::new()`, scoped import aliases, default exports, and Rust
  inline-module aliases.
- Lock and journal files reject FIFO, device, symlink, and hardlinked targets;
  journal readers are regular-file and byte bounded. MCP validates JSON-RPC
  envelopes and does not answer notifications.
- Search counts the exact ∪ FTS union by symbol identity; impact ties use stable
  semantic fields; the trust contract lists
  capped, stale, ambiguous, empty, and safety/review exceptions.
- Root `lib.rs`/`main.rs` inline modules now use the crate-root namespace (`a`),
  not the file-module prefix (`lib.a`), so `crate::a` resolves consistently for
  file and inline modules. Duplicate module/symbol import candidates are
  reported as `AMBIGUOUS` instead of one arbitrary candidate.

### Added — transitive callers (2026-11-03)

- `aw callers <name> --depth N` follows the resolved reverse call closure up to
  N hops. Depth 1 keeps the existing output unchanged; deeper rows append
  `[depth=N]`. MCP exposes the same `depth` argument. No schema change.

### Changed — agent trust contract (2026-11-03)

- The `aw-first` skill now states that successful navigation output is ground
  truth for the fields it returns. `tests`, `dead`, and `unresolved` remain
  candidate-only. Source reads still gate behavioral claims, edits, safety, and
  review. No CLI, output, or MCP contract changed.

### Changed — project rename (2026-11-03)

- The package is now `agent-weave` and the installed command is `aw`. The
  index directory is `.aw/`, environment variables use the `AW_` prefix, and
  the MCP server identifies itself as `agent-weave`. Repository and
  documentation links move to `https://github.com/quy-doan/agent-weave`.
- Historical benchmarks recorded before 0.3.0 used the former name `kg`; the
  evaluation summary keeps those names only when quoting old artifacts.

### Changed — internal module layout (2026-11-03)

- `src/extract.rs` is now a facade over `extract/walk.rs` (AST walk) and
  `extract/resolve.rs` (global resolution). `src/query.rs` is a facade over
  `query/callgraph.rs`, `query/impact.rs`, `query/analysis.rs`, and
  `query/scope.rs`. Public crate paths and all output bytes are unchanged.
- `tests/scripts/write_path_check.sh` scans nested `src/**/*.rs` and its
  negative control covers a write inside a subdirectory.
- Production code no longer mutates process environment: `GIT_OPTIONAL_LOCKS=0`
  is set on each git probe, and the MCP `--no-journal` flag disables the
  journal through a process-local flag instead of `AW_NO_JOURNAL`.

### Fixed — query edge cases and diagnostics (2026-11-03)

- `Class.method` names now resolve as a qualname suffix when the exact name is
  absent (for example `aw callers Pointer.render`); several suffix matches
  still exit 2 with the candidate list.
- A single-token `aw search` keeps exact matches first and appends
  deduplicated prefix/compound matches, so `search run` still returns
  `run_build` after an exact `run` symbol.
- Ambiguous pointers keep the space before `[ambiguous]`, and `callees`
  renders the recorded target name for unresolved/ambiguous rows.
- `dead` rows carry the qualified name; `symbols .` and `symbols ./` cover the
  repository root without a `/` prefix.
- Path-scoped SQL binds its scope instead of escaping string literals and
  uses a sargable range predicate. Fresh v7 builds add the
  `symbols_file_path` secondary index, and `complexity --path` omits the
  forced complexity index so SQLite selects it. Existing v7 indexes remain
  compatible (the planner falls back).
- Usage and payload journal lines parse through `serde_json`, so quoted
  queries and escaped pointers survive. `stats`, `hotspots`, and `report`
  record real result counts, `resolve` records latency on its early exit, and
  `doctor` reports a build lock only while another process holds it.

### Fixed — Rust receiver and module resolution (2026-11-03)

- Rust method calls on `self`, typed parameters, and constructor-typed locals
  now produce call edges. `let x: Foo`, `let x = Foo::new()`, and
  `let x = Foo { .. }` type the receiver; `&`/`&mut` wrappers are unwrapped.
- `Self::assoc()`, `self::f()`, and `super::f()` resolve through the enclosing
  impl target or the module tree.
- Free functions inside an inline `mod` keep kind `function` instead of being
  labeled methods.
- When a receiver type is unknown, a uniquely named same-file method resolves;
  common stdlib/trait names and multi-match names stay unresolved instead of
  producing ambiguous fan-out.

### Fixed — build walk over ignored trees (2026-11-03)

- When `git ls-files` succeeds, `build` enumerates the repository-relative
  path list directly instead of walking the whole tree. Ignored trees such as
  `target/` and `node_modules/` are never traversed, so a large or unreadable
  ignored directory cannot slow or abort a build. `WalkBuilder` remains the
  fallback when git is unavailable.
- `build` still indexes tracked-but-ignored files, untracked-unignored files,
  and hidden files, and still skips `.aw/`, symlinks, directories, and files
  over 1 MiB.

### Removed — ineffective surfaces (2026-11-03)

- `aw api` and its MCP tool are removed. The `symbols.exported` column and
  extraction remain for schema compatibility; no CLI or MCP surface reads it.
- `search --kind`, `search --path`, and `search --exact` are removed. Base
  `aw search <terms>` and the identity fast path remain.
- The six `--exclude-generated` flags on `dead`, `clones`, `hotspots`,
  `report`, and `complexity` are removed. `files.generated`, `stats
  generated=N`, and the generated tag remain.
- `dead` stays as a command but is no longer routed by `aw-first`; it is
  candidate-only until a precision gate exists.
- Package version moves to `0.3.0` for the breaking CLI/MCP contract change.
  Schema stays v7.

### Changed — build performance (2026-11-03)

- Fresh builds create secondary indexes after the bulk insert
  (`schema_indexes.sql`); the final schema (index names and key order) is
  unchanged. The index file is about 4% smaller.
- The disposable `.aw/index.db.new` connection runs with
  `journal_mode=OFF`/`synchronous=OFF`; the finished file is fsynced before the
  atomic rename, so the published index stays durable.
- Meta totals come from one scan per table; insert paths borrow rows instead
  of cloning them; forced builds populate FTS from the in-memory extraction.
- Parse/extract collects error, decision, comment, LOC, and code-mask data in
  one tree walk, shares line bounds and metric tables across callables, and
  reuses one parser per worker.
- Resolution looks up a parent symbol kind through `by_qualname` instead of a
  reverse scan.
- No schema, stats-line, or query-semantics change. Query outputs are
  byte-identical except `aw stats`'s `db_bytes`, which reports the smaller
  index file. Best-of-7 `build --force` on the 2,025-file synthetic corpus:
  1.87 s vs 3.61 s baseline on a heavily loaded host (interleaved A/B); details
  come from an internal optimization report (original evidence unavailable publicly).

### Changed — schema v7

- `symbols.file_path` denormalizes `files.path`; `symbols_complexity_idx`
  covers `(cyclomatic, effective_loc, qualname, file_path, line_start)` so
  `aw complexity` tie-breaks by path, not insertion-order `file_id`.
- v6 indexes are not migrated: query opens exit 5 and `aw build` rebuilds.
  Fresh inserts and carry-forward both populate the column; `probe_old`
  rejects a v7-stamped index that lacks it.

### Fixed — incremental resolution and index validation

- Incremental Rust `impl Trait for T` resolves against carried
  traits/interfaces; qualified `crate::Trait` impl targets resolve.
- Root TS `index.*`, Python `from .. import X`, and Rust inline `mod` members
  resolve their imports/calls; Rust trait-impl exports no longer leak
  inherent private methods, while module-parent `pub fn` exports.
- A pre-manifest v7 index rebuilds instead of reporting an empty default
  `complexity` result.

### Fixed — import resolution and query consistency

- Python dotted imports keep every ancestor module edge; Rust crate roots,
  wildcard use trees, and single-segment module/symbol imports resolve; star
  imports heal after destination restore.
- Cross-file Rust impls and non-identifier TS mixin calls store unresolved
  `inherit` rows; `--context` per-site behavior is normative.
- `search --exact` bypasses FTS and journals no fuzzy miss; path scopes
  reject embedded `..` and root `api`; per-site empty reasons keep their
  suffix; nested `test_*.py` files are excluded from `dead`; `unresolved
  --names` has the standard staleness header and unlocated-row count.
- `effective_loc` uses comment-node byte coverage; TS `protected` is private;
  Rust trait and trait-impl methods of an exported type are exported.
- Gate commands select real tests; GT harnesses report Wilson precision.
- 1M-symbol `complexity` default is 6.9 ms via `meta.callable_count`;
  path scope is amended to a 250 ms budget (201 ms measured).

### Fixed — structural graph correctness and latency

- Rust imports: structural `use` trees (grouped, alias, `self`/`super`,
  wildcard, single-segment), `mod.rs`/`lib.rs`/`index.*` module mapping, and
  external-crate suppression. Star imports suppress unresolved names only on
  their own statement; deleted destinations become unresolved and restore
  when the file returns.
- TypeScript imports: `.ts/.tsx/.js/.jsx/.mts/.cts/.mjs/.cjs` and trailing
  `index` collapse to one module identity; default imports emit no symbol
  edge; `from pkg import mod` keeps only the canonical child edge unless the
  statement also imports a symbol.
- Exports: two-pass resolution exports methods of `impl Type` declared before
  `pub struct Type`; TS arrow exports and local `export { X }` are tagged;
  `api` covers interfaces.
- Metrics/complexity: TS const arrows use the arrow node for both metrics and
  line range; `complexity` excludes zero-metric trait signatures; schema v7
  orders ties by path with a covering index.
- Contract: import sites count once per statement; `--context` tags only
  function/method sites; `complexity`/`api` carry staleness and payloads;
  unknown directory scopes report `0 symbols in '<dir>' (indexed)`; `context`
  enforces its documented 50-row section cap; `timing_assert.sh` measures
  semantic exit codes with one interpreter.
- Latency: `tests` and `impact --depth>1` keep only the top-`limit` display
  set plus an exact distinct count; fresh build best-of-7 is 2.82 s on the
  2,025-file corpus (budget 2.91 s).
- Gates: committed real-corpus ground truth and harnesses for imports
  (40 pairs), tests (20 symbols), and inheritance (11 pairs), plus a
  Flutter-corpus build smoke; determinism dumps include schema-v7 columns
  and `unresolved_calls`.

### Added — dependency, test, and symbol queries (schema v6)

- `aw implementers <name>`: resolved inbound `inherits` edges; Rust
  `trait_item`/`function_signature_item` and TS `interface_declaration`
  extraction; same-file Rust `impl Trait for Type`; cross-file impls are a
  documented limit.
- `aw imports <file>`: outbound resolved imports for Python plain/dotted/
  relative/star, TS relative/side-effect/require, and Rust `use`/`mod`;
  unresolved imports are stored as `unresolved_calls(kind='import')`.
- `aw tests <name|file>`: reverse call-edge test candidates with explicit
  static-resolution caveats and depth/truncation reporting.
- `aw impact` gains `--edges calls,imports,inherits,all`, `--depth N`,
  `--context`, and `--explain`; the default grouped output is unchanged.
- `aw symbols <dir> --outline`, `aw files`, `aw complexity`, `aw api`, and
  `aw resolve --stdin`.
- `aw unresolved --names`: `path:line kind name` sites with the `kind`
  column and real site lines.
- `aw dead` now treats inbound imports/inherits as usage, excludes
  trait/interface signature methods, and supports `--exclude-generated`;
  test exclusion uses `tests`/`test`/`__tests__` segments and filename globs.
- Schema v6 additive: per-symbol `cyclomatic`/`branches`/`nesting`/
  `effective_loc`/`exported`, `files.generated`, and
  `unresolved_calls.kind`/`site_line`; the fast-path probe verifies every
  new column and the complexity index, and `metrics_version` invalidates
  formula changes.
- Generated-file tagging (path suffix or first-five-line marker) surfaces
  `generated=N` in `aw stats`.
- MCP tools added for `implementers`, `imports`, `tests`, `complexity`,
  `api`, `files`, and `resolve`; `impact`, `dead`, `symbols`, `search`,
  `unresolved`, `clones`, `hotspots`, and `report` gained their new flags.

### Fixed — bounded queries and structural analysis

- `dead` trait/interface parent exclusion is materialized and indexable:
  2,025-file corpus 15.57 s -> 0.14 s; 1M-symbol COUNT no longer scans
  symbols².
- Import edges are deduplicated and reconciled: one edge per
  `(source, destination, site)`, parent-package edges for dotted imports,
  no unresolved row when any candidate resolved, star/self duplicates
  removed.
- Inheritance ignores generic arguments and scoped-path leaves; lexical
  namespace/inline-mod bases resolve; AMBIGUOUS and unresolved `inherit`
  rows are written; Rust impl-trait edges no longer enter `class_bases`;
  Rust impl targets resolve by full qualname and reject tuple/pointer types.
- `aw tests` requires `function|method`, skips seed tests, normalizes `./`,
  and chunks BFS frontiers at 500 ids.
- `aw report --path/--exclude-generated` no longer crashes on the section-4
  alias; `complexity`, `api`, `files`, `unresolved --names`, and
  `symbols <dir>` use SQL LIMIT + COUNT; `symbols <dir>` is one query.
- `aw api` now reads the populated `exported` flag, includes signatures,
  accepts a file or directory path (root `.` stays rejected), and follows
  Python `__all__`, Rust `pub`, and TS `export` rules.
- `aw resolve` requires `--stdin`, enforces 1,000 names / 4,096-byte lines /
  1 MiB, journals count/candidate_limit, and MCP sets `isError`.
- `search --exact` uses the untokenized input (case-sensitive); `--path`
  inputs are validated (exit 2); empty results carry their caveats.
- `effective_loc` handles docstrings and block comments; Rust
  `non_comment_loc` recognizes `line_comment`/`block_comment`; Python
  `match_statement` is no longer double-counted; trait signature methods
  store zero metrics; generated marker scan no longer caps at 4 KB.

## [0.2.1] - 2026-11-02

**User impact:** fuller signatures, better call resolution, and lower no-op build cost.
**Required action:** run `aw build` to refresh old signatures; review pointer identity and grouped `impact` output in integrations.

### Fixed — accuracy, utility, and latency hardening

- Signatures capture the full multi-line parameter list (collapsed, capped
  160), stopping before the body; 500/3,039 truncated Python signatures on
  corpus A drop to 0. A one-time `signature_version` marker re-extracts old
  indexes without a schema bump.
- Lexical scopes: Python nested defs belong to the enclosing function; Rust
  inline `mod` and TypeScript `namespace` qualify members; module symbols no
  longer create false ambiguity when a callable shares the bare name.
- `super()`/constructor receivers and re-exported base classes resolve.
- Caller/callee/context lines carry the call-site identity; context callees
  dedupe by destination with `[ambiguous]`; `impact` defaults to one row per
  file (`--sites` restores sites); `dead` suppresses per symbol and gains
  `--exclude-tests`.
- Clones: hash prefilter, merge-before-`--min-lines`, longest-first ordering,
  and enclosing-symbol identity per span.
- Search: qualified-phrase and exact-name fast paths, acronym splitting,
  and a bounded stem fallback for zero-hit queries.
- `unresolved` reports distinct names, `internal=` gaps, and top names;
  hotspots count Python `elif`/`case` and TS ternaries and exclude
  comment-only LOC; `report` warns when sampled files changed; `stats` reads
  manifest counters in O(1).
- A no-op `aw build` skips the database write (2,201 files: 0.09-0.11 s to
  0.02-0.03 s, index byte-unchanged); MCP returns `-32602` for a missing
  required argument.

## [0.2.0] - 2026-11-02

**User impact:** symbol context, call paths, search, outlines, and clone candidates expand navigation beyond individual call queries.
**Required action:** rebuild older indexes with `aw build`; query opens reject incompatible schemas with exit 5.
Review the output-contract changes below if scripts consume query text.

### Added — `aw context <symbol>`

- One call returns the definition line with signature, direct callers, and
  direct callees (resolved `calls` edges only), with per-section showing
  counts and a first-line caveat. `--limit N` per section (default 10).
- Journal `args` gains `name` + `limit`; payload carries the displayed
  pointer lines; MCP tool `context` with G10 byte parity; contract-bump
  entry recorded in the spec.

### Added — `aw path <from> <to>`

- Shortest call path between two symbols over resolved call edges
  (`kind = 'calls' AND dst_id IS NOT NULL`): level-batched BFS, deterministic
  (FIFO frontier, neighbors ordered by `dst qualname, dst path, site_line`,
  multi-edges deduped to the smallest `site_line`, parent = first
  discoverer); `--depth N` (default 10, clamp 1-10).
- Output contract (contract-bump): first-line caveat
  `aw: path — resolved static edges only; dynamic dispatch may hide paths`,
  hop call-site pointers in path order (no cap — `path` has no limit
  surface, recorded deviation), final line
  `aw: chain <resolved qualnames> (K hops)`. Exit 1 negatives distinguish
  exhaustive vs depth-cap-truncated; exit 2 ambiguity names the failing
  endpoint (`as the from|to endpoint`).
- Journaled (`args` gains `to`; no `limit` key); payload
  per result line (`name` = the `from` argument; `usage --verify` unchanged).
  MCP tool `path` with G10 byte parity.
- Spec review: six independent Goldfish reviewers, 73 findings consolidated
  to 20, 2 fix cycles, round-2 APPROVE
   (internal review unavailable publicly); internally reported G17 gate
   evidence (original evidence unavailable publicly): 10/10 exact-byte fixtures,
  8/8 frozen GT pairs, 0.6-1.4 ms in-process best-of-7 across the 85k-symbol
  synth corpus and the 26-file self-index (spawn-inclusive wall < 10 ms).

### Changed — `aw search` readability

- Indexed text widened with `split_ident(path)` + `split_ident(signature)`
  tokens (derived data; schema stays v4; the FTS is repopulated at build
  end). Signature-only vocabulary ("usize", "option path", "any", "i64 vec")
  now retrieves the symbols that carry it — frozen gap set 6/6 improved,
  frozen G14 recall unchanged at 0.850 pre-rebuild; the shipped index
  measures 0.800 (23/28) — the G18(i) premise is corrected in the
  remediation.
- Search hit lines gain the stored one-line signature:
  `path:line qualname signature` (contract bump; empty signature keeps the
  12b byte shape). Payload/G10 parity unchanged.

### Changed — residual closures (2026-11-02)

- PEP 420 namespace-package imports resolve (longest indexed prefix).
- Empty `dead`/`clones` results now print the mandatory caveat on stdout.
- MCP `initialize` negotiates the protocol version with the client.
- Serial-vs-parallel build determinism tested via `AW_WORKERS`.
- GitHub Actions pinned by SHA in both workflows.

### Reverted — `aw search` docstring indexing (attempted)

- Attempted schema v6 `symbols.docstring` + FTS widening. Retrieval improved
  (prose-gap 5/6, G14 unchanged at 0.800) but the Stage 2c utility A/B failed
  (Grok tool calls +12%, OpenCode −9%) — reverted under gate G20's kill rule.
  Evidence retained internally; original Stage 2c evidence is unavailable publicly.

### Fixed — remediation (schema v5)

- Build abort on deeply nested source replaced by an explicit depth guard
  (8,192) and 64 MB worker stacks; 2,000-20,000 nesting now builds cleanly.
- `clones` pair generation bounded (64 regions/group, 20,000 pairs) with a
  lower-bound note — a shared license header no longer OOMs.
- `callees --depth N` now closes exactly N levels (was N+1).
- Fast path heals resolution gaps via schema v5 `unresolved_calls`: a new
  symbol re-extracts only the carried files whose unresolved names match.
- Rust `impl` methods scope to the impl type (grammar `type` field).
- Constructor calls emit edges to both the class and its constructor.
- MCP: 8 MiB frame cap, per-call panic isolation, `--no-journal` honored,
  `stats`/`build` newline parity.
- Grammar identity derived from `Cargo.lock`; `.git/index` no longer
  rewritten; broken pipe exits 0; tracked-but-gitignored files indexed;
  deterministic search tie-break.
- Bounded query cost: SQL LIMIT+COUNT (1.05M-edge callers 550 ms/109 MB ->
  0.14 s/7.4 MB; 1M-symbol search 1.02 s/162 MB -> 4.2 MB), work-queue dirty
  closure (4,000-chain edit 42.5 s -> 0.26 s), search terms cap 32,
  ambiguity render cap 50 with true total.
- Claims corrected: unresolved rate inverted (coverage 10-48% / unresolved
  52-90%), G14 0.800 on the shipped index (was 0.850 before), G13 -62.7%
  (was -64.8%), denominators (40 primary runs + 2 fills; 10-run second host),
  test count 105, self-index epoch-labeled.

### Fixed

- `tests/scripts/write_path_check.sh`: stale exclusion pattern (G5 was red
  on the committed tree after the qualified `std::fs::rename(crate::db::...)`
  refactor); `src/journal.rs` added to the aw_dir-scoped allowlist.
- `db::probe_old`: a v4-stamped index from the first landing
  wave (no `clone_groups` table) now disables the fast path and rebuilds
  fresh instead of failing carry-forward with
  `no such table: olddb.clone_groups`.

### Added — `aw symbols --outline`

- New `--outline` flag on `aw symbols <file>`: each line gains the stored
  one-line signature (first line of the symbol's node text, trimmed, hard
  cap 160 chars, no bodies). Default output (no flag) is byte-identical to
  the previous contract — contract-bump note: additive flag only, pointer
  shape unchanged, exit codes unchanged.
- Schema v4 (additive): `symbols.signature TEXT NOT NULL DEFAULT ''`,
  captured during the build parse phase at zero extra passes; carried
  forward by the fast path. Old indexes exit 5 with the rebuild hint
  (single normative version policy).
- Journal `args` for `symbols` gains the `outline` key (journal
  extension); MCP `symbols` tool gains the optional `outline` arg with G10
  byte-parity coverage.


### Added — agent-tooling design companion (docs)

- `docs/archive/designs/2026-11-01_p4-agent-tooling_amendment.md`: four agent-tooling
  features distilled from the pi-sherlock reference analysis — F1
  `aw symbols --outline` (signature column), F2 `aw search` (FTS5 symbol
  lookup), F3 skill hardening + drift gate, F4 `aw clones` (shingle
  candidates). Proposal spec-reviewed by an independent stateless Goldfish
  reviewer (round 1 NEEDS FIXES with 12 findings, round 2 APPROVE; record
  internal review unavailable publicly). OQ4a (signature text) and OQ4b
  (symbol-slice `search` deferral lift) approved by the owner 2026-11-01.
  Gates G13-G16 registered in the spec's Gate Registry. Implementation
  follows (F3 -> F1 -> F2 -> F4), one schema v4 bump for whatever survives
  the gates.

### Added — `aw search`

- New `aw search <terms>`: symbol-level FTS5 lookup — OR of prefix terms over
  camelCase/snake_case-split identifier tokens, `bm25()`-ranked. Output
  pointers `path:line qualname` (tier position empty — search hits are symbol
  identities, not call edges), standard cap/staleness/exit-code contract;
  empty terms exit 2, no match exits 1 with a reason line.
- Schema v4 (same bump as F1): standalone virtual table `symbols_fts`,
  repopulated by one pass from `symbols` at the end of every build (covers
  fresh and carried rows). No new dependencies (FTS5 is SQLite builtin).
- Journal `args` gains the `terms` key (journal extension); `search`
  joins the payload list; MCP `search` tool with G10 byte-parity.
- G14 gate PASS: mean per-query recall 0.850 on 20 frozen queries
  (pooled 24/28, Wilson 95% [0.685, 0.943]) —
  internally reported G14 evidence (original evidence unavailable publicly).

### Added — `aw clones`

- New `aw clones [--min-lines N]` (default 5): duplicate-block candidates —
  normalized 5-line-window FNV-1a shingles computed during the build parse
  phase (workers already hold the bytes — no extra reads), carried forward
  with the fast path; adjacent matching windows merged per file at query
  time; pair lines `pathA:startA-endA pathB:startB-endB` under the standard
  cap/staleness contract. Mandatory caveat first line: candidates, never
  proof. NOT payload-journaled (deviation: pair lines are not
  single pointers, `usage --verify` structural checks do not apply).
- Schema v4 (same bump as F1/F2): `clone_groups` + `clone_groups_hash`
  index. No new dependencies.
- G16 PASS: 10/10 seeded pairs, 0 spurious (bar >= 8/10, <= 20%); build
  overhead +23.2% interleaved-measured — owner amended the proposed 15% bar
  to 25% with INSERT-I/O attribution on record
  (internally reported G16 evidence; original evidence unavailable publicly).

### Changed — performance ceiling pass

Four changes, all measured on a 2,025-file / 7.9 MB synthetic Python corpus
(index 4.6 MB, same seed as the review baseline) plus the aw self-index; gates
`cargo test --locked` (91 tests), `cargo clippy -D warnings`, G2 self
precision/recall PASS.

- Query path: `PRAGMA quick_check` removed from `open_for_query` (full B-tree
  scan, 0.71-1.10 ms/MB measured, ran on every CLI open and every MCP tool
  call). Corruption now exits 3 via SQLITE_NOTADB/SQLITE_CORRUPT mapping with
  the unchanged contract message; `aw doctor` keeps `integrity_check`.
  `aw callers` best-of-7: 15.1 ms -> 7.6 ms; `aw unresolved` 13.9 -> 7.6 ms;
  latency is now flat at spawn floor + query regardless of index size.
- Build inserts: one transaction per build (500-row prepared statement batches
  retained; per-chunk BEGIN/COMMIT fsyncs dropped). Insert phase measured on
  identical row volume: 154 ms -> 55 ms; db phase 0.39 s -> 0.09 s.
- Parse+extract: parallel on `std::thread::scope` workers
  (`available_parallelism`, capped 8; no new deps, no unsafe, order-preserving
  merge). Parse phase 0.66 s -> 0.11 s; `aw build --force` wall 1.16 s ->
  0.27 s on the corpus, 0.43 s -> 0.13 s on aw self.
- Schema v3 (additive): `files.loc`, `files.cyclomatic` computed during the
  build parse phase; `aw hotspots`/`aw report` read the index instead of
  re-parsing every file per call. hotspots 409-781 ms -> 31 ms; report
  560 ms -> 44 ms. Behavior deltas: files deleted from disk are no longer
  silently skipped by hotspots; metrics are frozen at each file's last fresh
  extraction (changed files are always re-extracted by the fast-path rules).

### Added — navigation baseline and measurement harness (P0, f9d57b9..1ca1d0e)

- Frozen A/B baselines: 5 navigation task definitions (T1-T5), 30 verified
  ground-truth call sites on corpus-a, provisional top-10 pain list.
- D0 harness: `g` call logger (byte/call accounting both directions),
  `count.py`, `check.py` (precision/recall with AMBIGUOUS-candidate and
  dynamic-dispatch rules), derivation scripts, cold-run equivalence check.
- Grep-arm baseline: 12 tool calls / 49,224 bytes; P0 exit criterion passed
  (premise not weak).
- R0 independent review record (APPROVE after 1 fix cycle).

### Added — local call-graph navigation (9f75656..e72b9b0)

- `aw` Rust CLI (single binary, 5 pinned deps, `#![forbid(unsafe_code)]`):
  `build`, `callers`, `callees [--depth N]`, `impact`, `doctor`, `stats`,
  `hotspots`, `report`.
- Resolution algorithm: local module -> import alias map (with re-export
  following) -> receiver-class (self/this, constructor assignment, typed
  params, single-constructor fields, MRO over known bases) -> builtin table.
- Full-atomic-replacement builds with per-file fast path, flock build lock,
  FNV-1a staleness on the post-cap displayed set.
- Exit-code contract 0-7; response contract with caps and tier tags.
- Safety: zero network (Seatbelt/seccomp CI), Cargo.lock allowlist,
  write-path grep, tree-snapshot (content-hashed), proc-hygiene test.
- 54 tests; clippy `-D warnings` clean.
- Gate results on corpus-a (~102k LOC): G1 1.91s/21.4MB/1.9MB; G2 recall 29/29,
  precision 493/495; G3 p95 10.1ms; G6 -50% tool calls, -67% bytes.
- Independent review record (APPROVE after 2 fix cycles).

### Added — index statistics and repository reports (d9951ce..5034939)

- `aw stats`, `aw hotspots` (per-metric min-max normalization of
  cyclomatic + LOC + churn-90d + fix-density), `aw report` (4 sections).
- G7 rank-overlap 8/10 vs pain list (degraded: pain list provisional).
- G8 -67% tool calls on architecture questions (via `callees --depth`).
- `code-index` skill wrapper in the agent-skills repo (OQ2).
- Independent review record (APPROVE after 1 fix cycle).

### Added — TypeScript, JavaScript, and Rust indexing (2aeeb9d..7a99343)

- TypeScript/TSX and JavaScript/JSX grammars (one-commit upgrade rule);
  Rust grammar (rs-0.24.2).
- TS/TSX extraction: classes, methods, arrow-const components/hooks,
  `new` -> constructor redirect, `this`/`super` MRO calls, ESM imports
  (default/named/namespace/type), directory-based relative resolution
  (dot-stem safe), external-module drop rule.
- Rust extraction: functions, structs/enums/impls, `use` and `mod`
  declarations, `crate::` call paths, scoped identifiers.
- G2-corpus-b: 20/20 recall, 57/57 precision. G2-self (dogfooding): 9/10,
  14/14. Python regression green throughout.
- Independent review record (APPROVE after 2 fix cycles).

### Added — docs

- In-repo canonical spec + concepts brief under `docs/archive/designs/` (cross-repo
  references retired); GitHub community health files.

### Fixed — post-review

- Fast path: files with any call/import/inherits edge into a changed file are
  re-extracted (transitive dirty closure). Previously their copied edges died
  with the old destination ids and `aw callers`/`aw impact` silently lost
  results after any incremental build.
- Schema v2 (additive): per-file `files.unresolved` column;
  `unresolved_callee_count` is now an authoritative SQL total and no longer
  inflates monotonically on incremental builds.
- mtime keys use nanosecond precision; same-second, same-size edits no longer
  defeat the fast path or the staleness check.
- Build lock: orphan sidecar cleanup happens after the flock is held, so a
  second concurrent build can no longer delete the in-progress
  `index.db.new` of the build holding the lock.
- MRO resolution: multiple same-level matches (diamond) now emit an AMBIGUOUS
  edge as documented, instead of silently dropping the call site.
- hotspots: language-aware cyclomatic proxy (Python/TS/JS/Rust) and churn/fix
  filters over all source extensions, not `.py` only.
- `callers`/`callees`: no false ambiguous match when a symbol's bare name
  equals its qualname; `--max-parse-error-rate` is validated to [0, 1].
- README review-records table corrected (early cycles in-tree; later cycles live in
  commit history).

### Added — usage observability layer

- Result-payload journal: `.aw/usage-results.jsonl` — one self-contained
  JSONL line per successful pointer query (callers/callees/impact/dead/
  symbols) carrying the displayed set byte-identical to the stdout data
  lines plus index `built_at`/`source_rev`. Non-pointer tools and
  empty/ambiguous/error outcomes write none. Rotation (5 MB), opt-out
  (`AW_NO_JOURNAL` / `--no-journal`), and best-effort contract mirror
  usage.log.
- Session attribution: `AW_SESSION`/`AW_ARM` env tags ride every usage.log
  and payload line as additive fields — A/B attribution per task
  without transcript parsing.
- `aw usage --verify [--limit N]`: deterministic even-stride sample over
  payload entries; the index's staleness check separates CHANGED
  (unverifiable, never counted wrong) from OK/FAIL; unchanged files get
  structural checks (line bounds; `callers` also requires the queried
  name's bare segment on the pointer line). Report-only exit 0; exit 1 on
  an empty payload log. The MCP `usage` tool accepts optional
  verify/limit args (G10 byte parity preserved per invocation).
- `aw usage` payload section: entries/pointers totals and top-5 queried
  names by call count (demand map). `aw doctor` reports payload journal
  volume or its explicit absence.
- G11 gate (payload conformance) added to the Gate Registry: one payload
  line per successful pointer call, byte-equal pointers, verify FAIL=0 on
  fresh builds. Conformance run on the 3 GT repos (aw self / corpus-a /
  corpus-b): 594 pointers verified, FAIL=0; byte-parity payload-vs-stdout
  confirmed. Tests 91/91; clippy -D warnings clean.
- Support changes: index meta `built_at` now carries millisecond precision
  (the verify epoch comparison must not collide on same-second rebuilds);
  aw-self GT rows G01/G02/G07/G09/G10 refreshed after the journal line
  shifts (provenance in the GT file; G06 annotation corrected — the M3-era
  file mislabeled the Dead dispatch as Stats).

### Added — prebuilt index artifacts

- Dual-key fast path: (mtime, size) match carries as before; a (mtime,
  size) mismatch with same size AND matching FNV-1a content hash also
  carries — a restored `index.db` from a CI cache is reused without
  re-parse. Hash-carried rows get their stored mtime refreshed to the fresh
  checkout's stat (staleness contract stays truthful).
- Cost model: extra reads only in the restore case (one read+hash per file,
  parse+extract still skipped); the normal workflow pays zero extra reads.
- Dirty closure, parse-quality gate, and version-gate semantics unchanged.
- Restore-equivalence test: cache -> fresh checkout -> build carries all
  files, queries byte-correct, no stale flags, and a subsequent real edit
  still re-resolves via the dirty closure.

### Added — MCP stdio server

- `aw mcp`: newline-delimited JSON-RPC 2.0 server on stdin/stdout; 21 tools
  mapping 1:1 to CLI subcommands (doctor excluded — `--invalidate` stays a
  human-CLI maintenance action). Spawned by the agent host; aw still spawns
  no daemons. Zero network; writes confined to `.aw/`.
- Tool results return the CLI's byte-identical stdout text (G10 parity
  conformance is automated in tests/mcp.rs); ambiguous names carry the
  candidate list in the result text with isError (exit-2 semantics
  preserved); unknown tools rejected with -32602.
- J2: every tool call journals with `surface:"mcp"` (same journal
  schema); the server process itself is not journaled.
- Dependency policy (additive): `serde_json = "1"`
  joins the pinned set — the MCP boundary parses untrusted client JSON;
  `rmcp` rejected (tokio runtime tree contradicts minimal-dependency/no-daemon
  build semantics).
- Structural refactor: `run_build` moved to shared `src/build.rs` (CLI and
  MCP share it); ambiguous-name candidates now travel in
  `AwError::Ambiguous` instead of being printed mid-query (stdout stays a
  pure protocol/data channel on every surface).
- aw-self GT rows G02-G10 refreshed after the M3 line shifts (provenance in
  the GT file); G2 corpus-a 29/29 + 493/495, corpus-b 20/20 + 57/57 unchanged.
- Tests 76/76 (3 new MCP conformance tests); clippy -D warnings clean.

### Added — resolution recall

- `crate::a::b` calls now resolve when intermediate segments are plain
  namespaces (src/ layout without mod.rs): module-chain attempt first, then a
  direct qualname lookup under the crate root. Root cause of the G2-self G01
  gap was deeper: `dotted_chain` did not recognize the Rust `crate` keyword
  node, so whole `crate::` call chains were never recorded — fixed.
- Receiver typing (typed params, constructor assignments) now takes
  precedence over module interpretation for 2-segment calls: a local binding
  shadows a same-named module, matching Python scoping.
- aw-self GT rows G02-G10 refreshed after M1's line shifts (provenance noted
  in the GT file); G01 unchanged.

### Added — usage journal

- Every journaled subcommand appends one JSONL line to `.aw/usage.log`:
  ts, surface, tool, args summary (names/paths/scalars only — never file
  contents), result/shown/total, ambiguous count, exit code, latency, stale
  flag, aw version. Opt-out: `AW_NO_JOURNAL=1` or `--no-journal`. Rotation
  at 5 MB (one prior generation). Best-effort writes — journal failures can
  never alter output or exit codes.
- `aw usage` — journal summary: calls per tool, error/empty/stale rates,
  ambiguous pointer counts, p50/p95 latency. Read-only; no index required;
  exit 1 on an empty journal.
- QueryOutput carries `stale` (header fired on the displayed set) for
  journaling; `unresolved`/`symbols` record their own result counts.

### Added — query surface

- `aw dead` — dead-code CANDIDATES (zero inbound call edges, AMBIGUOUS-name
  aware) with a permanent first-line caveat: candidates, never proof.
- `aw unresolved` — per-file unresolved-callee counts, descending; exit 1 on
  full coverage.
- `aw symbols <file>` — one file's symbols as `path:line kind qualname`.
- Resolution fix: `from .x import y` inside a repo-root plain module resolved
  to ".x" (stray leading dot) and the call was misclassified as dropped
  external; root-level relative imports now resolve to the sibling module
  (fixture regression test added).
- Test-fixture fix: the shared fixture constants lost their indentation to
  Rust `\`-line continuations, so class bodies collapsed to top-level defs
  and method resolution was never exercised; fixtures are now raw strings
  (all 63 tests pass against the semantic fixtures).

### Changed — M0 re-baseline (Elephant/Goldfish cycle)

- All measurable gates re-run on the post-fix binary (internal M0 rebaseline
  evidence unavailable publicly): G1 0.47s/22.8MB/1.98MB, G2 corpus-a 29/29 + 493/495,
  corpus-b 20/20 + 57/57, self 9/10 + 14/14 (all matching pre-fix numbers —
  zero regression), G3 p95 9.8ms, G7 8/10.
- G2-corpus-b G02 row refreshed (corpus-b working tree moved; GT staleness,
  not a regression). G6/G8 A/B re-runs marked pending with rationale.
- README gates table updated to the re-baselined numbers.

### Skipped

- Stdio MCP server (original trigger): "CLI discoverability measurably
  fails" never occurred — the phase was skipped per its trigger rule, then
  ACTIVATED by owner decision (MCP-native demand), recorded as
  an additive dependency change.
- M5 additional languages (Go/Java/C#): trigger "real workload demand" not
  met — no such workload identified. Re-evaluable on request; the
  per-language template (grammar pin -> extraction -> fixtures -> GT -> G2)
  remains the playbook.

[Unreleased]: no tagged releases yet — history starts at f9d57b9 (repo init).
