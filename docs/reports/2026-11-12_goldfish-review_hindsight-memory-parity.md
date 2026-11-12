# Goldfish Review Report — Hindsight-Derived Memory Capabilities

- Target: `docs/designs/2026-11-12_hindsight-memory-parity.md` (FeatureRange / DetailedRange)
- Date: 2026-11-12
- Lenses run: 5 (Zoom-Out, Comprehension, Design Critic, Implementation Readiness, Codebase Details)
- Cycle: 1 of at most 3 (`AGENTS.md` §1)
- Semantic split at cycle start: `## Detailed Implementation Plan`

## Findings

### F1 — Legacy and v2 union keeps the consonant collision and mixes BM25 scales

- Severity: Critical
- Lenses: Zoom-Out, Design Critic, Codebase Details
- Scope: `## Technical Plan` B1; goals
- Description: A union arm over `memories_fts` and `memories_fts_v2` matches legacy skeleton tokens, so `nh` still returns `Bộ nhớ`. `MIN(bm25)` across two tokenizers is not comparable.
- Recommendation: Query v2 only when present; use legacy only when v2 is absent.

### F2 — Migration hooks cover only two of five write opens

- Severity: Critical
- Lenses: Zoom-Out, Design Critic, Codebase Details
- Scope: B1 migration
- Description: `open_global_memory_store`, `open_global_memory_mutation`, and `open_memory_mutation_at` bypass the named hooks, so global stores never gain v2 and dual deletes fail.
- Recommendation: Call the ensure function once in `open_memory_conn_path`.

### F3 — Backfill indexes superseded rows and leaves orphans

- Severity: Critical
- Lenses: Implementation Readiness, Codebase Details, Comprehension
- Scope: B1 backfill
- Description: The backfill selects all `memories` rows, while the legacy FTS mirrors current rows only; GC deletes `memories` rows without touching v2; rowid reuse can bind a stale v2 row to a new note.
- Recommendation: Mirror the legacy FTS row set exactly; delete orphans; delete v2 on supersede, delete, and GC.

### F4 — Upgrade without a write leaves the defect in place

- Severity: Critical
- Lenses: Zoom-Out, Codebase Details, Comprehension
- Scope: CUJ 1; B1
- Description: v2 is created only on a write open; a read-only query against an upgraded store still fails.
- Recommendation: Document the first-write trigger; seed CUJ 1 through `mem write`; use legacy terms when v2 is absent; add a read-only fallback test.

### F5 — Old-binary writes leave v2 stale

- Severity: Critical
- Lenses: Zoom-Out, Codebase Details, Comprehension
- Scope: CUJ 7; B1
- Description: A 1.0.4 write inserts legacy text only; the anti-join would never refresh an existing v2 row.
- Recommendation: Add `fts_version`; reconcile rows `< 2`; emulate the old binary in tests.

### F6 — Explain map keyed by row id collides across stores

- Severity: Critical
- Lenses: Zoom-Out, Implementation Readiness, Codebase Details
- Scope: A2
- Description: Local and global row ids are independent; a merged query can attach one store's explanation to another note.
- Recommendation: Key by `(is_global, id)`.

### F7 — New CLI tests are never compiled

- Severity: Critical
- Lenses: Implementation Readiness, Codebase Details
- Scope: file ledger; test plan
- Description: `tests/cli/main.rs` declares each module; a new file without `mod` is an orphan, and the filter matches zero tests.
- Recommendation: Add `tests/cli/main.rs (Modify)` with the module line; same for `tests/mcp/main.rs`.

### F8 — Phase 2 depends on a type created in Phase 3

- Severity: Critical
- Lenses: Implementation Readiness, Comprehension
- Scope: phases
- Description: `memory_fts_candidates` takes `TimeWindow` before `src/memory/time.rs` exists.
- Recommendation: Create `TimeWindow` in Phase 1; add the parameter in Phase 3.

### F9 — The listed signature changes break unlisted call sites

