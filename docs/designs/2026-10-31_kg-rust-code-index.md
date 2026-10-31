# kg — Local Code Knowledge Index (Rust)

- Author: Quy Doan (spec compiled from consultation session 2026-10-30/31)
- Canonical location: this file, in the kg repo. A historical copy lives in
  the agent-skills repo (`docs/designs/`); the kg copy is authoritative —
  amendments land here first.
- Status: Approved (owner approved P0 proceed, 2026-10-31; supersedes "Draft (Gate 2 pending)")
- Date: 2026-10-31
- Concepts brief: `docs/designs/2026-10-31_concepts-brief_kg-rust-code-index.md`

## Overview

`kg` is a single-binary local CLI that indexes a repository's symbols and call/inheritance edges into SQLite and answers three navigation questions for AI coding agents and the owner: who calls this, what does this call, what is impacted by this file. It is consultative only: it registers no hooks, injects no prompts, runs no daemon, and touches no network. Built in Rust with tree-sitter; Phase 1 targets Python (`vihap`, ~102k LOC), Phase 2 adds TypeScript/TSX/JSX (pilot: `~/Workspace/personal/finguard`, a ~12k-LOC TS/TSX monorepo, plus the React 19 web frontends `mskit-ui`/`qauto-ui`); C is out of scope (verified absent from the workload — earlier .c counts were vendored venv sources). Every design element is distilled from six analyzed third-party tools with recorded provenance; every gate is kill-able.

## The Problem

Agent sessions navigate code by grep/read loops. On large repos this costs many tool calls and input bytes; on proprietary code, cloud alternatives (repowise) upload the code, and local tools each violate a trust requirement (rtk rewrites commands pre-execution; headroom interposes on provider credentials; context-mode pseudo-sandboxes and self-updates; codegraph bundles a Node runtime with telemetry and 1.6-1.7GB RSS). Current workaround: plain grep + AGENTS.md discipline. Measured baseline of that workaround is a Phase 0 deliverable, not yet recorded.

## Goals

- G-A: on 30 pre-verified call sites in vihap — precision (correct resolutions / all emitted resolutions) >= 80%, EXTRACTED-tier precision >= 95%, recall (true callers found / true callers in ground truth; an AMBIGUOUS result counts as a hit only when the true callee appears in its candidate list) >= 70%; Wilson 95% interval reported alongside every rate.
- G-B: vihap build < 5s wall, < 100MB RSS, < 50MB DB, query p95 < 20ms (targets until measured in P0/P1; all gates carry target labels until a measurement run exists).
- G-C: utility A/B shows kg >= 30% tool-call reduction on navigation tasks vs measured grep baseline, with no increase in total transcript bytes (both directions: tool-result bytes returned to the agent and agent tool-call input bytes — accounting defined in the D0 harness) and no correctness loss vs ground truth.
- G-D: four safety invariants hold and are CI-enforced (below).
- G-E: one static binary, five pinned dependencies, zero network syscalls, zero unsafe Rust in this crate (tree-sitter C runtime accepted as pinned dependency).

### Gate Registry (normative numbering)

| Gate | Goal | What | Pass command | Kill action |
|------|------|------|--------------|-------------|
| G1 | G-B | build performance on vihap | `hyperfine 'kg build'` (prereq: `brew install hyperfine`; fallback `tests/scripts/timing.sh` using `/usr/bin/time -l`) | retune or stop |
| G2 | G-A | precision + recall on 30 ground-truth sites | `tests/scripts/precision_recall.sh` | retune resolution algorithm or stop |
| G3 | G-B | query latency p95, hot + cold page cache | `hyperfine --warmup 3 'kg callers <hot_fn>'` x 30 runs | retune or stop |
| G4 | — | staleness detection + false-stale rate | edit-after-build integration test | fix |
| G5 | G-D/G-E | safety invariants 1-3 (CI) | seatbelt + seccomp + proc-table + tree-snapshot tests | fix before any ship |
| G6 | G-C | utility A/B vs P0 baseline | D0 harness: 5 tasks x 2 modes x 2 runs + correctness sampling | project stops; fallback Alternative B |
| G7 | P1.5 | hotspots rank overlap vs owner pain list | comparison script, >= 7/10 | feature dropped |
| G8 | P1.5 | report utility on architecture questions | A/B, >= 30% tool-call reduction | feature dropped |

