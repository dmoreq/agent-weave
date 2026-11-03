# Changelog

All notable changes to aw are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning is
[Calm SemVer](https://calver.org) 0.x — breaking contract
changes bump the minor while 0.x is unstable.

## Reading this history

- Start with [getting started](docs/getting-started.md) for installation and rebuild steps, not historical rollout instructions.
- Entries describe their dated implementation snapshot. Later entries can supersede earlier schemas, limits, and output formats.
- Historical measurements and review verdicts below are internally reported, not new verification of this checkout.
  Original internal evidence is unavailable in the public distribution; this history does not make it publicly reproducible.
- Read [evaluation](docs/EVALUATION.md) for the public evidence summary and its limitations, not replacement original evidence.

## [Unreleased]

**User impact:** faster fresh builds, broader dependency navigation, and more reliable incremental resolution.
**Required action:** run `aw build` to replace older indexes with schema v7. Query opens on v6 exit 5.
Consumers that parse output must review the query-shape changes listed below before upgrading.

### Changed — project rename (2026-11-03)

- The package is now `agent-weave` and the installed command is `aw`. The
  index directory is `.aw/`, environment variables use the `AW_` prefix, and
  the MCP server identifies itself as `agent-weave`. Repository and
  documentation links move to `https://github.com/quy-doan/agent-weave`.
- Historical benchmarks recorded before 0.3.0 used the former name `kg`; the
  evaluation summary keeps those names only when quoting old artifacts.

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

### Changed — schema v7 (Amendment 20)

- `symbols.file_path` denormalizes `files.path`; `symbols_complexity_idx`
  covers `(cyclomatic, effective_loc, qualname, file_path, line_start)` so
  `aw complexity` tie-breaks by path, not insertion-order `file_id`.
- v6 indexes are not migrated: query opens exit 5 and `aw build` rebuilds.
  Fresh inserts and carry-forward both populate the column; `probe_old`
  rejects a v7-stamped index that lacks it.

### Fixed — incremental resolution and index validation (P8 review cycle 3, R12, Amendment 22)

- Incremental Rust `impl Trait for T` resolves against carried
  traits/interfaces; qualified `crate::Trait` impl targets resolve.
- Root TS `index.*`, Python `from .. import X`, and Rust inline `mod` members
  resolve their imports/calls; Rust trait-impl exports no longer leak
  inherent private methods, while module-parent `pub fn` exports.
- A pre-manifest v7 index rebuilds instead of reporting an empty default
  `complexity` result.

### Fixed — import resolution and query consistency (P8 review cycle 2, R11, Amendment 21)

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
- P8 gate commands select real tests; GT harnesses report Wilson precision.
- 1M-symbol `complexity` default is 6.9 ms via `meta.callable_count`;
  path scope is amended to a 250 ms budget (201 ms measured).

### Fixed — structural graph correctness and latency (P8 review cycle, R10)

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
  Flutter-corpus build smoke; P8 determinism dumps include schema-v7 columns
  and `unresolved_calls`.

### Added — dependency, test, and symbol queries (P8, Amendment 19, schema v6)

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

### Fixed — bounded queries and structural analysis (P8 implementation review, R9)

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

### Fixed — accuracy, utility, and latency hardening (R7, Amendment 18)

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

### Added — `aw context <symbol>` (Amendment 17, P8)

- One call returns the definition line with signature, direct callers, and
  direct callees (resolved `calls` edges only), with per-section showing
  counts and a first-line caveat. `--limit N` per section (default 10).
- Journal `args` gains `name` + `limit`; payload carries the displayed
  pointer lines; MCP tool `context` with G10 byte parity; contract-bump
  entry recorded in the spec.

### Added — `aw path <from> <to>` (Amendment 13, F5)

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
- Journaled per Amendment 6 (`args` gains `to`; no `limit` key); payload
  per Amendment 10 (`name` = the `from` argument; `usage --verify` unchanged).
  MCP tool `path` with G10 byte parity.
- Spec review: six independent Goldfish reviewers, 73 findings consolidated
  to 20, 2 fix cycles, round-2 APPROVE
   (internal review R5-P5, unavailable publicly); internally reported G17 gate
   evidence (original evidence unavailable publicly): 10/10 exact-byte fixtures,
  8/8 frozen GT pairs, 0.6-1.4 ms in-process best-of-7 across the 85k-symbol
  synth corpus and the 26-file self-index (spawn-inclusive wall < 10 ms).

### Changed — `aw search` readability (Amendment 14, P6)

- Indexed text widened with `split_ident(path)` + `split_ident(signature)`
  tokens (derived data; schema stays v4; the FTS is repopulated at build
  end). Signature-only vocabulary ("usize", "option path", "any", "i64 vec")
  now retrieves the symbols that carry it — frozen gap set 6/6 improved,
  frozen G14 recall unchanged at 0.850 pre-rebuild; the shipped P6 index
  measures 0.800 (23/28) — the G18(i) premise is corrected in the R6
  remediation (R6c F2).
- Search hit lines gain the stored one-line signature:
  `path:line qualname signature` (contract bump; empty signature keeps the
  12b byte shape). Payload/G10 parity unchanged.

### Changed — R6 residual closures (2026-11-02)

- PEP 420 namespace-package imports resolve (longest indexed prefix).
- Empty `dead`/`clones` results now print the mandatory caveat on stdout.
- MCP `initialize` negotiates the protocol version with the client.
- Serial-vs-parallel build determinism tested via `AW_WORKERS`.
- GitHub Actions pinned by SHA in both workflows.

### Reverted — `aw search` docstring indexing (attempted Amendment 16)

- Attempted schema v6 `symbols.docstring` + FTS widening. Retrieval improved
  (prose-gap 5/6, G14 unchanged at 0.800) but the Stage 2c utility A/B failed
  (Grok tool calls +12%, OpenCode −9%) — reverted under gate G20's kill rule.
  Evidence retained internally; original Stage 2c evidence is unavailable publicly.

### Fixed — R6 remediation (Amendment 15, schema v5)

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
  52-90%), G14 0.800 on the shipped index (was 0.850 pre-P6), G13 -62.7%
  (was -64.8%), denominators (40 primary runs + 2 fills; 10-run second host),
  test count 105, self-index epoch-labeled.