- Severity: Critical
- Lenses: Implementation Readiness, Codebase Details
- Scope: A1; file ledger
- Description: Eight `MemFilter` literals exist, plus `memory_fts_candidates` calls in `guard.rs` and two test files, plus `memory_ref_candidate_rows` in `graph.rs`.
- Recommendation: Enumerate every site; add `src/memory/graph.rs`, `src/db/memory_tests.rs`, and `src/db/memory_gc_tests.rs` to the ledger.

### F10 — The 500-LOC hygiene gate fails under the planned edits

- Severity: Critical
- Lenses: Implementation Readiness, Codebase Details
- Scope: file ledger
- Description: `service/from_cli.rs` (491), `service/tools.rs` (467), `tests/mcp/memory.rs` (482), and detector tests in `note.rs` would exceed 500.
- Recommendation: Pre-plan splits: `from_cli/memory.rs`, `tools/memory.rs`, `tests/mcp/memory_window.rs`, `memory/secret.rs`.

### F11 — Audit filtered-empty change is inert

- Severity: Important
- Lenses: Implementation Readiness, Codebase Details
- Scope: A1
- Description: Empty `mem query` exits 1; `Metrics::add` already skips non-zero exits; the audit `Entry` has no `args`.
- Recommendation: Drop the audit change and mark `src/audit/collect.rs` as no change.

### F12 — Explain lexical counts contradict the match

- Severity: Important
- Lenses: Zoom-Out, Design Critic
- Scope: A2
- Description: `mem_tokens` preserves accents while FTS matches through SQL folding; `lexical 0/2` would explain a successful match.
- Recommendation: Replace the ratio with `lexical match` when the row came from the FTS arm.

### F13 — `ref <token>` contradicts the no-note-text rule

- Severity: Important
- Lenses: Design Critic, Comprehension
- Scope: A2
- Description: Tokens are alphanumeric; `src/memory/rank.rs` is a raw ref, not a token; the spec both prints and forbids note-derived text.
- Recommendation: Print the raw ref truncated to 60 characters (tags as `tag <token>`), never body text, in fixed order.

### F14 — `refs shared by N` is not causal and depends on the candidate set

- Severity: Important
- Lenses: Zoom-Out, Design Critic
- Scope: A2; D2
- Description: It is a corroboration proxy, not a match reason; it changes with cap, limit, window, and store scope.
- Recommendation: Cut it; D2 becomes "cut".

### F15 — Default bytes cannot be unchanged for accented content

- Severity: Important
- Lenses: Design Critic, Implementation Readiness, Comprehension
- Scope: goals; verification
- Description: Unicode tokenization intentionally changes candidates and ranking.
- Recommendation: Scope the promise to the ASCII oracle with a fixed ranked set and no `--explain`.

### F16 — Time parser signature, grammar, and edge cases were underspecified

- Severity: Important
- Lenses: Implementation Readiness, Design Critic, Comprehension
- Scope: A1
- Description: No `command` parameter, bare integers are ambiguous with dates, invalid calendar dates and overflow were undefined.
- Recommendation: Add the `command` parameter; require `@<seconds>`; reject invalid dates and overflow; state exact messages.

### F17 — MCP numeric `since` is silently ignored

- Severity: Important
- Lenses: Codebase Details, Implementation Readiness
- Scope: MCP args
- Description: `ArgReader::s` returns `None` for non-strings, so `"since": 1762905600` becomes no filter.
- Recommendation: Add `s_opt_spec`; non-string values are usage exit 2.

### F18 — The window is not applied to `memory_current_keys`

- Severity: Important
- Lenses: Design Critic, Codebase Details
- Scope: A1
- Description: An out-of-window local row still shadows an in-window global row.
- Recommendation: Add the window to the shadowing probe.

### F19 — `--scan` can leak the key and its output contract is ambiguous

- Severity: Important
- Lenses: Zoom-Out, Design Critic, Implementation Readiness
- Scope: B2
- Description: The key is scanned and printed; multi-field matches, ordering, and N=0 were undefined.
- Recommendation: Print `row <id>` when the key matched; one line per note with comma-separated fields in fixed order; N=0 prints nothing.

### F20 — Scan scope and restore behavior were unstated

- Severity: Important
- Lenses: Design Critic
- Scope: B2
- Description: Only current rows are scanned; `mem import` rejects a warned note.
- Recommendation: State "current notes only" and document the restore consequence.

