# aw — Local Code Knowledge Index (Rust)

## Reading guide and current status

This is the canonical technical contract, maintained through numbered amendments rather than a rewritten base specification.
The original scope and rollout plan remain below for history; later explicit amendments supersede conflicting earlier clauses.

| Reader task | Start here |
|---|---|
| Use the tool | [Getting started](../getting-started.md) and [usage](../usage.md) |
| Change behavior safely | Amendments below, response contract, safety invariants, and [contribution rules](../../CONTRIBUTING.md) |
| Understand current query surfaces | Amendments 12-15 and 17-22; the companion designs explain their rationale |
| Understand storage compatibility | Schema/version policy, Amendments 19-22, and structural-graph companion §6.1 |
| Assess measured value | [Public evaluation summary](../EVALUATION.md), with evidence availability and snapshot limitations |

**Task and impact:** index local code to locate callers, callees, dependencies, and symbols without uploading source.
Static results guide inspection; they do not prove complete runtime reachability, test coverage, or deadness.

Current recorded storage is schema v7 (Amendment 20); Amendments 21-22 refine correctness and validation.

Project rename (2026-11-03): the package is `agent-weave`, the command is `aw`, and the
index directory is `.aw/`. Earlier clauses and historical evidence that say `kg` are
superseded names; the contracts are otherwise unchanged.
Amendment 16 was reverted. Dart support remains a separate proposal, not part of the implemented structural-graph scope.

**Evidence availability:** historical measurements and review outcomes below are internally reported.
Original internal evidence is unavailable in the public distribution; public evaluation summaries are not replacement original evidence.
Gate commands with `<internal-...>` placeholders preserve historical methods but require unavailable inputs and are not public reproduction instructions.

- Author: Quy Doan (spec compiled from consultation session 2026-10-30/31)
- Canonical location: this file, in the aw repo. A historical copy lives in
  the agent-skills repo (`docs/designs/`); the aw copy is authoritative —
  amendments land here first.
- Status: Approved (owner approved P0 proceed, 2026-10-31; supersedes "Draft (Gate 2 pending)")
- Date: 2026-10-31
- Concepts brief: `docs/designs/2026-10-31_concepts-brief_agent-weave-rust-code-index.md`

## Overview

`aw` is a single-binary local CLI that indexes a repository's symbols and call/inheritance edges into SQLite and answers three navigation questions for AI coding agents and the owner: who calls this, what does this call, what is impacted by this file. It is consultative only: it registers no hooks, injects no prompts, runs no daemon, and touches no network. Built in Rust with tree-sitter; Phase 1 targets Python (`vihap`, ~102k LOC), Phase 2 adds TypeScript/TSX/JSX (pilot: `~/Workspace/personal/finguard`, a ~12k-LOC TS/TSX monorepo, plus the React 19 web frontends `mskit-ui`/`qauto-ui`); C is out of scope (verified absent from the workload — earlier .c counts were vendored venv sources). Every design element is distilled from six analyzed third-party tools with recorded provenance; every gate is kill-able.

## The Problem

Agent sessions navigate code by grep/read loops. On large repos this costs many tool calls and input bytes; on proprietary code, cloud code-search services upload the source, while many local tools violate at least one stated trust requirement (command rewriting, credential interposition, self-updating sandboxes, or a bundled runtime with telemetry). Vendor-specific claims are intentionally out of scope for this specification. Current workaround: plain grep + AGENTS.md discipline. Measured baseline of that workaround is a Phase 0 deliverable, not yet recorded.

## Goals

- G-A: on 30 pre-verified call sites in vihap — precision (correct resolutions / all emitted resolutions) >= 80%, EXTRACTED-tier precision >= 95%, recall (true callers found / true callers in ground truth; an AMBIGUOUS result counts as a hit only when the true callee appears in its candidate list) >= 70%; Wilson 95% interval reported alongside every rate.
- G-B: vihap build < 5s wall, < 100MB RSS, < 50MB DB, query p95 < 20ms (targets until measured in P0/P1; all gates carry target labels until a measurement run exists).
- G-C: utility A/B shows aw >= 30% tool-call reduction on navigation tasks vs measured grep baseline, with no increase in total transcript bytes (both directions: tool-result bytes returned to the agent and agent tool-call input bytes — accounting defined in the D0 harness) and no correctness loss vs ground truth.
- G-D: four safety invariants hold and are CI-enforced (below).
- G-E: one static binary, five pinned dependencies, zero network syscalls, zero unsafe Rust in this crate (tree-sitter C runtime accepted as pinned dependency).

### Gate Registry (normative numbering)

| Gate | Goal | What | Pass command | Kill action |
|------|------|------|--------------|-------------|
| G1 | G-B | build performance on vihap | `hyperfine 'aw build'` (prereq: `brew install hyperfine`; fallback `tests/scripts/timing.sh` using `/usr/bin/time -l`) | retune or stop |
| G2 | G-A | precision + recall on 30 ground-truth sites | `python3 tests/scripts/precision_recall.py <repo> <aw-binary> <internal-ground-truth>` (original internal input unavailable publicly) | retune resolution algorithm or stop |
| G3 | G-B | query latency p95, hot + cold page cache | `hyperfine --warmup 3 'aw callers <hot_fn>'` x 30 runs | retune or stop |
| G4 | — | staleness detection + false-stale rate | edit-after-build integration test | fix |
| G5 | G-D/G-E | safety invariants 1-3 (CI) | seatbelt + seccomp + proc-table + tree-snapshot tests | fix before any ship |
| G6 | G-C | utility A/B vs P0 baseline | D0 harness: 5 tasks x 2 modes x 2 runs + correctness sampling | project stops; fallback Alternative B |
| G7 | P1.5 | hotspots rank overlap vs owner pain list | comparison script, >= 7/10 | feature dropped |
| G8 | P1.5 | report utility on architecture questions | A/B, >= 30% tool-call reduction | feature dropped |
| G11 | Amendment 10 | payload conformance: 1 payload line per successful pointer call, byte-equal pointers, verify FAIL=0 on fresh builds | `cargo test payload` + `aw usage --verify` on the 3 GT repos after fresh builds | fix journal wiring |
| G12 | Amendment 11 | perf-pass conformance: corrupt index exits 3 without quick_check; schema v3 carry-forward preserves loc/cyclomatic; hotspots reads the index (no re-parse); parallel build output identical to serial | `cargo test --locked` + G2 precision/recall on GT repos + timing battery (build wall, query best-of-7, hotspots) | revert to serial / re-add gate |
| G13 | Amendment 12 | outline utility: `aw symbols --outline` A/B on T1-T5, >= 30% tool-call reduction, no correctness loss | D0 harness (original internal evidence unavailable publicly): `count.py` + `check.py` over the `transcripts/kgoutlinearm/` arm | feature dropped; schema v4 ships without `symbols.signature` |
| G14 | Amendment 12 | symbol-search recall on 20 frozen NL queries (internal search ground truth, unavailable publicly) | `python3 tests/scripts/search_recall.py --queries <internal-search-ground-truth> <aw-binary> --aw-root <aw-repo> --vihap-root <vihap-worktree>` (recall >= 70%, FPR + Wilson 95% reported; internal input unavailable publicly) | feature dropped; schema v4 ships without `symbols_fts` |
| G15 | Amendment 12 | skill-contract drift (agent-skills repo) | `AW_BIN=<aw> python3 tests/skill_contract_drift.py` (agent-skills; skips when AW_BIN unset) | CI conformance check — fix skill text (not a product kill) |
| G16 | Amendment 12 | clone detection on 10 seeded pairs + build overhead | `cargo test clones_seed` (>= 8/10 pairs, <= 20% spurious) + interleaved before/after build timing (<= 25% wall; bar amended from the proposed 15% by owner decision 2026-11-01 after attribution showed the cost is clone-row INSERT I/O, linear in LOC — original internal G16 evidence unavailable publicly) | feature dropped; schema v4 ships without `clone_groups` |
| G17 | Amendment 13 | path correctness + latency: 10 seeded exact-byte fixtures; frozen GT pairs (internal path ground truth unavailable publicly, every hop edge verified EXTRACTED at freeze); latency budget | `cargo test path_seed` (10/10 exact-output assertions) + `python3 tests/scripts/path_recall.py --pairs <internal-path-ground-truth> <aw-binary> --aw-root <aw-repo> --vihap-root <vihap-worktree>` (internal input unavailable publicly; 100% agreement; mismatch routing: a missing/moved hop edge is resolution-attributable -> G2 retune, a wrong route or byte shape is a path kill) + `bash tests/scripts/timing.sh 7 path <binary> path <from> <to> --root <synth-corpus>` (budget < 20 ms; measured 0.6-1.4 ms in-process journal latency best-of-7 across the 2,025-file / 85k-symbol synth corpus and the 26-file self-index — original internal G17 evidence unavailable publicly) | feature dropped; schema v4 ships without `path` |
| G18 | Amendment 14 | search readability: G14 recall non-regression; vocabulary-gap recall beats the pre-change binary on >= 4/6 frozen path/signature queries; Stage 2b utility retest: search-routed arm cuts tool calls >= 20% vs grep+symbols control on BOTH hosts | `python3 tests/scripts/search_recall.py --queries <internal-search-ground-truth> <aw-binary> --aw-root <aw-repo> --vihap-root <vihap-worktree>` (mean >= 0.70) + `python3 tests/scripts/search_gap_recall.py --queries <internal-search-gap-ground-truth> --new-aw <dir> --new-vihap <dir> --old-aw <dir> --old-vihap <dir> <new-binary> <old-binary>` (new recall > old on >= 4/6; old/new indexes are built from identical source archives by the two binaries) + Stage 2b runner (internal runner unavailable publicly, 16 cells / 2 tasks, >= 20% call cut both hosts); the pre-change baseline binary is built from the pre-P6 commit (archive + `cargo build`); original internal inputs unavailable publicly | revert both changes; 12b shape restored; no schema consequence |
| G19 | Amendment 15 | R6 remediation conformance: deep-nesting no-abort; bounded clones; exact callees depth; fast-path gap healing; impl scoping; class+ctor edges; grammar identity; tracked-ignored indexing; bounded query materialization | `cargo test --locked` (105/105 incl. the 8 named regression tests) + internal schema-v5 performance re-measure (original evidence unavailable publicly; 2,025-file fresh 2.33 s / 183 MB; 1.05M-edge callers 0.14 s / 7.4 MB; 4,000-chain edit 0.26 s) | a failing regression test blocks release |