### Fixed

- `tests/scripts/write_path_check.sh`: stale exclusion pattern (G5 was red
  on the committed tree after the qualified `std::fs::rename(crate::db::...)`
  refactor); `src/journal.rs` added to the aw_dir-scoped allowlist.
- `db::probe_old`: a v4-stamped index from the Amendment 12 first landing
  wave (no `clone_groups` table) now disables the fast path and rebuilds
  fresh instead of failing carry-forward with
  `no such table: olddb.clone_groups`.

### Added — `aw symbols --outline` (Amendment 12a, F1)

- New `--outline` flag on `aw symbols <file>`: each line gains the stored
  one-line signature (first line of the symbol's node text, trimmed, hard
  cap 160 chars, no bodies). Default output (no flag) is byte-identical to
  the previous contract — contract-bump note: additive flag only, pointer
  shape unchanged, exit codes unchanged.
- Schema v4 (additive): `symbols.signature TEXT NOT NULL DEFAULT ''`,
  captured during the build parse phase at zero extra passes; carried
  forward by the fast path. Old indexes exit 5 with the rebuild hint
  (single normative version policy).
- Journal `args` for `symbols` gains the `outline` key (Amendment 6
  extension); MCP `symbols` tool gains the optional `outline` arg with G10
  byte-parity coverage.


### Added — agent-tooling design companion (P4 roadmap, Amendment 12, docs)

- `docs/designs/2026-11-01_p4-agent-tooling_amendment.md`: four agent-tooling
  features distilled from the pi-sherlock reference analysis — F1
  `aw symbols --outline` (signature column), F2 `aw search` (FTS5 symbol
  lookup), F3 skill hardening + drift gate, F4 `aw clones` (shingle
  candidates). Proposal spec-reviewed by an independent stateless Goldfish
  reviewer (round 1 NEEDS FIXES with 12 findings, round 2 APPROVE; record
  R4-P4, internal review unavailable publicly). OQ4a (signature text) and OQ4b
  (symbol-slice `search` deferral lift) approved by the owner 2026-11-01.
  Gates G13-G16 registered in the spec's Gate Registry. Implementation
  follows (F3 -> F1 -> F2 -> F4), one schema v4 bump for whatever survives
  the gates.

### Added — `aw search` (Amendment 12b, F2)