### F21 — Detector coverage holes and false positives

- Severity: Important
- Lenses: Design Critic, Codebase Details
- Scope: P1
- Description: Fine-grained GitHub PATs, `gho_/ghu_/ghs_/ghr_`, case-insensitive `Bearer`, and dotted identifiers were unhandled.
- Recommendation: Add prefixes; require `eyJ`; case-insensitive `Bearer`; add a false-positive corpus.

### F22 — Secret tests trip the hygiene scanner and the old test fails

- Severity: Important
- Lenses: Implementation Readiness, Codebase Details
- Scope: P1
- Description: `ghp_` + 36 and `AKIA` + 16 literals are leak-pattern hits; the existing short-vector assertions now pass.
- Recommendation: Build vectors at runtime; replace the old assertions; keep tests in `memory/secret.rs`.

### F23 — Documentation surfaces were incomplete

- Severity: Important
- Lenses: Implementation Readiness, Codebase Details, Design Critic
- Scope: P2; cross-surface
- Description: The MCP `mem_touch` description, service doc comment, CLI reference, skill, and schema header were missing; the architecture page has no timestamp definitions.
- Recommendation: List every surface and the exact wording.

### F24 — Baselines, fixtures, and scale commands were not runnable

- Severity: Important
- Lenses: Implementation Readiness, Codebase Details, Comprehension
- Scope: verification
- Description: `docs/baselines/` is absent; `mem_recall.py` does not create it and gates at 0.8; no scale generator existed.
- Recommendation: `mkdir -p docs/baselines`, `--threshold 1.0`, add `tests/scripts/mem_scale.py`.

### F25 — Old-binary verification was not executable

- Severity: Important
- Lenses: Zoom-Out, Implementation Readiness, Codebase Details
- Scope: verification
- Description: No 1.0.4 binary is available offline.
- Recommendation: Emulate the old write (legacy text + `fts_version = 0`) in a test; label it an emulation.

### F26 — Context and terms were not self-contained

- Severity: Important
- Lenses: Zoom-Out, Comprehension
- Scope: whole document
- Description: Hindsight, parity, Gate 2, D1-D4, FTS, BM25, current row, and purity were undefined or linked only to the predecessor.
- Recommendation: Add `## Terms`, the Hindsight definition, the decision table, and the version.

### F27 — Cross-table BM25, kill switch, and availability paths were unmitigated

- Severity: Important
- Lenses: Design Critic
- Scope: NFR; risks
- Description: No union is retained; a v2 failure blocks a write; dropping v2 is not a durable switch.
- Recommendation: State write rollback, query failure, and "downgrade is the switch" explicitly; add a risk table.

### F28 — Concurrent create and v2 schema mismatch were unhandled

- Severity: Important
- Lenses: Codebase Details, Comprehension
- Scope: B1
- Description: Two writers can race the create; an existing table with the wrong tokenizer was not checked.
- Recommendation: `CREATE VIRTUAL TABLE IF NOT EXISTS`; inspect `sqlite_master.sql`; drop and recreate on mismatch.

## Suggestions

| ID | Finding | Decision |
|---|---|---|
| S1 | Reuse `civil_from_days` instead of duplicating date math | Fixed: `src/time.rs` |
| S2 | Reuse `object_exists` for the v2 probe | Fixed |
| S3 | Reuse the existing no-store error for `--local` | Fixed |
| S4 | Replace two booleans with `StoreScope` | Fixed |
| S5 | `FtsTables` struct is speculative | Fixed: removed; plain `v2: bool` |
| S6 | Scan field cardinality | Fixed in B2 |
| S7 | Bare integer ambiguity | Fixed: `@<seconds>` |
| S8 | `ref` label note-text contradiction | Fixed in A2 |
| S9 | `graph unavailable` hides index errors | Fixed: positive-only labels |
| S10 | Reconcile on every write open | Fixed: rows `< 2` only, chunked |
| S11 | CUJ hardcodes the `why:` string | Fixed: invariant only |
| N1 | "three validation call sites" | Fixed: "all validation sites" |
| N2 | Test module registration and fixture ledger | Fixed |
| N3 | Slack/AWS tests missing | Fixed: one case per pattern |