### Per-Phase Independent Review Protocol (normative)

Each phase ends with an independent expert review before the next phase starts:

- Reviewer: a stateless subagent with fresh context (Goldfish contract). Inputs: deliverable paths + that phase's acceptance criteria + the lens focus below. Never conversation history or author rationale.
- Verdict: `APPROVE` or `NEEDS FIXES` with per-finding severity (Critical/Important/Suggestion/Nit) and required fix. Gate: 0 open Critical + Important.
- Fix bound: <= 2 fix cycles per phase (rules/AGENTS.md §1 canonical bound); unresolved after that -> phase blocked, escalate to owner.
- Review records land in `docs/baselines/reviews/R<n>-<phase>.md` with the finding list and dispositions.

| Review | After phase | Expert lens focus |
|--------|-------------|-------------------|
| R0 | P0 baseline | Measurement methodology: baseline validity, sampling representativeness, byte accounting both directions, harness correctness, no mode bias |
| R1 | P1 core | Rust systems + program analysis: spec conformance, resolution-algorithm correctness, gate evidence audit (every number has its command), idiomatic Rust (clippy -D warnings, error handling, no unsafe) |
| R1.5 | P1.5 extensions | Agent integration + UX: response-contract readability for agents, wrapper violates no no-hijack invariant, hotspots metric honesty, install verification |
| R2 | P2 languages | Multi-language parsing: TS/TSX/JSX extraction correctness, React patterns (hooks, component composition), grammar upgrade rule applied, Python regression fixtures green |
| R3 | P3 MCP (if built) | MCP/protocol: stdio lifecycle, watchdog, schema exposure, per-session process behavior |

## Non-Goals