- New `aw search <terms>`: symbol-level FTS5 lookup — OR of prefix terms over
  camelCase/snake_case-split identifier tokens, `bm25()`-ranked. Output
  pointers `path:line qualname` (tier position empty — search hits are symbol
  identities, not call edges), standard cap/staleness/exit-code contract;
  empty terms exit 2, no match exits 1 with a reason line.
- Schema v4 (same bump as F1): standalone virtual table `symbols_fts`,
  repopulated by one pass from `symbols` at the end of every build (covers
  fresh and carried rows). No new dependencies (FTS5 is SQLite builtin).
- Journal `args` gains the `terms` key (Amendment 6 extension); `search`
  joins the Amendment 10 payload list; MCP `search` tool with G10 byte-parity.
- G14 gate PASS: mean per-query recall 0.850 on 20 frozen queries
  (pooled 24/28, Wilson 95% [0.685, 0.943]) —
  internally reported G14 evidence (original evidence unavailable publicly).

### Added — `aw clones` (Amendment 12d, F4)

- New `aw clones [--min-lines N]` (default 5): duplicate-block candidates —
  normalized 5-line-window FNV-1a shingles computed during the build parse
  phase (workers already hold the bytes — no extra reads), carried forward
  with the fast path; adjacent matching windows merged per file at query
  time; pair lines `pathA:startA-endA pathB:startB-endB` under the standard
  cap/staleness contract. Mandatory caveat first line: candidates, never
  proof. NOT payload-journaled (Amendment 12 deviation: pair lines are not
  single pointers, `usage --verify` structural checks do not apply).
- Schema v4 (same bump as F1/F2): `clone_groups` + `clone_groups_hash`
  index. No new dependencies.
- G16 PASS: 10/10 seeded pairs, 0 spurious (bar >= 8/10, <= 20%); build
  overhead +23.2% interleaved-measured — owner amended the proposed 15% bar
  to 25% with INSERT-I/O attribution on record
  (internally reported G16 evidence; original evidence unavailable publicly).

### Changed — performance ceiling pass (Amendment 11, G12)

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
  ground-truth call sites on vihap, provisional top-10 pain list.
- D0 harness: `g` call logger (byte/call accounting both directions),
  `count.py`, `check.py` (precision/recall with AMBIGUOUS-candidate and
  dynamic-dispatch rules), derivation scripts, cold-run equivalence check.
- Grep-arm baseline: 12 tool calls / 49,224 bytes; P0 exit criterion passed
  (premise not weak).
- R0 independent review record (APPROVE after 1 fix cycle).

### Added — local call-graph navigation (P1 core, 9f75656..e72b9b0)

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
- Gate results on vihap (~102k LOC): G1 1.91s/21.4MB/1.9MB; G2 recall 29/29,
  precision 493/495; G3 p95 10.1ms; G6 -50% tool calls, -67% bytes.
- R1 independent review record (APPROVE after 2 fix cycles).

### Added — index statistics and repository reports (P1.5, d9951ce..5034939)

- `aw stats`, `aw hotspots` (per-metric min-max normalization of
  cyclomatic + LOC + churn-90d + fix-density), `aw report` (4 sections).
- G7 rank-overlap 8/10 vs pain list (degraded: pain list provisional).
- G8 -67% tool calls on architecture questions (via `callees --depth`).
- `code-index` skill wrapper in the agent-skills repo (OQ2).
- R1.5 independent review record (APPROVE after 1 fix cycle).

### Added — TypeScript, JavaScript, and Rust indexing (P2, 2aeeb9d..7a99343)

- TypeScript/TSX and JavaScript/JSX grammars (one-commit upgrade rule);
  Rust grammar (P2.5, rs-0.24.2).
- TS/TSX extraction: classes, methods, arrow-const components/hooks,
  `new` -> constructor redirect, `this`/`super` MRO calls, ESM imports
  (default/named/namespace/type), directory-based relative resolution
  (dot-stem safe), external-module drop rule.
- Rust extraction: functions, structs/enums/impls, `use` and `mod`
  declarations, `crate::` call paths, scoped identifiers.
- G2-finguard: 20/20 recall, 57/57 precision. G2-self (dogfooding): 9/10,
  14/14. Python regression green throughout.
- R2 independent review record (APPROVE after 2 fix cycles).

### Added — docs

- In-repo canonical spec + concepts brief under `docs/designs/` (cross-repo
  references retired); GitHub community health files.