## Reconciliation

| Finding | Decision | Evidence |
|---|---|---|
| F1 | PROPOSE FIX | Design Critic 1 and 4; `src/memory/tokens.rs:78`; measured `bo nho` no-results |
| F2 | PROPOSE FIX | `src/db/memory_store.rs:190-229` lists five write opens sharing `open_memory_conn_path` |
| F3 | PROPOSE FIX | `src/db/memory_write.rs:326` deletes legacy FTS on supersede; `src/db/memory_query.rs:291-297` deletes superseded rows |
| F4 | PROPOSE FIX | `src/db/memory_store.rs:332-344` opens read-only without migration |
| F5 | PROPOSE FIX | `src/db/memory_write.rs:184-188` writes legacy only |
| F6 | PROPOSE FIX | `src/service/mem_query.rs:259-292` merges candidates; `id` is per-database |
| F7 | PROPOSE FIX | `tests/cli/main.rs:4-24`, `tests/mcp/main.rs:4-8` |
| F8 | PROPOSE FIX | Phase dependency is internal to the spec |
| F9 | PROPOSE FIX | `guard.rs:84`, `graph.rs:66,280`, `memory_tests.rs:38,128,298`, `memory_gc_tests.rs:28,69` |
| F10 | PROPOSE FIX | `wc -l`: 491, 467, 482; `hygiene_check.sh:80,94` |
| F11 | PROPOSE FIX | `src/audit/collect.rs:122-141`; `src/error.rs:117` |
| F12 | PROPOSE FIX | `mem_tokens` preserves accents; SQL folding does the match |
| F13 | PROPOSE FIX | `src/memory/tokens.rs:92-125` returns tokens, not raw refs |
| F14 | PROPOSE FIX | D2 re-resolved as cut; no consumer requires it |
| F15 | PROPOSE FIX | The change intentionally alters accented ranking |
| F16 | PROPOSE FIX | `src/memory/time.rs` is new; no existing parser |
| F17 | PROPOSE FIX | `src/service/args.rs:29-31` returns `None` for non-strings |
| F18 | PROPOSE FIX | `src/service/mem_query.rs:281-284` passes no filter |
| F19 | PROPOSE FIX | A key can be the secret; output is the acceptance surface |
| F20 | PROPOSE FIX | `src/service/mem_import.rs` validates every note |
| F21 | PROPOSE FIX | `src/memory/note.rs:80-85` is a five-prefix substring check |
| F22 | PROPOSE FIX | `hygiene_check.sh:32,43-48`; `src/memory.rs:359-386` |
| F23 | PROPOSE FIX | `tools.rs:363`, `mem_ops.rs:75`, `docs/cli-reference.md:253`, `skills/engram-first/SKILL.md:94` |
| F24 | PROPOSE FIX | `docs/baselines` absent; `mem_recall.py:90,156-157` |
| F25 | PROPOSE FIX | No offline 1.0.4 artifact |
| F26 | PROPOSE FIX | Whole-document readability |
| F27 | PROPOSE FIX | Risk table added |
| F28 | PROPOSE FIX | `memory_store.rs:89-98` already tolerates the create race |
| Zoom-Out C3 single-table rebuild | CONTEST | A single rebuilt table removes the legacy tokens, so 1.0.4 accented queries stop matching. Combining both token sets in one table reintroduces the skeleton collision (Design Critic F1). Dual tables with v2-only queries are the only shape that satisfies both constraints. |
| Zoom-Out I3 union/BM25 | PROPOSE FIX | Union removed; v2-only plus repair |
| Design Critic 6 export as backup | PROPOSE FIX | Export is JSONL export, not a full snapshot; wording corrected |
| Design Critic 10 kill switch | PROPOSE FIX | Downgrade is stated as the only switch |
| Comprehension 1 byte-identical | PROPOSE FIX | Scoped to the ASCII oracle and fixed ranked set |

## Residual risks