- No MCP server in P1 (Phase 3 optional). No daemon/watcher/auto-rebuild. No LLM calls, no embeddings, no telemetry. No verbatim source responses. No code modification. No Windows guarantee. No hooks or agent-config writes. No framework route mapping, no dynamic-dispatch resolution (explicit non-goal, see resolution algorithm). No C indexing (verified absent from the workload). No Go indexing (one repo, outside the owner's stated stack). No HTML/Markdown doc-layer indexing (no call-graph value; not requested). `symbols`/`search` subcommands are deferred until universal-ctags/ast-grep demonstrably fail the workflow (they cover them today at zero maintenance).

## Open Questions

- OQ1 (owner, before P1 end): final repo home path (default `~/Workspace/kg`) and license choice (default: private, no LICENSE file until published).
- OQ2 (owner, at P1.5): skill wrapper `code-index` in agent-skills — requires updating the hardcoded count asserts in `tests/test_setup_script.sh` (27) and the usage-text "all 38" in `setup.sh`; the printed count itself adapts dynamically. Scheduled after G6 passes.
- OQ3 (resolved 2026-10-31): C preprocessor strategy — moot; probe verified zero project C code in the workload (earlier .c counts were vendored `.venv` sources). P2 targets TypeScript/TSX/JSX instead.
- Amendment 1 (2026-10-31, P1 authoring): schema v1 gains `edges.site_line INTEGER NOT NULL DEFAULT 0` — the call-site line. G2 scores against 30 call-site ground-truth rows (`caller file:line`), which is only achievable if caller pointers carry the call-site line, not the enclosing symbol's start line. Additive-only policy honored (column added before first release).
- Amendment 2 (2026-10-31, G6): `kg callees` gains `--depth N` (default 1, clamp 1..10) — transitive call closure via recursive CTE. Load-bearing for G6: flow tasks collapse from 5 chained queries to 1. Output format unchanged.
- Amendment 3 (2026-10-31, R1): error-class taxonomy — the spec's CUJ7 message used class `no-index` while the spec's KgError enum has no NoIndex variant. Resolved: exit 1 uses class `no-results` for both no-index and empty results; detail strings distinguish ("no .kg/index.db under <root> — run kg build" vs "0 callers found (dynamic dispatch possible — grep fallback recommended)" / "0 callees found" / "0 impacting files found"). Star-import import edges carry the module symbol id with confidence AMBIGUOUS (schema comment updated) — calls-edge AMBIGUOUS rows keep dst_id NULL.
- Amendment 4 (2026-10-31, post-R2 review): fast-path correctness. (a) A carried file with ANY edge (calls/imports/inherits) into a changed file is re-extracted via a transitive dirty closure over the previous index — previously its copied edge died with the old destination id and `kg callers`/`kg impact` silently lost results. (b) Schema v2 (additive): `files.unresolved INTEGER NOT NULL DEFAULT 0` — per-file unresolved-callee count; `unresolved_callee_count` becomes an authoritative SQL sum, stable across incremental builds (it previously inflated monotonically). (c) `files.mtime` and staleness keys use nanosecond precision — same-second, same-size edits no longer defeat the fast path. (d) Orphan sidecar cleanup happens after the build flock is held (a second concurrent build could previously delete the in-progress `index.db.new`). (e) MRO resolution: two or more same-level matches emit an AMBIGUOUS edge as documented (previously the call site was silently dropped). (f) `kg hotspots` uses a language-aware cyclomatic proxy and churn/fix filters over all source extensions (previously Python-only in the multi-language build).
- Amendment 5 (2026-10-31, M1): query surface additions. `kg dead [--limit N]` — dead-code CANDIDATES: symbols with kind `function`/`method`/`class` that have no resolved inbound `calls` edge (`dst_id` = symbol id) AND no AMBIGUOUS `calls` edge whose `dst_name` equals the symbol's qualname or bare name; pointer contract applies (cap, `showing N of M`, staleness, exit 1 when none) and stdout always carries the first-line caveat `kg: dead-code candidates — dynamic dispatch, entry points, and external callers are not proven absent`; the output is a candidate list, NEVER proof of deadness. `kg unresolved [--limit N]` — report shape (no pointers): files with `files.unresolved > 0`, descending by count, lines `path unresolved=N`, header `kg: unresolved=T callees in M files`, capped; exit 1 with `0 unresolved callees (full call-site coverage)` when the index has no gaps; no staleness pass (counts are index-internal). `kg symbols <file> [--limit N]` — one indexed file's symbols as `path:line kind qualname` ordered by `line_start`; single-file staleness check produces the standard header; exit 1 `path '<file>' not indexed` for unknown paths.

## Critical User Journeys

### CUJ 1: Build (happy path)
- Given: vihap repo with `.git`, no `.kg/` directory.
- When: `kg build` in any subdirectory of vihap.
- Then: exit 0; stats line on stdout (files, symbols, edges, parse-error count, unresolved-callee count, per-phase timings, CPU-seconds); `.kg/index.db` + manifest present; `git status --porcelain` shows nothing outside `.kg/`.

### CUJ 2: Navigate (agent question)
- Given: index built; 200+ callers for a hot symbol; one result file edited after build.
- When: `kg callers hot_fn`.
- Then: stdout pointer list `path:line [tier]` capped at 50 by default, final stdout line `showing 50 of 213`, stderr `[kg] capped at 50 results (213 total)`, stdout header line `kg: 3 of 12 result files changed since build — run kg build`; tier tags on every line; exit 0.

### CUJ 3: Failure (corrupted index)
- Given: `index.db` corrupted (truncated file).
- When: `kg callers foo`.
- Then: exit 3; stderr `kg: corrupt-index: index.db failed integrity check — run kg build`; no panic; no partial output.

### CUJ 4: Ambiguous name
- Given: `process` exists as method in 3 classes.
- When: `kg callers process`.
- Then: exit 2; stdout candidate list `path:line qualname` x3; message names the `Class.method` form; no merged-silent result.

### CUJ 5: Version skew
- Given: index built by kg v0.1 (schema v1); binary upgraded to schema v2.
- When: `kg callers foo`.
- Then: exit 5; `kg: schema-mismatch: index schema v1, binary expects v2 — run kg build`. Old binary reading v2 DB: exit 5 with mirror message.

### CUJ 6: Concurrent builds
- Given: two agent sessions run `kg build` in the same repo.
- When: second build starts while first holds `.kg/build.lock`.
- Then: second exits 4 with `kg: locked: build in progress (pid N) — retry after it exits`; no interleaved writes.

### CUJ 7: Query without index
- Given: repo never indexed; no `.kg/` directory.
- When: `kg callers foo`.
- Then: exit 1; `kg: no-index: no .kg/index.db under <root> — run kg build`; nothing else on stdout.

### CUJ 8: Parse-quality abort
- Given: repo where 12% of .py files fail tree-sitter parse; threshold default 5%.
- When: `kg build`.
- Then: exit 7; `kg: parse-quality: 12% of M files failed to parse (threshold 5%) — inspect with kg doctor or override with --force`; `--max-parse-error-rate <f>` adjusts the threshold; `--force` proceeds and records per-file `parse_status`.

## Technical Plan

### Component layout (single crate, no workspace)

```
src/main.rs     entry, exit-code mapping
src/cli.rs      clap definitions, help texts
src/walk.rs     ignore-crate walk; excludes .kg/ unconditionally; per-file size cap 1MB (skipped + counted)
src/parse.rs    tree-sitter-python; ERROR-node counting per file
src/extract.rs  symbols/edges; resolution algorithm; confidence tiers
src/db.rs       rusqlite; schema; user_version; manifest; flock; atomic swap
src/query.rs    callers/callees/impact; staleness check (result-set scope); caps; tier tags
src/doctor.rs   integrity, versions, parse coverage, orphans, lock state
src/hash.rs     FNV-1a 64 (raw bytes, no normalization)
src/error.rs    KgError enum, stable stderr prefix `kg: <class>: <detail>`
schema.sql      embedded via include_str!
```

Dependencies (pinned, complete): `clap` (derive), `tree-sitter` + `tree-sitter-python` (P1) + `tree-sitter-typescript`/`tree-sitter-javascript` (P2; tsx grammar ships in the typescript crate) + `tree-sitter-rust` (P2.5, optional), `rusqlite` (bundled), `ignore`. No rayon (single-threaded P1; add only if G-B fails, then <= 4 workers). No serde. `#![forbid(unsafe_code)]` at crate root.

### Schema (v1, additive-only evolution policy)

```sql
PRAGMA user_version = 1;                      -- schema contract; normative policy below
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
-- meta rows: kg_version, grammar_version, built_at, source_rev, file_count, symbol_count, edge_count, parse_error_count, unresolved_callee_count
CREATE TABLE files(id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE, size INTEGER NOT NULL, mtime INTEGER NOT NULL,
                   content_hash INTEGER NOT NULL, parse_status TEXT NOT NULL, error_nodes INTEGER NOT NULL);
CREATE TABLE symbols(id INTEGER PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL,
                     qualname TEXT NOT NULL, file_id INTEGER NOT NULL REFERENCES files(id),
                     line_start INTEGER NOT NULL, line_end INTEGER NOT NULL);
CREATE TABLE edges(id INTEGER PRIMARY KEY, src_id INTEGER NOT NULL REFERENCES symbols(id),
                   dst_id INTEGER REFERENCES symbols(id),          -- NULL when AMBIGUOUS
                   dst_name TEXT NOT NULL, kind TEXT NOT NULL,     -- calls | imports | contains | inherits
                   confidence TEXT NOT NULL);                      -- EXTRACTED | AMBIGUOUS (INFERRED deferred to P2)
CREATE INDEX edges_dst ON edges(dst_id, kind);
CREATE INDEX symbols_qualname ON symbols(qualname);
```

No FTS5 table in P1: no shipped P1 surface queries it (AMBIGUOUS candidates come from the exit-2 candidate list backed by `symbols_qualname`); additive policy permits adding it in a later phase.

Version policy (normative, single statement): the binary opens a DB only when `PRAGMA user_version` equals its own compiled schema version N — older and newer both exit 5 with a rebuild hint. The DB is never migrated in place.

### Build semantics (single rule, resolves all skew hazards)

`kg build` always produces a **full atomic replacement of the DB file**: extract -> write to `.kg/index.db.new` (fresh schema, `PRAGMA user_version` stamped) -> checkpoint + close (WAL removed) -> `rename(2)` `index.db.new` onto `index.db` — atomic, so a concurrent reader sees the old or the new DB, never a missing one; no delete step exists, and orphan sidecars are cleaned at build start and by `doctor`. Per-file extraction fast path: a file whose (mtime, size) match AND whose stored rows were extracted under the same schema and grammar versions is carried forward without re-parse, UNLESS it is in the transitive dirty closure over edges into changed files (Amendment 4a — a carried file with any edge into a changed file is re-extracted, or its copied edge dies with the old destination id); any schema/grammar version mismatch disables the fast path for the whole build. `kg build --force` disables the fast path unconditionally (re-extract everything). `.kg/build.lock` (`flock`, holds pid + kg version) is taken for the whole build; second build exits 4 (CUJ6). Rollback = reinstall old binary + `kg build` (< 5s target). One-command exit to grep-only: `rm -rf .kg/`. `kg doctor --invalidate` deletes the index.

### Call-target resolution algorithm (core IP — normative)

Input: callee expression at a call site. Resolution order, first match wins:

1. **Local module**: name defined in the same file or same package module -> candidate(s).
2. **Import alias map**: per-module table of `import x as y` / `from m import n as y` bindings -> resolve through to source module.
3. **Receiver-class method**: call `self.m()` inside class C, or `obj.m()` where `obj` is assigned from a constructor of exactly one known class in scope -> candidate(s) in that class MRO.
4. **Builtin table**: hardcoded list (stdlib builtins) -> edge dropped, counted in stats.

Outcome: exactly one candidate -> `EXTRACTED` edge. Multiple candidates -> `AMBIGUOUS` edge carrying `dst_name`, `dst_id` NULL (candidates resolvable via FTS by the agent). Zero candidates -> no edge; count reported in `build` stats.

Explicit non-goals: `getattr` dynamic dispatch, monkey-patching, star-import targets (star import -> `AMBIGUOUS` edge to module symbol), string-based dispatch. Fixture corpus must include: aliased imports, shadowed method names across classes, star import, `getattr` call, variable reassignment.

### Response contract (versioned; golden snapshots in CI)

- Pointers: `repo-root-relative/path:line [tier-tag]`; line numbers are 1-based (tree-sitter's 0-based rows are offset at emission); qualname is the dot-joined lexical path (`module.Class.method`). Tier tag is ` [ambiguous]` or empty (EXTRACTED).
- Cap: default 50; `--limit N` — N < 1 rejected (exit 2), N > 10,000 clamped to 10,000. When capped: final stdout line `showing N of M`; stderr `[kg] capped at N results (M total)`. Contract byte-stable; changes require contract-bump release note + wrapper update in same release.
- Staleness: checked on the **post-cap displayed set only** (cost bounded by the displayed count): per file, `stat(mtime, size)` short-circuits; a mismatch triggers FNV-1a re-hash. Header on stdout `kg: K of N result files changed since build — run kg build`. Never auto-rebuild.
- Empty result: exit 1 with a reason line distinguishing "symbol not in index" / "path not indexed" / "0 callers found (dynamic dispatch possible — grep fallback recommended)".
- No index at all: exit 1, `kg: no-index: no .kg/index.db under <root> — run kg build` (CUJ7).
- Exit codes: 0 success; 1 no results / no index; 2 usage or ambiguous name; 3 corrupt index; 4 locked; 5 schema mismatch; 6 internal; 7 parse-quality abort (`kg: parse-quality: N% of M files failed to parse (threshold T%) — inspect with kg doctor or override with --force`, CUJ8).

### Safety invariants and test mechanisms

1. **Zero network**: no network syscalls. Tests: dependency allowlist check (no crate may link a resolver/TLS stack — CI greps `Cargo.lock` against allowlist); macOS CI job runs `kg build` under a Seatbelt profile denying `network*`; Linux job under seccomp-deny profile.
2. **No long-lived process**: test asserts zero child processes remain after each subcommand (proc table scan).
3. **Read-only over source**: integration test snapshots repo tree+hashes before/after `build`/`query`; diff must be empty outside `.kg/`.
4. **No behavior hijack**: review-checked in spec + CI grep: no writes outside `.kg/`, no settings/hooks/config files touched (constant list of allowed write paths).

Invariant wording: "no unsafe Rust in this crate; the tree-sitter C parser runtime is an accepted, pinned dependency."

### Tree-sitter versioning rule

Runtime crate and all grammar crates upgrade together in one commit. Golden fixture corpus (per-file node counts + kinds) must replay identically in CI; otherwise the bump is rejected. `grammar_version` recorded in meta at build; `doctor`/`stats` surface mismatch.

## Alternatives Considered

### Alternative A: Python stdlib `ast` indexer
- Pros: zero dependencies, fully auditable, 3-5 days.
- Cons: no path to multi-language (Phase 2), no static binary, no speed headroom; owner explicitly chose Rust for speed, Phase 2, and Rust experience.
- Reason for rejection: owner decision 2026-10-31; Python remains the fallback if Rust G-B targets prove unreachable.

### Alternative B: Adopt codegraph upstream (per-project)
- Pros: strongest published A/B evidence (-88% tool calls, -62% tokens on 7 large repos), MIT.
- Cons: bundled Node runtime, anonymous telemetry, 1.6-1.7GB RSS, +80% residual context (vendor-reported, unverified), no control over supply chain.
- Reason for rejection: violates control requirement. Retained as the designated fallback if G-C fails.

### Alternative C: universal-ctags + ast-grep only (no kg)
- Pros: zero build, zero maintenance, local, permissive licenses; covers symbol listing and structural search.
- Cons: no call-graph edges, no impact analysis — the three questions kg exists for.
- Reason for rejection as sole solution; adopted as P0 baseline tooling and complement.

### Alternative D: repowise
- Pros: community-endorsed (r/ClaudeCode review), wiki + health scores.
- Cons: cloud service — proprietary code leaves the machine; closed-source engine (public repos are an unrelated RAG chatbot, no-license).
- Reason for rejection: control requirement; concept re-implemented locally as P1.5 `hotspots`/`report`.

## Non-Functional Requirements

- Security and privacy: four invariants above; no secrets handling at all (no keys exist); `.kg/` gitignored by tool on first build (tool writes `.kg/.gitignore` containing `*` — inside its own directory, allowed by invariant 3's constant list).
- Performance and latency (targets until measured; P0 records grep baseline, P1 records kg numbers): build vihap < 5s wall / < 100MB RSS (streamed chunked inserts, no full-corpus accumulation) / < 50MB DB; query p95 < 20ms measured hot and cold page cache; concurrency: p95 under 4 parallel query processes + 1 build in flight; scaling: one synthetic 1M-LOC corpus run records wall/RSS/DB and stated scaling behavior.
- Rollout and rollback: per-repo adoption (vihap first); rollback = `rm -rf .kg/` (documented one-command exit); version skew handled by exit 5 + rebuild; read window: the binary opens only its own compiled schema version N — older and newer both exit 5 (single normative policy, matches CUJ5).

## Detailed Implementation Plan

### Phases

- **P0 — Baseline (no Rust, ~1 day)**: freeze 30 call sites + top-10 pain list (owner, 30 min); record grep-only transcripts for 5 standardized tasks; commit baselines + task definitions to kg repo `docs/baselines/`. Deliverable D0: A/B harness (task scripts + byte/call counters + correctness checker). Gates G-A..G-E numbers finalized against reality. **P0 exit criterion (cheapest kill)**: if the grep baseline answers every navigation task in <= 2 tool calls, the premise is weak — stop before P1 and keep universal-ctags/ast-grep; if the projected reduction is clearly below 15%, present findings to the owner before spending P1.
- **P1 — Core (Rust, ~6-10 days)**: files in ledger below; gates G1-G5 (Gate Registry); then G6 utility A/B (D0 harness, correctness sampling included). Kill rule: G6 fail after one fix cycle -> project stops, fallback Alternative B.
- **P1.5 (only after G-C pass)**: `report` (4 sections), `hotspots` (score formula: cyclomatic + LOC + churn-90d + fix-density; validated by G7 rank-overlap >= 7/10 vs owner pain list), `stats`; golden response-contract freeze; skill wrapper in agent-skills (OQ2); A/B transcripts committed (G8: >= 30% tool-call reduction on architecture questions).
- **P2 — TypeScript/TSX/JSX on finguard (~3-5 days)**: add `tree-sitter-typescript` (+ tsx) and `tree-sitter-javascript` (.jsx) grammars in one commit (runtime+grammars rule, golden replay); extract TS symbols (function, class, method, component, hook), imports/exports edges, receiver-class resolution for `this.`/typed calls; index the pnpm workspace whole (node_modules excluded by the `ignore` crate); freeze 20 ground-truth call sites in finguard -> G2 rerun (>= 80% / >= 70%); Python regression fixtures stay green. finguard is read-only; writes only under its `.kg/`.
- **P2.5 (optional) — Rust self-index**: add `tree-sitter-rust`; kg indexes its own repository (dogfooding); G2 rerun on kg's own fixtures.
- **P3 (optional)**: stdio MCP server wrapping the same query functions; 90s hard-exit watchdog; only if CLI discoverability measurably fails.

### File Ledger (repo `~/Workspace/kg`, all Create)

#### Cargo.toml
- Scope: manifest; five pinned deps; release profile (lto=thin, strip).
- Rationale: complete dependency disclosure for the allowlist test.

#### Cargo.lock
- Scope: committed; CI allowlist test reads it.
- Rationale: reproducible builds; supply-chain audit surface.

#### schema.sql
- Scope: embedded DDL above; user_version = 1.
- Rationale: schema is a contract; single readable source.

#### src/main.rs, src/cli.rs
- Scope: clap subcommands (build, callers, callees, impact, doctor [+ --invalidate], build --force); exit-code mapping; help texts stating direction semantics (impact = "files that call into <file>").
- Rationale: CLI contract per response contract section.

#### src/walk.rs
- Scope: `ignore` WalkBuilder from git root discovered upward; no git root ancestor -> build refuses (exit 2) unless `--root <path>` is passed; skip `.kg/` unconditionally; skip files > 1MB (counted); Extensions allowlist (.py in P1; .ts/.tsx/.js/.jsx in P2; .rs in P2.5).
- Rationale: CUJ1; prevents self-indexing, OOM inputs, and accidental home-directory walks.

#### src/parse.rs
- Scope: tree-sitter-python parse; ERROR-node count per file; per-file parse_status (ok | partial | failed); build aborts when parse-error rate exceeds `--max-parse-error-rate <f>` (default 0.05) with exit 7 unless `--force`.
- Rationale: silent hollow graphs are the worst failure mode — degraded extraction must be visible and gatable.

#### src/extract.rs
- Scope: symbol extraction (function, class, method, module); edge extraction per resolution algorithm section; alias map construction; confidence tier assignment; stats counters.
- Rationale: core IP; normative algorithm lives in spec, code mirrors it.

#### src/db.rs
- Scope: rusqlite open/create with user_version check; meta manifest (kg_version via env!("CARGO_PKG_VERSION"), grammar_version via compile-time constant string `<lang>-<grammar-crate-version>` derived from the Cargo.lock-pinned grammar crate versions, e.g. `py-0.23.6`; source_rev via git probe with `-dirty` suffix on a dirty tree; counts); flock build.lock; streamed chunked inserts (fixed 500-row chunks, no full-corpus accumulation); atomic swap sequence; WAL cleanup.
- Rationale: full-atomic-replacement policy; skew handling.

#### src/query.rs
- Scope: callers/callees/impact with: qualname disambiguation (exit 2 + candidates), result-set-scoped staleness check, cap/limit, tier tags, empty-result reason lines.
- Rationale: CUJ2/CUJ3/CUJ4; response contract.

#### src/doctor.rs
- Scope: integrity_check, user_version + grammar_version report, parse-error rate, stale ratio, orphan `.kg/` files detection + cleanup, lock leftovers, DB size vs target.
- Rationale: 3 AM diagnosability; CUJ3.

#### src/hash.rs
- Scope: FNV-1a 64 over raw bytes; fast path skip only when mtime AND size match.
- Rationale: staleness contract frozen (schema-relevant semantics).

#### src/error.rs
- Scope: `enum KgError { CorruptIndex, Locked, SchemaMismatch, Usage, NoResults, ParseQuality, Internal }`; Display with `kg: <class>:` prefix; panic-free boundary.
- Rationale: exit-code taxonomy; CUJ3, CUJ7, CUJ8.

#### tests/ (corpus + integration)
- Scope: fixture Python trees (aliased imports, shadowed methods, star import, getattr call, variable reassignment, >1MB file, syntax-error file) with committed expected outputs so any rerun replays the gate; golden stdout/stderr snapshots (cap + stale + ambiguous cases); tree-snapshot read-only test; proc-table test; SIGKILL-mid-build recovery test (next build succeeds, user_version intact); schema-skew test (hand-bump user_version -> exit 5 from every query); two-process flock test (hold lock, second build exits 4 with pid message, CUJ6); no-index test (CUJ7); parse-abort test (CUJ8); seatbelt + seccomp network-deny scripts; dep-allowlist test.
- Rationale: gates G1-G8; reproducible verification by committed fixtures.

#### .github/workflows/ci.yml
- Scope: cargo build --locked; clippy -D warnings; test (macos-latest + ubuntu-latest); allowlist check; golden snapshots; sandbox jobs — Seatbelt network-deny (macOS) and seccomp network-deny (Linux) running `tests/scripts/{seatbelt,seccomp}-build.sh`.
- Rationale: CI-locked invariants.

#### docs/baselines/ (P0 output)
- Scope: grep-only transcripts, task definitions, ground-truth call sites, pain list, harness scripts.
- Rationale: G6 evidence chain — kill decisions must stay auditable against the stored baseline, not memory.

#### README.md
- Scope: Heilmeier answers in brief; install (cargo install --path .); one-line per subcommand; safety invariants; response contract.
- Rationale: fresh-implementer/owner reference.

### Verification Plan

- Prerequisite: `brew install hyperfine` (fallback: `tests/scripts/timing.sh` wrapping `/usr/bin/time -l`).
- Build: `cargo build --locked --release` — exit 0, binary < 15MB (target).
- Lint: `cargo clippy --all-targets -- -D warnings` — zero warnings.
- Tests: `cargo test` — all fixture + integration + snapshot tests green; gate scripts: `tests/scripts/seatbelt-build.sh`, `tests/scripts/seccomp-build.sh` exit 0; allowlist check exit 0.
- Gates: `hyperfine 'kg build'` (G1); `hyperfine 'kg callers <hot_fn>'` 30 runs hot+cold (G3); precision/recall script vs ground truth (G2); D0 harness A/B run (G6); doctor fault-injection (corruption, skew, orphans) — all kill-able per Gate Registry.
- Honesty rule: all numbers reported as measured with command attached; byte counts only, no token claims.