### Fixed — post-R2 review (Amendment 4)

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
- README review-records table corrected (R0/R1 in-tree; R1.5/R2 cycles live in
  commit history).

### Added — A3 observability layer 3 (Amendment 10)

- Result-payload journal: `.aw/usage-results.jsonl` — one self-contained
  JSONL line per successful pointer query (callers/callees/impact/dead/
  symbols) carrying the displayed set byte-identical to the stdout data
  lines plus index `built_at`/`source_rev`. Non-pointer tools and
  empty/ambiguous/error outcomes write none. Rotation (5 MB), opt-out
  (`AW_NO_JOURNAL` / `--no-journal`), and best-effort contract mirror
  usage.log.
- Session attribution: `AW_SESSION`/`AW_ARM` env tags ride every usage.log
  and payload line as additive fields — R1/R2 A/B attribution per task
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
  fresh builds. Conformance run on the 3 GT repos (aw self / vihap /
  finguard): 594 pointers verified, FAIL=0; byte-parity payload-vs-stdout
  confirmed. Tests 91/91; clippy -D warnings clean.
- Support changes: index meta `built_at` now carries millisecond precision
  (the verify epoch comparison must not collide on same-second rebuilds);
  aw-self GT rows G01/G02/G07/G09/G10 refreshed after the Amendment 10 line
  shifts (provenance in the GT file; G06 annotation corrected — the M3-era
  file mislabeled the Dead dispatch as Stats).

### Added — M4 prebuilt index artifacts (Amendment 9)

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

### Added — M3 MCP stdio server (Amendment 8, P3 activated)

- `aw mcp`: newline-delimited JSON-RPC 2.0 server on stdin/stdout; 11 tools
  mapping 1:1 to CLI subcommands (doctor excluded — `--invalidate` stays a
  human-CLI maintenance action). Spawned by the agent host; aw still spawns
  no daemons. Zero network; writes confined to `.aw/`.
- Tool results return the CLI's byte-identical stdout text (G10 parity
  conformance is automated in tests/mcp.rs); ambiguous names carry the
  candidate list in the result text with isError (exit-2 semantics
  preserved); unknown tools rejected with -32602.
- J2: every tool call journals with `surface:"mcp"` (same Amendment 6
  schema); the server process itself is not journaled.
- Dependency policy (additive, recorded in Amendment 8): `serde_json = "1"`
  joins the pinned set — the MCP boundary parses untrusted client JSON;
  `rmcp` rejected (tokio runtime tree contradicts minimal-dependency/no-daemon
  build semantics).
- Structural refactor: `run_build` moved to shared `src/build.rs` (CLI and
  MCP share it); ambiguous-name candidates now travel in
  `AwError::Ambiguous` instead of being printed mid-query (stdout stays a
  pure protocol/data channel on every surface).
- aw-self GT rows G02-G10 refreshed after the M3 line shifts (provenance in
  the GT file); G2 vihap 29/29 + 493/495, finguard 20/20 + 57/57 unchanged.
- Tests 76/76 (3 new MCP conformance tests); clippy -D warnings clean.

### Added — M2 resolution recall (Amendment 7)

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

### Added — J1 usage journal (Amendment 6)

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

### Added — M1 query surface (Amendment 5)

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
  evidence unavailable publicly): G1 0.47s/22.8MB/1.98MB, G2 vihap 29/29 + 493/495,
  finguard 20/20 + 57/57, self 9/10 + 14/14 (all matching pre-fix numbers —
  zero regression), G3 p95 9.8ms, G7 8/10.
- G2-finguard G02 row refreshed (finguard working tree moved; GT staleness,
  not a regression). G6/G8 A/B re-runs marked pending with rationale.
- README gates table updated to the re-baselined numbers.

### Skipped

- P3 stdio MCP server (original trigger): "CLI discoverability measurably
  fails" never occurred — the phase was skipped per its trigger rule, then
  ACTIVATED as M3 by owner decision (MCP-native demand), recorded as
  Amendment 8.
- M5 additional languages (Go/Java/C#): trigger "real workload demand" not
  met — no such workload identified. Re-evaluable on request; the P2
  per-language template (grammar pin -> extraction -> fixtures -> GT -> G2)
  remains the playbook.

[Unreleased]: no tagged releases yet — history starts at f9d57b9 (repo init).