- A read-only store that has never been written since the upgrade stays on legacy tokens until the first write. Documented; the bilingual oracle seeds through `mem write`.
- A mixed-version window in which 1.0.4 writes a note and the new binary only reads it leaves that note unindexed in v2 until a write open repairs it.
- The secret detector remains best-effort; `--scan` is advisory and does not block export.
- Ranking for non-ASCII notes intentionally changes; the bilingual oracle records the delta.

## Re-audit

Cycle 1 produced 28 consolidated findings; 27 were accepted and one was contested with code evidence. A single re-audit runs the Design Critic and Implementation Readiness lenses against the revised ranges.

## Cycle 2 re-audit

- Lenses: Design Critic (FeatureRange) and Implementation Readiness (DetailedRange)
- Revised ranges: `FeatureRange` 1-417; `DetailedRange` 418-782
- Result: 31 findings (6 Critical, 20 Important, 5 Suggestion/Nit). All accepted and applied; no contest.

### Critical

| ID | Finding | Fix applied |
|---|---|---|
| R1 | v2 table could exist while only partially backfilled, so reads silently omitted rows | Added `memory_meta.fts_v2_complete`; reads use v2 only when the marker is `1`; every repair clears it before the first chunk and sets it after the last |
| R2 | `migrate_memory_schema` early-returns on `anchored_at`, so `fts_version` and the new indexes would never be added | Exact replacement body: independent column/index steps, duplicate-column tolerance, propagated transaction errors |
| R3 | The phases never scheduled the tokenizer rewrite; the legacy/v2 text assignment was implicit | Phase 1 now owns the tokenizer; writes name `mem_fts_text_legacy` for legacy and `mem_fts_text` for v2 |
| R4 | `fts_version` could be added to `MEMORY_REQUIRED`, making every old store exit 5 before migration | Stated explicitly that `fts_version`, `memory_meta`, and the v2 table are excluded from the manifest |
| R5 | The guard's v2/legacy term pairing was unspecified, so consolidation hints could silently stop | Fixed pairing: usable -> `(v2, mem_query_terms)`; otherwise `(legacy, mem_query_terms_legacy)`; fixture writes both tables |
| R6 | The candidate SQL used ambiguous `rowid` and no window parameter; the phase signature contradicted the goal | Exact qualified SQL verified on SQLite 3.54 (aliasing breaks `bm25`); final signature includes `window` and `now`; current-row predicates precede `LIMIT` |

### Important

| ID | Finding | Fix applied |
|---|---|---|
| R7 | Window predicates and call-site signatures were under-specified | Exact `-1` sentinel predicates for all four read paths and eight `MemFilter` sites |
| R8 | `object_exists` is private and swallows errors | Added `src/db/query_state.rs` to the ledger with a `pub(super)` or `Result` variant |
| R9 | `Candidate` literal in `src/memory.rs` would not compile | Added to the Phase 4 change list |
| R10 | The `service.rs` line budget was left as a conditional decision | Moved `CommandRequest` to `src/service/request.rs`; re-exported |
| R11 | `mem_export_request` does not exist | Specified the new function and its destructuring |
| R12 | A const slice cannot be spliced from a submodule | `tools/memory.rs` exports individual `pub(super) const` specs referenced by `TOOL_SPECS` |
| R13 | `MemQueryArgs` had no `explain` field | Added `explain` and `legacy` |
| R14 | The export journal arm was unassigned | Assigned to the `MemExport` arm of `journal_args_memory` with exact order |
| R15 | Phase gates could match zero tests | Tests are created and registered in the phase that uses them; exact test paths in the matrix |
| R16 | The hygiene gate checked only the new files, not modified ones | Gate passes `$(git ls-files)` plus the new paths |
| R17 | Baseline commands assumed a release binary that was never built | Added `cargo build --locked --release` |
| R18 | The ledger named one baseline file while the commands write three | Three exact paths plus the write baseline |
| R19 | The scale gate contradicted "recorded, not asserted" | Defined the workstation gate, fixed seed, warm run, and JSON keys |
| R20 | No golden artifact existed for the byte claim | Byte-identical claim removed; replaced with a testable `why:` presence/absence invariant |
| R21 | The epoch fixture value was wrong | `2026-11-12` = `1794441600` (verified) |
| R22 | Tokenizer test vectors contradicted the design | Exact vectors: `["bộ","nhớ","dự","án"]`, `["dường"]`, legacy `["b","nh"]` |
| R23 | The purity test did not assert the legacy text | Both tables' text asserted for write, import, and supersede |
| R24 | Several required tests were unnamed | Matrix expanded to 36 rows, including partial v2, epoch, stale-cap, conflict parity, and bool strictness |
| R25 | `mem_ops.rs` and the single CLI test file risked the 500-line cap | Added `mem_export.rs`, `scope.rs`, `explain.rs`, and a `memory_parity/` test directory |
| R26 | `from_fts` origin was not derivable | `fts_ids` set collected before merging; ref-only and graph rows pass `false` |
| R27 | New MCP booleans were silently coerced | Added `ArgReader::b_spec`; one conflict check in `StoreScope::from_flags` |
| R28 | Documentation wording was grouped, not per-file | Added a per-file table with anchors |
| R29 | Fresh and migrated column order could differ | `fts_version` is last in both; `PRAGMA table_info` parity test |
| R30 | Old-binary supersede could leave 300 stale v2 rows and starve the cap | Candidate SQL filters current rows before `LIMIT`; stale-cap test |
| R31 | The read-only upgrade path and runtime switch were missing | Added `eg mem reindex` and `--legacy`; documented downgrade |