### Per-Phase Independent Review Protocol (normative)

Each phase ends with an independent expert review before the next phase starts:

- Reviewer: a stateless subagent with fresh context (Goldfish contract). Inputs: deliverable paths + that phase's acceptance criteria + the lens focus below. Never conversation history or author rationale.
- Verdict: `APPROVE` or `NEEDS FIXES` with per-finding severity (Critical/Important/Suggestion/Nit) and required fix. Gate: 0 open Critical + Important.
- Fix bound: <= 2 fix cycles per phase (rules/AGENTS.md §1 canonical bound); unresolved after that -> phase blocked, escalate to owner.
- Historical review records were retained internally with finding lists and dispositions; original records are unavailable publicly. Public review verdicts belong in the pull request per [CONTRIBUTING.md](../../CONTRIBUTING.md).

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

- OQ1 (owner, before P1 end): final repo home path (default `~/Workspace/aw`) and license choice (default: private, no LICENSE file until published).
- OQ2 (owner, at P1.5): skill wrapper `code-index` in agent-skills — requires updating the hardcoded count asserts in `tests/test_setup_script.sh` (27) and the usage-text "all 38" in `setup.sh`; the printed count itself adapts dynamically. Scheduled after G6 passes.
- OQ3 (resolved 2026-10-31): C preprocessor strategy — moot; probe verified zero project C code in the workload (earlier .c counts were vendored `.venv` sources). P2 targets TypeScript/TSX/JSX instead.
- OQ5 (resolved 2026-11-02): `aw path` direction semantics — owner approved forward-only (reverse question = swap the two arguments, no `--dir` flag; no demand signal for a flag). Proposed with the P5 amendment and pinned through the six-reviewer spec review.
- Amendment 1 (2026-10-31, P1 authoring): schema v1 gains `edges.site_line INTEGER NOT NULL DEFAULT 0` — the call-site line. G2 scores against 30 call-site ground-truth rows (`caller file:line`), which is only achievable if caller pointers carry the call-site line, not the enclosing symbol's start line. Additive-only policy honored (column added before first release).
- Amendment 2 (2026-10-31, G6): `aw callees` gains `--depth N` (default 1, clamp 1..10) — transitive call closure via recursive CTE. Load-bearing for G6: flow tasks collapse from 5 chained queries to 1. Output format unchanged.
- Amendment 3 (2026-10-31, R1): error-class taxonomy — the spec's CUJ7 message used class `no-index` while the spec's AwError enum has no NoIndex variant. Resolved: exit 1 uses class `no-results` for both no-index and empty results; detail strings distinguish ("no .aw/index.db under <root> — run aw build" vs "0 callers found (dynamic dispatch possible — grep fallback recommended)" / "0 callees found" / "0 impacting files found"). Star-import import edges carry the module symbol id with confidence AMBIGUOUS (schema comment updated) — calls-edge AMBIGUOUS rows keep dst_id NULL.
- Amendment 4 (2026-10-31, post-R2 review): fast-path correctness. (a) A carried file with ANY edge (calls/imports/inherits) into a changed file is re-extracted via a transitive dirty closure over the previous index — previously its copied edge died with the old destination id and `aw callers`/`aw impact` silently lost results. (b) Schema v2 (additive): `files.unresolved INTEGER NOT NULL DEFAULT 0` — per-file unresolved-callee count; `unresolved_callee_count` becomes an authoritative SQL sum, stable across incremental builds (it previously inflated monotonically). (c) `files.mtime` and staleness keys use nanosecond precision — same-second, same-size edits no longer defeat the fast path. (d) Orphan sidecar cleanup happens after the build flock is held (a second concurrent build could previously delete the in-progress `index.db.new`). (e) MRO resolution: two or more same-level matches emit an AMBIGUOUS edge as documented (previously the call site was silently dropped). (f) `aw hotspots` uses a language-aware cyclomatic proxy and churn/fix filters over all source extensions (previously Python-only in the multi-language build).
- Amendment 5 (2026-10-31, M1): query surface additions. `aw dead [--limit N]` — dead-code CANDIDATES: symbols with kind `function`/`method`/`class` that have no resolved inbound `calls` edge (`dst_id` = symbol id) AND no AMBIGUOUS `calls` edge whose `dst_name` equals the symbol's qualname or bare name; pointer contract applies (cap, `showing N of M`, staleness, exit 1 when none) and stdout always carries the first-line caveat `aw: dead-code candidates — dynamic dispatch, entry points, and external callers are not proven absent`; the output is a candidate list, NEVER proof of deadness. `aw unresolved [--limit N]` — report shape (no pointers): files with `files.unresolved > 0`, descending by count, lines `path unresolved=N`, header `aw: unresolved=T callees in M files`, capped; exit 1 with `0 unresolved callees (full call-site coverage)` when the index has no gaps; no staleness pass (counts are index-internal). `aw symbols <file> [--limit N]` — one indexed file's symbols as `path:line kind qualname` ordered by `line_start`; single-file staleness check produces the standard header; exit 1 `path '<file>' not indexed` for unknown paths.
- Amendment 6 (2026-10-31, J1): usage journal — real-world effectiveness observability. Every journaled subcommand (build, callers, callees, impact, dead, unresolved, symbols, stats, hotspots, report — NOT doctor, NOT usage itself) appends one JSONL line to `.aw/usage.log` after the process outcome is known: `{"ts":<unix-secs>,"surface":"cli","tool":"<name>","args":{...},"result_count":N,"shown":N,"total":N,"ambiguous":N,"exit_code":N,"latency_us":N,"stale":true|false,"aw_version":"..."}`. `args` carries only symbol names, paths, depth and limit scalars — NEVER file contents. Opt-out: env `AW_NO_JOURNAL=1` or global `--no-journal`. Rotation: when `usage.log` exceeds 5 MB it is renamed `usage.log.1` (one prior generation kept). Journal writes are best-effort: any failure to write is silently ignored and can never alter a command's output or exit code. Writes stay inside `.aw/` (invariant 3 unchanged); zero network unchanged. `aw usage` (read-only, no index required) summarizes the journal: calls per tool, error counts, p50/p95 latency, empty rate, stale rate, ambiguous pointer counts; exit 1 on an empty journal. The MCP surface (M3 amendment) writes the same schema with `surface:"mcp"`.
- Amendment 7 (2026-10-31, M2): call-resolution recall. (a) `crate::a::b::c` calls fall back to a direct qualname lookup under the crate root (`<crate-root>.a.b.c`) when intermediate segments are not indexed modules — previously a crate-root dir segment that is not itself a module (no `mod.rs` layout) made every deeper `crate::` call unresolved (the G2-self G01 gap). (b) Receiver typing (typed params, constructor assignments, self-fields) takes PRECEDENCE over module interpretation for the first chain segment of a 2-segment call — a local binding shadows a same-named module, matching Python scoping; previously an indexed module sharing the variable's name routed the call into the module tree and lost it.
- Amendment 8 (2026-10-31, M3): MCP stdio surface (P3 activated — trigger re-approved by owner 2026-10-31: MCP-native agents are the primary consumers). `aw mcp` runs a newline-delimited JSON-RPC 2.0 server on stdin/stdout, spawned by the agent host (aw itself still spawns no daemons — invariant 2 restated, not removed). Tools map 1:1 to CLI subcommands: build, callers, callees, impact, dead, unresolved, symbols, stats, hotspots, report, usage. `doctor` is NOT exposed: `--invalidate` is a destructive maintenance action reserved for the human CLI, and the read-only doctor report is covered by stats/usage. Tool results return the CLI's byte-identical stdout text; a non-zero outcome adds the stderr line and marks `isError:true`. Ambiguous-name results carry the candidate list in the result text (exit-2 semantics preserved end to end). Journal: every tool call appends the Amendment 6 schema with `surface:"mcp"` (J2); the server process itself is not journaled. Dependency policy change (additive, recorded): `serde_json = "1"` joins the pinned set — the MCP boundary parses UNTRUSTED client JSON, and a hand-rolled parser on a trust boundary is a security defect; serde_json is one audited dependency instead of an async framework. `rmcp` was rejected: it pulls the tokio runtime tree, contradicting the no-daemon / minimal-dependency build semantics. G10 gate: MCP-vs-CLI parity conformance is automated (same fixture, byte-identical result text, identical journal schema) and is the pass/fail condition; the agent-side A/B (tool-call and byte displacement) rides the harness G6/G8 re-run, which must now cover the MCP surface. Safety invariants 1, 3, 4 unchanged and CI-tested for the server binary (zero network, writes confined to `.aw/`, no behavior hijack).
- Amendment 9 (2026-10-31, M4): dual-key fast path — prebuilt index artifacts for CI. The fast-path carry key becomes (mtime, size) match OR (size match AND FNV-1a content_hash match): a restored `index.db` (CI cache) whose file mtimes all differ from the fresh checkout is still carried without re-parse when the content hashes agree. Hash-carried rows get their stored mtime updated to the fresh checkout's stat so the staleness contract stays truthful. The dirty closure, parse-quality gate, and version-gate (`probe_old`) semantics are unchanged. Cost model: the content hash is computed only when (mtime, size) does not already match — the normal workflow pays zero extra reads; the restore workflow pays one read+hash per file but still skips parse+extract entirely.
- Amendment 10 (2026-11-01, A3 — observability layer 3): result-payload journal, session attribution, replay verification. (a) Every journaled call that SUCCEEDS and returns pointers (callers, callees, impact, dead, symbols) appends one self-contained JSONL line to `.aw/usage-results.jsonl`: `{"ts":<unix-secs>,"surface":"cli"|"mcp","tool":"<name>","name":"<query target>","session":"<str>","arm":"<str>","built_at":<unix-secs>,"source_rev":"<str>","stale":true|false,"shown":N,"total":N,"pointers":["<rendered pointer text>", ...]}`. `pointers` is the rendered text of the DISPLAYED set (post-cap) — byte-identical to the query's stdout data lines; `built_at`/`source_rev` are read from the open index's meta manifest. Non-pointer tools (build, stats, usage, hotspots, report, unresolved) and empty/ambiguous/error outcomes write NO payload line — ambiguous candidates stay in usage.log counts and stdout. Rotation mirrors usage.log (5 MB -> `.1`, one prior generation); writes are best-effort under the same contract (a failed payload write can never alter output or exit codes); the opt-out switch (`AW_NO_JOURNAL=1` / `--no-journal`) silences both files. Payload lines are self-contained by design — replay verification needs no join with usage.log. (b) Session attribution: `journal::begin` reads env `AW_SESSION` and `AW_ARM` (trimmed, capped at 64 chars, JSON-escaped) and emits them as ADDITIVE fields `session` and `arm` in EVERY usage.log line and payload line (empty string when unset) — the R1/R2 A/B protocol can now attribute calls and results per task arm without transcript parsing. (c) `aw usage --verify [--limit N]` (default N=50): reads the payload log (both generations), samples deterministically (even stride, reproducible), and re-verifies sampled pointers against current source using the index: a file whose content differs from the index (`query::file_changed`) yields CHANGED — unverifiable, never counted as wrong; an unchanged file gets structural checks (line within current bounds; for `callers` additionally the line must contain the queried name's bare segment); an entry whose `built_at` predates the open index (a rebuild happened between query and verify) cannot prove a pointer wrong, so its structural failures degrade to CHANGED while structural passes still count OK — FAIL is reserved for the current epoch, where a failing pointer is proven wrong against the index it came from (e.g., corrupted payload text). Outcomes OK/CHANGED/FAIL; the report is informational (exit 0); exit 1 only when the payload log is empty or absent. The MCP `usage` tool accepts optional `verify`/`limit` args and shares the code path — per-invocation byte parity (G10) is preserved. G11 gate (payload conformance): every successful journaled pointer call has exactly one payload line; payload pointer text is byte-equal to stdout data lines; verify on a fresh build reports FAIL=0 (OK/CHANGED only); rotation and opt-out hold for both files.
- Amendment 11 (2026-11-01, performance ceiling pass — measured on a 2,025-file / 7.9 MB synthetic corpus, index 4.6 MB): four changes. (a) **Query-path integrity policy**: `open_for_query` no longer runs `PRAGMA quick_check` — it is a full B-tree scan, linear in DB size (measured 0.71-1.10 ms/MB, i.e. +35-55 ms at the 50 MB target `doctor` states), and it ran on EVERY query open on both the CLI and MCP surfaces (MCP opens per tool call). CUJ3 is preserved without it: `SQLITE_NOTADB` at open and `SQLITE_CORRUPT` at any statement map to exit 3 with the unchanged message `aw: corrupt-index: index.db failed integrity check — run aw build`; `aw doctor` remains the deep-integrity surface (`PRAGMA integrity_check`). A corrupt OLD index hit during carry-forward now also exits 3 (previously 6) — same rebuild guidance, more accurate class. (b) **Single-transaction build inserts**: the fresh DB is written under one `BEGIN`/`COMMIT` (carry-forward + files + symbols + edges + meta). The spec's "streamed chunked inserts (fixed 500-row chunks)" clause is superseded on COMMIT GRANULARITY only — 500-row prepared batches remain, no full-corpus accumulation is introduced. Measured on the identical row volume: insert phase 154 ms -> 55 ms (fsync per 500-row commit was the cost). A crash mid-build leaves `index.db.new` orphaned; existing cleanup semantics unchanged (SQLite rolls the uncommitted transaction back on close). (c) **Parallel parse+extract**: fresh files are read, parsed, error-counted, extracted, hashed, and stat'ed on `std::thread::scope` workers (worker count = `available_parallelism`, capped at 8; no new dependencies, no unsafe). Determinism: results merge in fresh-file order; `file_idx` alignment is preserved; tree-sitter trees stay thread-local. The stats line keeps its format; `parse=` is the phase wall time and `extract=` the summed per-worker extraction time (the two overlap on parallel builds). (d) **Schema v3 (additive)**: `files.loc INTEGER NOT NULL DEFAULT 0`, `files.cyclomatic INTEGER NOT NULL DEFAULT 0`, computed during the build parse phase with the exact formulas `hotspots` previously used per call (`bytes.split(b'\n').count()`; language-aware decision-node count over the same grammar). `aw hotspots`/`aw report` read the stored metrics instead of re-parsing every file per invocation (measured 409-781 ms -> ~100 ms bound; the two `git log` subprocesses remain). Behavior deltas, both intended: files deleted from disk are no longer silently skipped by hotspots (the index is the source of truth; staleness remains the `doctor`/`file_changed` surface), and hotspots scores are frozen at each file's last fresh extraction (a changed file is always re-extracted by the fast-path rules, so staleness of the metrics equals staleness of the file, already surfaced). G7 rank-overlap must be re-validated. G12 gate (perf-pass conformance): corrupt index exits 3 without quick_check; schema v3 carry-forward preserves loc/cyclomatic; hotspots output is byte-identical in shape; parallel build output identical to serial.
- Amendment 12 (2026-11-01, P4 agent-tooling — features distilled from the dmoreq/pi-sherlock v0.1.0 reference analysis; proposal spec-reviewed by an independent stateless Goldfish reviewer, internal review R4-P4 unavailable publicly, verdict APPROVE; evidence companion `docs/designs/2026-11-01_p4-agent-tooling_amendment.md`): four additive features behind ONE schema v4 bump. (a) **F1 `aw symbols --outline`**: `symbols.signature TEXT NOT NULL DEFAULT ''` — one-line signature captured at extraction from the already-parsed node (first line, trimmed, hard cap 160 chars, no bodies); default output byte-identical; OQ4a approved: the "No verbatim source responses" non-goal is amended to "no verbatim source *bodies*; signature lines capped at 160 chars are allowed"; journal `args` gains the boolean `outline` key. (b) **F2 `aw search <terms>`**: OQ4b approved — the `search` deferral is lifted for the symbol-search slice ONLY (content search remains rg's job); standalone FTS5 virtual table `symbols_fts` populated by one `INSERT ... SELECT` from `symbols` at build end (covers fresh and carried rows); indexed text is lowercased camelCase/snake_case-split `qualname + name`, preprocessed identically at query time; `bm25()` ranking; pointer lines with the tier field position EMPTY (search hits are symbol identities, not call edges); journal `args` gains the `terms` key; `search` joins the payload list. (c) **F3 skill hardening**: the existing `code-index` skill in agent-skills (commit d0c6136) is audited against all subcommands and gated by `tests/skill_contract_drift.py` (env `AW_BIN`, skips when unset — the agent-skills CI cannot build the private aw repo, so enforcement is the owner's local pre-commit check; limitation recorded); NOT a kill-able gate (G15). (d) **F4 `aw clones [--min-lines N]`** (default 5): `clone_groups(shingle_hash, file_id, line_start, line_end)` — FNV-1a shingles over normalized line windows computed at build for fresh files and carried forward with the fast path; mandatory first-stdout-line caveat "aw: clone candidates — normalized-text equality, not proof of semantic duplication"; journaled per Amendment 6 (`args` gains `min_lines`) but deliberately NOT in the Amendment 10 payload list — pair lines are not single pointers and `usage --verify` structural checks are undefined for pairs (recorded deviation). Gates: G13 (outline A/B, kill -> v4 ships without `symbols.signature`), G14 (search recall on frozen internal search ground truth, unavailable publicly, kill -> v4 ships without `symbols_fts`), G15 (drift conformance check), G16 (seeded clones + build overhead, kill -> v4 ships without `clone_groups`). All kills collapse before release; a post-release drop is a schema v5 rebuild under the version policy, never in-place removal. `aw doctor`/`aw usage` behavior unchanged.
- Amendment 13 (2026-11-02, P5 call-chain traversal — demand signal from RepoGraph (arXiv 2410.14684, reported) after the dmoreq/pi-scope analysis; proposal spec-reviewed by SIX independent stateless Goldfish reviewers (lenses: output contract, algorithm, spec-integration, safety-observability, evidence-gate design, adversarial agent-usability), 73 raw findings consolidated to 20, 2 fix cycles, round-2 disposition + amend-delta verification verdict APPROVE; internal review R5-P5 unavailable publicly, evidence companion `docs/designs/2026-11-01_p5-call-chain_amendment.md`; OQ5 approved by owner 2026-11-02). **F5 `aw path <from> <to> [--depth N]`** (default 10, clamp 1-10, same clamp as `callees`): shortest call path over the resolved call-edge set ONLY (`kind = 'calls' AND dst_id IS NOT NULL`); level-batched BFS — one indexed query per frontier level (500-id chunks per the build's batch precedent), frontier nodes in FIFO discovery order, each node's neighbors sorted by `(ds.qualname, df.path, e.site_line)`, multi-edges deduped to the smallest `site_line`, parent = first discoverer; deterministic across runs and across parallel/serial builds given an identical index. Endpoints resolve via `target_ids` (exact bare name or full printed qualname; partial qualnames and pasted pointers reject with `symbol '<name>' not in index`), `from` then `to`; ambiguity exits 2 with the failing endpoint's role in the detail (`... as the from|to endpoint`); resolving both then comparing: equal ids exit 1. Output contract (contract-bump entry): stdout = first-line caveat `aw: path — resolved static edges only; dynamic dispatch may hide paths` (staleness header appended after a newline exactly as `dead` renders caveat + staleness), hop call-site pointers in path order (bare `repo-relative/path:line`, always the caller's file, never ` [ambiguous]`, NO cap — recorded deviation: `path` has no limit surface, so no `--limit` argument, no `showing N of M` line, and journal `args` carry `name`/`to`/`depth` unconditionally without a `limit` key), final stdout line `aw: chain <resolved full qualnames> (K hops)`. Exit-1 reasons (class `no-results`, `<q1>`/`<q2>` rendered as resolved full qualnames, `D` as the clamped depth): `'<q1>' and '<q2>' resolve to the same symbol`; `no call path from '<q1>' to '<q2>' at any depth <= D (resolved edges only)` (frontier exhausted: definitive negative); `no call path from '<q1>' to '<q2>' within depth D; a longer path may exist (resolved edges only)` (cap-truncated: the depth-frontier was probed and an unvisited out-neighbor exists). `path` joins the Amendment 10 payload list — `name` = the `from` argument, `to` travels in usage.log `args` only, `shown` = `total` = K, hop pointers byte-identical to stdout data lines; `usage --verify` needs NO verifier change (the bounds-only default branch already covers non-`callers` tools). MCP tool `path`: inputSchema `from`/`to` strings + `depth` integer min 1 max 10 default 10 (MCP default = CLI default), required `[from, to]`, G10 byte parity. Schema: NO change — v4 stays v4; a kill ships without `path` and withdraws the MCP tool, journal `to` key, payload entry, contract-bump lines, and skill paragraph (no data migration). Gate G17 (original internal evidence unavailable publicly): 10 exact-byte fixtures, frozen GT pairs with 100% agreement and resolution-vs-BFS mismatch routing, latency < 20 ms (measured 0.6-1.4 ms in-process best-of-7 across the 85k-symbol synth corpus and the self-index). Companion fix pre-step (own commits, pre-existing defects surfaced by the phase): `tests/scripts/write_path_check.sh` stale exclusion repaired (G5 was red on the committed tree), and `probe_old` requires the complete v4 table set — a v4-stamped index from the Amendment 12 first landing wave (clone_groups absent) disables the fast path instead of failing carry-forward.

- Amendment 14 (2026-11-02, P6 search utility upgrade — root-cause fix from the Stage 2 agent-utility A/B (original internal evidence unavailable publicly): search tied with grep because the FTS haystack was qualname+name only while grep matches path, signature, and comments, and a search hit line carried no more information than a grep line). Two changes, NO schema bump: (a) **indexed text widened** — `symbols_fts` text becomes `split_ident(qualname) + split_ident(name) + split_ident(files.path) + split_ident(symbols.signature)`; path and signature tokens are derived data (v4 unchanged), the FTS is repopulated from symbols at build end so carried rows pick the new text up on the next build, and a query against a not-yet-rebuilt index returns pre-change quality until then (staleness contract unchanged). (b) **hit line gains the stored signature** — search pointers render `path:line qualname signature` (empty signature -> byte-identical to the 12b shape); contract-bump entry; payload pointers stay byte-identical to stdout data lines (verify rule unchanged: bounds-only branch); MCP G10 parity unchanged. Gate G18 (kill = revert both changes; no schema consequence): (i) G14 recall non-regression on the frozen 20 queries (mean >= 0.70; baseline 0.850 pre-P6 — re-measured 0.800 on the rebuilt P6 index, see Amendment 15); (ii) vocabulary-gap recall on a frozen 6-query set whose target words live in path/signature tokens only — the shipped binary must beat the pre-change binary (`/tmp/aw-presearch-2026-11-02`) on >= 4/6 queries; (iii) Stage 2b utility retest on vihap: the search-routed arm must cut tool calls >= 20% vs the grep+symbols control on BOTH hosts, correctness parity required. If (iii) fails while (i)-(ii) pass, revert remains automatic but the owner may re-decide to keep the recall improvement as a separate amendment decision — recorded, not silently taken. **Owner re-decision (2026-11-02): KEEP both changes.** G18(i) passed (G14 recall unchanged 0.850), G18(ii) passed (gap set 6/6 improved), G18(iii) failed on OpenCode only (−17% vs the 20% bar; Grok −38% calls / −28% tokens) — the utility gain is recorded as host-dependent, the recall and readability improvements ship.

- Amendment 15 (2026-11-02, R6 remediation pass — findings R6a-R6g, seven-lens Goldfish review): correctness and cost fixes behind ONE schema bump. (a) **Schema v5 (additive)**: `unresolved_calls(file_id, dst_name)` stores every call site that failed to resolve with its written callee name, carried forward with the fast path; when fresh extraction introduces symbol names absent from the old index, carried files whose unresolved or AMBIGUOUS names match those symbols are re-extracted — bounded by gap rows, not corpus size (the fast path now heals resolution gaps; this supersedes Amendment 4a's resolved-edge-only closure). (b) Build safety: parse/extract workers run on 64 MB stacks and the AST walk guards at `MAX_AST_DEPTH = 8192` (over-deep subtrees are skipped, never a process abort); error-node and decision-node counters are iterative. (c) `clones` pair generation is bounded: <= 64 merged regions per shingle group and <= 20,000 total pairs, with a stderr lower-bound note; a shared 5-line header no longer materializes millions of pairs. (d) `callees --depth N` closes exactly N levels (the CTE's final SELECT requires `r.d < depth`). (e) Rust `impl` methods scope to the grammar's `type` field (parameter/return types no longer leak into qualnames). (f) Constructor calls emit edges to BOTH the class and its unique `__init__`/`constructor`; `callers Class` and `dead` are correct. (g) MCP: 8 MiB frame cap with a `-32600` reply, per-call panic isolation, `--no-journal` honored on the MCP surface, trailing-newline parity for `stats`/`build`. (h) CLI safety: grammar identity is derived from `Cargo.lock` by `build.rs`; `GIT_OPTIONAL_LOCKS=0` removes `.git/index` writes; broken-pipe exits 0; tracked-but-gitignored files are indexed; search tie-break is deterministic across fresh/carried builds. (i) Bounded cost: pointer queries use SQL `LIMIT` plus a separate `COUNT(*)`; the dirty closure is a work-queue (4,000-file chain edit 42.5 s -> 0.26 s); search terms cap at 32; ambiguous-name rendering caps at 50 candidates while the error carries the true total; at 1.05M edges `callers` measures 0.14 s / 7.4 MB and at 1M symbols `search` < 10 ms / 4.2 MB (original internal schema-v5 performance evidence unavailable publicly). (j) G18(i) premise correction: G14 recall on the rebuilt P6 index is 0.800 (23/28), not 0.850; the gate bar (>= 0.70) still passes and the owner keep-P6 decision stands (gap recall 6/6). If (iii) fails while (i)-(ii) pass, revert remains automatic but the owner may re-decide to keep the recall improvement as a separate amendment decision — recorded, not silently taken. **Owner re-decision (2026-11-02): KEEP both changes.** G18(i) passed (G14 recall unchanged 0.850), G18(ii) passed (gap set 6/6 improved), G18(iii) failed on OpenCode only (−17% vs the 20% bar; Grok −38% calls / −28% tokens) — the utility gain is recorded as host-dependent, the recall and readability improvements ship. Post-review closures (2026-11-02, P8 pass): PEP 420 namespace resolution implemented; empty-result caveat now printed for `dead`/`clones`; MCP protocol version negotiation echoes the client; serial-vs-parallel build determinism is tested via `AW_WORKERS`; GitHub Actions are SHA-pinned. Still deferred: G15 skill-drift script lives in the companion-skills repository (not in this tree); exit-6 panic-path contract test. Gate G19 (R6 remediation conformance; kill = failing regression test blocks release, no schema consequence).

- Amendment 16 (attempted 2026-11-02, REVERTED): docstring-token search (schema v6 `symbols.docstring`, FTS widening). G20(i) passed (G14 0.800) and G20(ii) passed (prose-gap 5/6), but G20(iii) failed (Stage 2c: tool calls +12% on Grok, −9% on OpenCode; aw calls up). The kill action was applied: the feature is reverted to schema v5; Stage 2c evidence and prose-search ground truth retained internally (original evidence unavailable publicly). This records the negative result without shipping unproven complexity.

- Amendment 17 (2026-11-02, P8 one-call symbol context — owner-directed redesign after the Stage 2c negative): `aw context <symbol> [--limit N]` (default 10, max 50) returns in one call: the definition line (`path:line kind qualname signature`), direct callers (call-site pointers), and direct callees (callee definition pointers), each section headed `callers (showing N of M):` / `callees (showing N of M):`. First stdout line is the caveat `aw: context — resolved static edges only; dynamic dispatch may hide callers or callees`; a staleness header follows it for the displayed set. Only resolved `calls` edges are used (AMBIGUOUS/NULL-dst rows are excluded from both sections). Exit codes unchanged (1 not-in-index, 2 ambiguous with candidates, 0 otherwise). Journal (Amendment 6): `args` gains `name` + `limit`. Payload (Amendment 10): pointers = the displayed caller+callee pointer lines, byte-identical to the stdout data lines; `name` = the symbol argument; `shown` = `total` = pointer count; the bounds-only verify branch applies. MCP tool `context` (`name` string, `limit` integer 1-50 default 10) with G10 byte parity. Contract-bump entry (new subcommand + new stdout shape). Gate G21: exact-byte fixtures (definition+callers+callees, empty section, cap, missing, ambiguous), MCP parity, and Stage 5 utility A/B (24 cells, three context questions): the context-routed arm must cut tool calls >= 20% vs composing `callers`+`callees`+`symbols` on BOTH hosts with correctness parity. Kill = remove `context` (no schema impact). **Owner re-decision (2026-11-02): KEEP `aw context`.** G21(iii) measured Grok tool calls −19% (1.25 points under the 20% bar) with aw calls −84%, and OpenCode tool calls −46%; the result is recorded as host-dependent (original internal Stage 5 evidence unavailable publicly) and the feature ships.
- Amendment 18 (2026-11-02, R7 ceiling-review hardening: accuracy, utility, and latency fixes from four Goldfish lenses — path/context, call-graph queries, retrieval, analysis/infrastructure; schema stays v5, no new dependencies). (a) **Signatures (supersedes Amendment 12a's "first line")**: capture the full parameter list across line wraps, stop before the body delimiter (`{`, `;`, or Python `:`), collapse whitespace, cap 160 chars. 16.5% of Python signatures previously ended at `(`; `aw search <unique_parameter>` returns the symbol now. (b) **Lexical scopes**: a Python nested `def` belongs to its enclosing function (innermost scope wins; methods still belong to their class); Rust inline `mod name { ... }` and TypeScript `namespace N { ... }` emit module symbols and qualify members (`pkg.mod.alpha.shared` vs `pkg.mod.beta.shared`); module symbols never create a false ambiguity when a callable shares the bare name. (c) **Receiver/import resolution**: `super().m()` / `super(C, self).m()` and `Class().m()` resolve to the base/class method; base classes reached through re-export chains resolve; PEP 420 namespace imports keep the longest-prefix fallback. (d) **Call-graph query shapes**: `callers`/`callees`/`context` lines carry the call-site's identity after the pointer (`path:line qualname`), context callees dedupe by resolved destination (identity for unresolved names, `[ambiguous]` tag), the definition carries `line-end` and signature, and the journal payload carries the same identity strings byte-for-byte; `impact` defaults to one row per caller file with `(N sites)` and `--sites` restores the per-site list; `dead` suppresses a symbol only when an edge could refer to it per-symbol (`dst_name = qualname` or same-file bare name) through ONE materialized referenced-id set (no per-symbol edge scan) and gains `--exclude-tests`. (e) **Clones**: SQL prefilter to hashes with >= 2 regions, merge overlapping windows per hash and per file pair BEFORE `--min-lines`, sort merged spans longest-first with a deterministic tie-break, cap 64 regions/group and 20,000 pairs with a lower-bound note; pair lines name the largest-overlap enclosing symbol per side (`path:start-end (qualname)`). (f) **Search**: qualified input (`.`, `/`, `::`) matches an adjacent-token phrase first (<1 ms on measured corpora) and falls back to OR; a single token that is an exact symbol name or qualname returns those identities; acronym boundaries split (`HTTPServer` -> http server); a zero-hit query retries with bounded English stems (`handlers` -> handler, `rotation` -> rotate). (g) **Unresolved**: output adds distinct-name count, an `internal=` count (names that exist in the index — actionable resolver gaps), and the top-10 unresolved names; external/library calls remain unflagged by this conservative exact-name test. (h) **Analysis**: hotspots counts Python `elif`/`case` clauses and TS ternaries, and LOC excludes blank and comment-only lines; `report` prints a first-line freshness caveat when sampled indexed files changed; `stats` reads build-time manifest counters (O(1), fallback to COUNT). (i) **Build latency**: a build with zero fresh/deleted files and no mtime refresh skips the database write entirely (2,201 files: 0.02-0.03 s vs 0.09-0.11 s; index.db byte-unchanged). (j) **MCP**: a missing required tool argument is a JSON-RPC `-32602` error instead of a downstream empty-name tool result. Contract-bump entry: signature content, caller/callee identity suffix, clone span suffix, unresolved output shape, and report freshness header. Measured trade-off: full signatures change bm25 ranking, so G14 recall is 0.775 (22/28, bar 0.70) vs 0.800 before this batch, while the signature-only query class moves from no hit in the top 50 to rank 2; the bar still passes. Gate G22 (R7 hardening conformance; kill = revert the specific fix whose gate regresses, no schema consequence).
- Amendment 19 (2026-11-02, P8 structural graph — owner instructed implementation; companion `docs/designs/2026-11-02_p8-structural-graph_amendment.md`, internal review R8-P8 unavailable publicly): schema v6 additive (`symbols.cyclomatic`, `branches`, `nesting`, `effective_loc`, `exported`; `files.generated`; `unresolved_calls.kind`, `site_line`; `symbols_complexity_idx`) and F1-F11 surfaces: `implementers`; `impact --edges/--depth/--context/--explain`; `dead` inbound imports/inherits + trait/interface-parent exclusion; `symbols <dir>`; `--path` scopes; `files`; `imports`; `tests`; `resolve`/search filters; `unresolved --names`; `complexity`; `api`; generated tagging; skill updates. P9 Dart is a separate addendum. Gate G23-G28. Contract-bump entry. Open review items O1-O10 are recorded in R8-P8; implementation follows spec rev 4 defaults. A killed feature withdraws its command/flags, MCP tool, journal/payload row, skill line, and docs; v6 releases only after P8a-P8c gates pass.
- Amendment 22 (2026-11-03, R12 fixes, schema v7): (a) the fast path carries
  `trait`/`interface` symbols into resolution; (b) Rust `scoped_type_identifier`
  impl targets become dotted chains, and `crate::Trait` maps through the crate
  root; (c) root-level TS `index.*` resolves to the root module, Python
  `from .. import X` from a first-level package binds root children, and direct
  children of a Rust inline `mod` are bare-name candidates even when stored as
  `method`; (d) Rust trait-impl methods are tracked per impl block so inherent
  non-`pub` methods are not over-exported, while `pub fn` in a module parent is
  exported; (e) `probe_old` requires `meta.callable_count`, so a pre-manifest
  v7 index rebuilds instead of returning an empty default `complexity` result.
  No schema or output-shape change beyond the corrected rows.
- Amendment 23 (2026-11-03, Rust resolution and query-edge contracts): (a)
  Rust receiver calls: `field_expression` receivers become call sites, and
  `self.m()`, typed parameters (`x: T`, `&T`, `&mut T`), and constructor-typed
  locals (`let x: T`, `let x = T::new()`, `let x = T { .. }`) type the receiver;
  `Self::assoc()`, `self::f()`, and `super::f()` resolve through the enclosing
  impl target or module tree; free functions inside inline `mod` keep kind
  `function` (supersedes Amendment 22(c)'s "even when stored as method").
  Unknown receivers fall back only to a unique same-file method name; common
  stdlib/trait names and multi-match names stay unresolved instead of emitting
  AMBIGUOUS fan-out. (b) `target_ids` adds a `Class.method` suffix fallback
  when the exact name/qualname lookup is empty (supersedes the "full printed
  qualname only" clause of Amendment 13 for this form); several suffix matches
  still exit 2 with the candidate list, and `path` endpoint roles are
  unchanged. (c) `dead` rows render `path:line qualname`; pointer rendering
  keeps the tag's leading space, so an ambiguous callee renders
  `path:line [ambiguous] detail`; `callees` falls back to
  `COALESCE(ds.qualname, e.dst_name)`, so unresolved/ambiguous rows name the
  recorded target instead of an empty detail. (d) single-token `search`
  returns exact name/qualname hits first, then appends deduplicated FTS
  prefix/compound matches up to `--limit` instead of discarding them. (e)
  `symbols .` and `symbols ./` scope the repository root without generating a
  `/` prefix. (f) path-scoped SQL uses bound parameters (`?N`) instead of
  string-escaped literals, and `complexity --path` omits
  `INDEXED BY symbols_complexity_idx` so SQLite can use `symbols_file_path`.
  (g) journal readers parse JSONL with `serde_json` (quoted/escaped arguments
  and pointers survive); `stats`, `hotspots`, and `report` journal result
  counts, and `resolve` records real latency on its early exit; `doctor`
  reports a build lock only when another process still holds it. No schema
  change, no new dependency (serde_json is already pinned).
- Amendment 21 (2026-11-03, R11 fixes and budget amendment, schema v7): (a)
  Python `import a.b` keeps every ancestor module edge; the redundant-parent
  suppression applies only to `from pkg import ...`. Rust `main.rs`/repo-root
  `lib.rs` are crate roots, `use crate::...` maps through them, and Rust
  wildcard use trees emit the pinned AMBIGUOUS module edge. A module import
  falls back to a same-qualname symbol edge when the target is a function,
  type, or trait. (b) Cross-file Rust `impl Trait for Type` and TS
  `extends mixin(<non-identifier>)` store unresolved `kind='inherit'` rows.
  (c) `search --exact` bypasses FTS entirely, never journals a fuzzy result,
  and handles tokenizer-invisible names; `--path` rejects embedded `..`
  components and `api` rejects root/empty paths; per-site empty reasons keep
  the unresolved-edge suffix; `dead --exclude-tests` matches nested
  `test_*.py`; `unresolved --names` prints the standard `K of N` staleness
  header and stderr-counts `site_line=0` rows. (d) `effective_loc` uses
  tree-sitter comment-node coverage plus the Python docstring interval; TS
  `protected` is not exported; Rust trait and trait-impl methods of an
  exported type are exported. (e) P8 gate commands select committed tests:
  `p8a_*`, `p8c_*`, `import_*`, and `tests_impact_seed_*`; GT harnesses
  report Wilson precision as well as recall. (f) The 1M-symbol `complexity`
  default budget stays 50 ms (measured 6.9 ms with a `callable_count` meta
  manifest); the path-scoped budget is amended from 100 ms to **250 ms**
  (measured 201 ms on the 987k-symbol corpus; keeping the old 100 ms bar
  requires extra indexes that pushed the 2,025-file build from 2.7 s past
  the 2.91 s budget).
- Amendment 20 (2026-11-02, P8 optimization): schema v7 additive — `symbols.file_path TEXT NOT NULL DEFAULT ''` denormalizes `files.path` so `aw complexity` can order by `(cyclomatic, effective_loc, qualname, path, line_start)` with `symbols_complexity_idx` covering the full key. The column is populated on fresh inserts and carried forward; `probe_old` requires it; v6 query opens exit 5 and rebuild. No other surface changes. Gate G26 gains the v7 partial-index/fresh-vs-carried order assertion.




## Critical User Journeys

### CUJ 1: Build (happy path)
- Given: vihap repo with `.git`, no `.aw/` directory.
- When: `aw build` in any subdirectory of vihap.
- Then: exit 0; stats line on stdout (files, symbols, edges, parse-error count, unresolved-callee count, per-phase timings, CPU-seconds); `.aw/index.db` + manifest present; `git status --porcelain` shows nothing outside `.aw/`.

### CUJ 2: Navigate (agent question)
- Given: index built; 200+ callers for a hot symbol; one result file edited after build.
- When: `aw callers hot_fn`.
- Then: stdout pointer list `path:line [tier]` capped at 50 by default, final stdout line `showing 50 of 213`, stderr `[aw] capped at 50 results (213 total)`, stdout header line `aw: 3 of 12 result files changed since build — run aw build`; tier tags on every line; exit 0.

### CUJ 3: Failure (corrupted index)
- Given: `index.db` corrupted (truncated file).
- When: `aw callers foo`.
- Then: exit 3; stderr `aw: corrupt-index: index.db failed integrity check — run aw build`; no panic; no partial output.

### CUJ 4: Ambiguous name
- Given: `process` exists as method in 3 classes.
- When: `aw callers process`.
- Then: exit 2; stdout candidate list `path:line qualname` x3; message names the `Class.method` form; no merged-silent result.

### CUJ 5: Version skew
- Given: index built by aw v0.1 (schema v1); binary upgraded to schema v2.
- When: `aw callers foo`.
- Then: exit 5; `aw: schema-mismatch: index schema v1, binary expects v2 — run aw build`. Old binary reading v2 DB: exit 5 with mirror message.

### CUJ 6: Concurrent builds
- Given: two agent sessions run `aw build` in the same repo.
- When: second build starts while first holds `.aw/build.lock`.
- Then: second exits 4 with `aw: locked: build in progress (pid N) — retry after it exits`; no interleaved writes.

### CUJ 7: Query without index
- Given: repo never indexed; no `.aw/` directory.
- When: `aw callers foo`.
- Then: exit 1; `aw: no-index: no .aw/index.db under <root> — run aw build`; nothing else on stdout.

### CUJ 8: Parse-quality abort
- Given: repo where 12% of .py files fail tree-sitter parse; threshold default 5%.
- When: `aw build`.
- Then: exit 7; `aw: parse-quality: 12% of M files failed to parse (threshold 5%) — inspect with aw doctor or override with --force`; `--max-parse-error-rate <f>` adjusts the threshold; `--force` proceeds and records per-file `parse_status`.

## Technical Plan

### Component layout (single crate, no workspace)

```
src/main.rs     entry, exit-code mapping
src/cli.rs      clap definitions, help texts
src/walk.rs     ignore-crate walk; excludes .aw/ unconditionally; per-file size cap 1MB (skipped + counted)
src/parse.rs    tree-sitter-python; ERROR-node counting per file
src/extract.rs  symbols/edges; resolution algorithm; confidence tiers
src/db.rs       rusqlite; schema; user_version; manifest; flock; atomic swap
src/query.rs    callers/callees/impact; staleness check (result-set scope); caps; tier tags
src/doctor.rs   integrity, versions, parse coverage, orphans, lock state
src/hash.rs     FNV-1a 64 (raw bytes, no normalization)
src/error.rs    AwError enum, stable stderr prefix `aw: <class>: <detail>`
schema.sql      embedded via include_str! (tables + FTS)
schema_indexes.sql  embedded via include_str! (secondary indexes; created after bulk insert)
```

Dependencies (pinned, complete): `clap` (derive), `tree-sitter` + `tree-sitter-python` (P1) + `tree-sitter-typescript`/`tree-sitter-javascript` (P2; tsx grammar ships in the typescript crate) + `tree-sitter-rust` (P2.5, optional), `rusqlite` (bundled), `ignore`. No rayon (single-threaded P1; add only if G-B fails, then <= 4 workers). No serde. `#![forbid(unsafe_code)]` at crate root.

### Schema (v1, additive-only evolution policy)

```sql
PRAGMA user_version = 1;                      -- schema contract; normative policy below
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
-- meta rows: aw_version, grammar_version, built_at, source_rev, file_count, symbol_count, edge_count, parse_error_count, unresolved_callee_count
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

`aw build` always produces a **full atomic replacement of the DB file**: extract -> write to `.aw/index.db.new` (fresh schema, `PRAGMA user_version` stamped) -> checkpoint + close (WAL removed) -> `rename(2)` `index.db.new` onto `index.db` — atomic, so a concurrent reader sees the old or the new DB, never a missing one; no delete step exists, and orphan sidecars are cleaned at build start and by `doctor`. Per-file extraction fast path: a file whose (mtime, size) match AND whose stored rows were extracted under the same schema and grammar versions is carried forward without re-parse, UNLESS it is in the transitive dirty closure over edges into changed files (Amendment 4a — a carried file with any edge into a changed file is re-extracted, or its copied edge dies with the old destination id); any schema/grammar version mismatch disables the fast path for the whole build. `aw build --force` disables the fast path unconditionally (re-extract everything). `.aw/build.lock` (`flock`, holds pid + aw version) is taken for the whole build; second build exits 4 (CUJ6). Rollback = reinstall old binary + `aw build` (< 5s target). One-command exit to grep-only: `rm -rf .aw/`. `aw doctor --invalidate` deletes the index.

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
- Cap: default 50; `--limit N` — N < 1 rejected (exit 2), N > 10,000 clamped to 10,000. When capped: final stdout line `showing N of M`; stderr `[aw] capped at N results (M total)`. Contract byte-stable; changes require contract-bump release note + wrapper update in same release.
- Staleness: checked on the **post-cap displayed set only** (cost bounded by the displayed count): per file, `stat(mtime, size)` short-circuits; a mismatch triggers FNV-1a re-hash. Header on stdout `aw: K of N result files changed since build — run aw build`. Never auto-rebuild.
- Empty result: exit 1 with a reason line distinguishing "symbol not in index" / "path not indexed" / "0 callers found (dynamic dispatch possible — grep fallback recommended)".
- No index at all: exit 1, `aw: no-index: no .aw/index.db under <root> — run aw build` (CUJ7).
- Exit codes: 0 success; 1 no results / no index; 2 usage or ambiguous name; 3 corrupt index; 4 locked; 5 schema mismatch; 6 internal; 7 parse-quality abort (`aw: parse-quality: N% of M files failed to parse (threshold T%) — inspect with aw doctor or override with --force`, CUJ8).

### Safety invariants and test mechanisms

1. **Zero network**: no network syscalls. Tests: dependency allowlist check (no crate may link a resolver/TLS stack — CI greps `Cargo.lock` against allowlist); macOS CI job runs `aw build` under a Seatbelt profile denying `network*`; Linux job under seccomp-deny profile.
2. **No long-lived process**: test asserts zero child processes remain after each subcommand (proc table scan).
3. **Read-only over source**: integration test snapshots repo tree+hashes before/after `build`/`query`; diff must be empty outside `.aw/`.
4. **No behavior hijack**: review-checked in spec + CI grep: no writes outside `.aw/`, no settings/hooks/config files touched (constant list of allowed write paths).

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

### Alternative C: universal-ctags + ast-grep only (no aw)
- Pros: zero build, zero maintenance, local, permissive licenses; covers symbol listing and structural search.
- Cons: no call-graph edges, no impact analysis — the three questions aw exists for.
- Reason for rejection as sole solution; adopted as P0 baseline tooling and complement.

### Alternative D: repowise
- Pros: community-endorsed (r/ClaudeCode review), wiki + health scores.
- Cons: cloud service — proprietary code leaves the machine; closed-source engine (public repos are an unrelated RAG chatbot, no-license).
- Reason for rejection: control requirement; concept re-implemented locally as P1.5 `hotspots`/`report`.

## Non-Functional Requirements

- Security and privacy: four invariants above; no secrets handling at all (no keys exist); `.aw/` gitignored by tool on first build (tool writes `.aw/.gitignore` containing `*` — inside its own directory, allowed by invariant 3's constant list).
- Performance and latency (targets until measured; P0 records grep baseline, P1 records aw numbers): build vihap < 5s wall / < 100MB RSS (streamed chunked inserts, no full-corpus accumulation) / < 50MB DB; query p95 < 20ms measured hot and cold page cache; concurrency: p95 under 4 parallel query processes + 1 build in flight; scaling: one synthetic 1M-LOC corpus run records wall/RSS/DB and stated scaling behavior.
- Rollout and rollback: per-repo adoption (vihap first); rollback = `rm -rf .aw/` (documented one-command exit); version skew handled by exit 5 + rebuild; read window: the binary opens only its own compiled schema version N — older and newer both exit 5 (single normative policy, matches CUJ5).

## Detailed Implementation Plan

### Phases

- **P0 — Baseline (no Rust, ~1 day)**: freeze 30 call sites + top-10 pain list (owner, 30 min); record grep-only transcripts for 5 standardized tasks; retain baselines + task definitions in the internal evidence archive (unavailable publicly). Deliverable D0: A/B harness (task scripts + byte/call counters + correctness checker). Gates G-A..G-E numbers finalized against reality. **P0 exit criterion (cheapest kill)**: if the grep baseline answers every navigation task in <= 2 tool calls, the premise is weak — stop before P1 and keep universal-ctags/ast-grep; if the projected reduction is clearly below 15%, present findings to the owner before spending P1.
- **P1 — Core (Rust, ~6-10 days)**: files in ledger below; gates G1-G5 (Gate Registry); then G6 utility A/B (D0 harness, correctness sampling included). Kill rule: G6 fail after one fix cycle -> project stops, fallback Alternative B.
- **P1.5 (only after G-C pass)**: `report` (4 sections), `hotspots` (score formula: cyclomatic + LOC + churn-90d + fix-density; validated by G7 rank-overlap >= 7/10 vs owner pain list), `stats`; golden response-contract freeze; skill wrapper in agent-skills (OQ2); A/B transcripts committed (G8: >= 30% tool-call reduction on architecture questions).
- **P2 — TypeScript/TSX/JSX on finguard (~3-5 days)**: add `tree-sitter-typescript` (+ tsx) and `tree-sitter-javascript` (.jsx) grammars in one commit (runtime+grammars rule, golden replay); extract TS symbols (function, class, method, component, hook), imports/exports edges, receiver-class resolution for `this.`/typed calls; index the pnpm workspace whole (node_modules excluded by the `ignore` crate); freeze 20 ground-truth call sites in finguard -> G2 rerun (>= 80% / >= 70%); Python regression fixtures stay green. finguard is read-only; writes only under its `.aw/`.
- **P2.5 (optional) — Rust self-index**: add `tree-sitter-rust`; aw indexes its own repository (dogfooding); G2 rerun on aw's own fixtures.
- **P3 (optional)**: stdio MCP server wrapping the same query functions; 90s hard-exit watchdog; only if CLI discoverability measurably fails.

### File Ledger (repo `~/Workspace/aw`, all Create)

#### Cargo.toml
- Scope: manifest; five pinned deps; release profile (lto=thin, strip).
- Rationale: complete dependency disclosure for the allowlist test.

#### Cargo.lock
- Scope: committed; CI allowlist test reads it.
- Rationale: reproducible builds; supply-chain audit surface.

#### schema.sql
- Scope: embedded table + FTS DDL above; `schema_indexes.sql` holds the secondary
  indexes, created after the bulk insert; `user_version` is stamped from
  `SCHEMA_VERSION`.
- Rationale: schema is a contract; a fresh build's final schema is the union of
  both files, unchanged from the pre-split layout.

#### src/main.rs, src/cli.rs
- Scope: clap subcommands (build, callers, callees, impact, doctor [+ --invalidate], build --force); exit-code mapping; help texts stating direction semantics (impact = "files that call into <file>").
- Rationale: CLI contract per response contract section.

#### src/walk.rs
- Scope: `ignore` WalkBuilder from git root discovered upward; no git root ancestor -> build refuses (exit 2) unless `--root <path>` is passed; skip `.aw/` unconditionally; skip files > 1MB (counted); Extensions allowlist (.py in P1; .ts/.tsx/.js/.jsx in P2; .rs in P2.5).
- Rationale: CUJ1; prevents self-indexing, OOM inputs, and accidental home-directory walks.

#### src/parse.rs
- Scope: tree-sitter-python parse; ERROR-node count per file; per-file parse_status (ok | partial | failed); build aborts when parse-error rate exceeds `--max-parse-error-rate <f>` (default 0.05) with exit 7 unless `--force`.
- Rationale: silent hollow graphs are the worst failure mode — degraded extraction must be visible and gatable.

#### src/extract.rs
- Scope: symbol extraction (function, class, method, module); edge extraction per resolution algorithm section; alias map construction; confidence tier assignment; stats counters.
- Rationale: core IP; normative algorithm lives in spec, code mirrors it.

#### src/db.rs
- Scope: rusqlite open/create with user_version check; meta manifest (aw_version via env!("CARGO_PKG_VERSION"), grammar_version via compile-time constant string `<lang>-<grammar-crate-version>` derived from the Cargo.lock-pinned grammar crate versions, e.g. `py-0.23.6`; source_rev via git probe with `-dirty` suffix on a dirty tree; counts); flock build.lock; streamed chunked inserts (fixed 500-row chunks, no full-corpus accumulation); atomic swap sequence; WAL cleanup.
- Rationale: full-atomic-replacement policy; skew handling.

#### src/query.rs
- Scope: callers/callees/impact with: qualname disambiguation (exit 2 + candidates), result-set-scoped staleness check, cap/limit, tier tags, empty-result reason lines.
- Rationale: CUJ2/CUJ3/CUJ4; response contract.

#### src/doctor.rs
- Scope: integrity_check, user_version + grammar_version report, parse-error rate, stale ratio, orphan `.aw/` files detection + cleanup, lock leftovers, DB size vs target.
- Rationale: 3 AM diagnosability; CUJ3.

#### src/hash.rs
- Scope: FNV-1a 64 over raw bytes; fast path skip only when mtime AND size match.
- Rationale: staleness contract frozen (schema-relevant semantics).

#### src/error.rs
- Scope: `enum AwError { CorruptIndex, Locked, SchemaMismatch, Usage, NoResults, ParseQuality, Internal }`; Display with `aw: <class>:` prefix; panic-free boundary.
- Rationale: exit-code taxonomy; CUJ3, CUJ7, CUJ8.

#### tests/ (corpus + integration)
- Scope: fixture Python trees (aliased imports, shadowed methods, star import, getattr call, variable reassignment, >1MB file, syntax-error file) with committed expected outputs so any rerun replays the gate; golden stdout/stderr snapshots (cap + stale + ambiguous cases); tree-snapshot read-only test; proc-table test; SIGKILL-mid-build recovery test (next build succeeds, user_version intact); schema-skew test (hand-bump user_version -> exit 5 from every query); two-process flock test (hold lock, second build exits 4 with pid message, CUJ6); no-index test (CUJ7); parse-abort test (CUJ8); seatbelt + seccomp network-deny scripts; dep-allowlist test.
- Rationale: gates G1-G8; reproducible verification by committed fixtures.

#### .github/workflows/ci.yml
- Scope: cargo build --locked; clippy -D warnings; test (macos-latest + ubuntu-latest); allowlist check; golden snapshots; sandbox jobs — Seatbelt network-deny (macOS) and seccomp network-deny (Linux) running `tests/scripts/{seatbelt,seccomp}-build.sh`.
- Rationale: CI-locked invariants.

#### Internal baseline archive (P0 output; unavailable publicly)
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
- Gates: `hyperfine 'aw build'` (G1); `hyperfine 'aw callers <hot_fn>'` 30 runs hot+cold (G3); precision/recall script vs ground truth (G2); D0 harness A/B run (G6); doctor fault-injection (corruption, skew, orphans) — all kill-able per Gate Registry.
- Honesty rule: all numbers reported as measured with command attached; byte counts only, no token claims.
| G21 | Amendment 17 | one-call context: exact-byte fixtures, MCP parity, Stage 5 utility (context vs composed callers+callees+symbols) >= 20% tool-call cut on BOTH hosts | `cargo test --locked path_seed`-style fixtures + `cargo test --locked --test mcp` + Stage 5 runner (internal runner unavailable publicly, 24 cells) | remove `context`; no schema impact |
| G22 | Amendment 18 | R7 hardening conformance: multiline signatures, lexical scopes (Python nested def, Rust inline mod, TS namespace), module-name ambiguity, search exact/acronym/stem paths, clones min-lines/identity, impact/dead/context shapes, unresolved internal split, hotspots metrics, report freshness, no-op build, MCP -32602 | `cargo test --locked` (132 tests incl. the named R7 regressions) + self GT 10/10 with call-site precision 49/49 + path GT 8/8 + G14 search recall 0.775 (22/28, bar 0.70) + `cargo fmt --check` + `cargo clippy --all-targets -D warnings` + no-op build byte-unchanged test + measured search 0.74 ms mean (2,201 files) and no-op build 0.02-0.03 s | revert the specific fix whose gate regresses; no schema change |
| G23 | Amendment 19 | P8a core: inheritance fixture (Rust trait, TS interface, Python multiple), dead positive/negative, impact depth/edges/context, symbols <dir>, --path, v6 migration/partial/carry, MCP parity | `cargo test p8a_` + `cargo test --test mcp` + `timing_assert.sh` | revert P8a; no v6 release |
| G24 | Amendment 19 | import graph: 40 frozen pairs, recall >= 90%, precision >= 90%, Wilson; Python/TS/Rust fixtures; gap healing; MCP parity | `python3 tests/scripts/import_recall.py ...` + `cargo test import_` | remove import extraction/command and consumers |
| G25 | Amendment 19 | test impact: 10 seeds + 20 real-repo symbols, recall >= 90%, zero non-test false positives; MCP parity | `cargo test tests_impact_seed` + `tests_recall.py` | remove tests command and consumers |
| G26 | Amendment 19 | metrics/API/generated/unresolved: exact fixtures, EXPLAIN, v6 carry, MCP parity | `cargo test p8c_` | remove failed feature before v6 release; commands only after |
| G27 | Amendment 19 | resolve/search filters + skill drift: bytes/exits/caps, fenced-block gate, static manifest | `cargo test resolve_batch` + agent-skills drift gate | remove resolve/filters and skill lines |
| G28 | Amendment 19/P9 | Dart addendum: pinned grammar/license/node kinds, 20 call sites, 10 inheritance pairs, precision >= 80%, recall >= 70%, allowlist/write-path | `cargo test dart_` + `dart_recall.py` + allowlist/write-path | remove Dart grammar/extraction and consumers |