### Suggestions and nits

| ID | Finding | Fix applied |
|---|---|---|
| R32 | `civil_from_days` duplication | Moved to `src/time.rs`; audit imports it |
| R33 | Purity definition followed legacy FTS, not `memories` | Redefined over current `memories` rows; reconcile inserts missing rows in both tables |
| R34 | Tokenizer epoch not tracked | Added `fts_epoch`; mismatch forces a full rebuild |
| R35 | Scan MCP parity claim contradicted CLI-only | Removed the claim; documented the exception in agent integration |
| R36 | `ref` in explain could leak a credential | Runs `suspected_secret`; prints `ref <redacted>` |
| R37 | Reconcile cost on every open | Pending-only scan with a partial index; orphan sweep only on full rebuild |
| R38 | Goals 3 and 7 were unfalsifiable | Replaced with `why:` presence/absence and numeric scale budgets |
| R39 | Storage estimate used the legacy index | `mem_scale.py` records both table sizes |
| R40 | Alternatives A/B were not viable peers | Labeled goal-incompatible; added trigger-maintained and prefix-only alternatives |
| R41 | `database-url` rejected user-plus-port URLs | Requires a password separator `:` immediately before `@` |
| R42 | No-change ledger entries | Moved to "Inspected, no change" |

### Residual risks after cycle 2

- A mixed-version read-only window remains: a 1.0.4 write is invisible to v2 until a write open or `eg mem reindex`.
- The secret detector is best-effort; `--scan` warns but does not block.
- Non-ASCII ranking changes intentionally; the bilingual oracle and scale JSON record the delta.
- The 5,000-note performance gate runs on the development workstation, not in CI.

## Cycle 3 re-audit (final)

- Lenses: Implementation Readiness (DetailedRange) and Codebase Details (DetailedRange)
- Result: 52 raw findings, 10 Critical/Important consolidated below plus 15 applied suggestions. All accepted; no contest.
- Review bound: this is cycle 3 of the `AGENTS.md` §1 maximum. Nits are recorded and closed; no further review cycle.

### Critical and Important

| ID | Finding | Fix applied |
|---|---|---|
| T1 | `CommandRequest::MemReindex` had no `run_memory` dispatch arm; moving `run_mem_export` broke the `run.rs` call | Added `src/service/run.rs` to the ledger and phases with the import and arm |
| T2 | New service modules had no `mod` declarations | Added `mod request; mod explain; mod scope; mod mem_export; mod mem_reindex;` to the `service.rs` entry and Phase 2 |
| T3 | `eg mem reindex` had no implementation home, signature, or open mode; the automatic ensure double-ran | Specified `src/service/mem_reindex.rs`, `MemoryMode::Mutation`, the forced-pass count, output streams, and MCP text |
| T4 | Phase 2 used a window signature introduced in Phase 3 | Phase 2 now uses the final signature with `TimeWindow::default()`; `TimeWindow` exists from Phase 1 |
| T5 | The migration was a behavioral summary, not exact DDL | Replaced with six ordered statements, transaction boundaries, and error rules |
| T6 | 1.0.4 compatibility had no executable reverse-direction test | Added `tests/scripts/old_binary_compat.sh` and the SQL emulation test |
| T7 | The bilingual oracle was created after implementation | Moved to Phase 0 with an immutable schema and a recorded SHA-256 |
| T8 | The Rust current-row predicate had no window branch | Added exact `since`/`until` conditions and the expired-ref predicate |
| T9 | Legacy fallback ref/tag tokenization was undefined | Added the three-mode tokenizer matrix and `mem_ref_tag_tokens_legacy` |
| T10 | The export/reindex journal arms were unassigned | Assigned to `journal_args_memory` with exact field order and values |
| T11 | Window function signatures and visibility were unspecified | Added full signatures and `pub(crate)` visibility |
| T12 | `MemQueryArgs.legacy` and `index_present` were undefined | Added both fields and `index_present = args.index_conn.is_some()` |
| T13 | `object_exists` left an API choice and hid errors | Fixed `object_exists_result` and the two-table probe |
| T14 | MCP parameter specs and typed readers were unspecified | Added the `ParamSpec` table and exact `s_opt_spec`/`b_spec` contracts |
| T15 | Required edge-case tests were missing | Matrix expanded to 50 rows |
| T16 | A CLI test asserted MCP parity | Split into `local_global_conflict_cli` and the MCP test |
| T17 | Schema-equivalence assertions were too narrow | Expanded to columns, indexes, v2 SQL, meta keys, and `user_version` |
| T18 | Scan field scope and limit semantics were ambiguous | Fixed to four fields, post-limit rows, empty export exits 1 |
| T19 | Reindex output streams contradicted each other | Stdout is one line; stderr carries multi-chunk progress |
| T20 | Baseline count and comparability were inconsistent | Four files, fixed parameters, `--baseline` comparison |
| T21 | Moved `from_cli` methods would lose visibility | Specified `pub(super)` on all four builders |
| T22 | Docs changes were not per-file | Added the tool count, `created_at` wording, and schema v2 doc line |
| T23 | Phase 1 would break the existing single-table path | Phase 1 keeps call sites on legacy functions; Phase 2 switches them |
| T24 | GC bypassed dual deletion and purity was incomplete | GC deletes v2; purity is defined over current rows; orphan sweep on rebuild |

### Suggestions and nits

| ID | Finding | Fix applied |
|---|---|---|
| T25 | Line-number anchors go stale | Replaced key anchors with function names |
| T26 | Legacy candidate SQL not written out | Added the mechanical twin and `candidate_sql_prepares` asserts both |
| T27 | `fts_version` placement between paths was ambiguous | One shared `insert_revision` statement; both callers inherit |
| T28 | Scan `session` and empty-store behavior | `session` excluded; empty export exits before scan |
| T29 | Prerequisites omitted `git` and `sqlite3` | Added; scale uses `DROP TABLE` to stay at 3.30 |
| T30 | Fresh vs migrated column order differs for `anchored_at` | Parity compares names and types, not positions |
| T31 | Test helpers and `ENGRAM_GLOBAL_DIR` were unnamed | Added the helper list and isolation rule |
| T32 | Tuning constants were undocumented | Documented 256, 32, and 80 |
| T33 | `audit/collect.rs` "ignore" wording was inaccurate | Corrected to success/empty skip with critical counts retained |
| T34 | `docs/cli-reference.md:218` omitted the field name | Added `created_at` |
| T35 | `agent-integration.md` tool count | Updated 19 -> 20 and seven maintenance tools |

### Residual risks after cycle 3

- Mixed-version read-only windows remain until a write or `eg mem reindex`.
- The detector is best-effort; the scan warns without blocking.
- Non-ASCII ranking changes are intentional and recorded.
- The 5,000-note performance gate runs on the development workstation.
- The spec contains no implementation evidence; all code claims are proposals.
