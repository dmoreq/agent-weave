# Changelog

All notable changes to kg are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning is
[Calm SemVer](https://calver.org) 0.x — breaking contract
changes bump the minor while 0.x is unstable.

## [Unreleased]

### Added — `kg path <from> <to>` (Amendment 13, F5)

- Shortest call path between two symbols over resolved call edges
  (`kind = 'calls' AND dst_id IS NOT NULL`): level-batched BFS, deterministic
  (FIFO frontier, neighbors ordered by `dst qualname, dst path, site_line`,
  multi-edges deduped to the smallest `site_line`, parent = first
  discoverer); `--depth N` (default 10, clamp 1-10).
- Output contract (contract-bump): first-line caveat
  `kg: path — resolved static edges only; dynamic dispatch may hide paths`,
  hop call-site pointers in path order (no cap — `path` has no limit
  surface, recorded deviation), final line
  `kg: chain <resolved qualnames> (K hops)`. Exit 1 negatives distinguish
  exhaustive vs depth-cap-truncated; exit 2 ambiguity names the failing
  endpoint (`as the from|to endpoint`).
- Journaled per Amendment 6 (`args` gains `to`; no `limit` key); payload
  per Amendment 10 (`name` = the `from` argument; `usage --verify` unchanged).
  MCP tool `path` with G10 byte parity.
- Spec review: six independent Goldfish reviewers, 73 findings consolidated
  to 20, 2 fix cycles, round-2 APPROVE
  (`docs/baselines/reviews/R5-P5.md`); G17 gate evidence
  (`docs/baselines/g17-path-2026-11-02.md`): 10/10 exact-byte fixtures,
  8/8 frozen GT pairs, 0.6-1.4 ms in-process best-of-7 across the 85k-symbol
  synth corpus and the 26-file self-index (spawn-inclusive wall < 10 ms).

### Changed — `kg search` readability (Amendment 14, P6)

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

### Added — `kg search` docstring vocabulary (Amendment 16, schema v6)

- `symbols.docstring` (additive): one-line documentation captured from the
  already-parsed node (Python docstring first line; nearest leading comment
  for TS/JS/Rust), marker-stripped, cap 160 chars, no bodies (OQ6 approved).
- Indexed text gains `split_ident(docstring)` tokens; G14 recall unchanged
  at 0.800; a frozen 6-query prose-gap set improves 5/6 over the pre-P7
  binary; search hit rendering unchanged.

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
  refactor); `src/journal.rs` added to the kg_dir-scoped allowlist.
- `db::probe_old`: a v4-stamped index from the Amendment 12 first landing
  wave (no `clone_groups` table) now disables the fast path and rebuilds
  fresh instead of failing carry-forward with
  `no such table: olddb.clone_groups`.

### Added — `kg symbols --outline` (Amendment 12a, F1)

- New `--outline` flag on `kg symbols <file>`: each line gains the stored
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


### Added — P4 roadmap amendment (Amendment 12, docs)

- `docs/designs/2026-11-01_p4-agent-tooling_amendment.md`: four agent-tooling
  features distilled from the pi-sherlock reference analysis — F1
  `kg symbols --outline` (signature column), F2 `kg search` (FTS5 symbol
  lookup), F3 skill hardening + drift gate, F4 `kg clones` (shingle
  candidates). Proposal spec-reviewed by an independent stateless Goldfish
  reviewer (round 1 NEEDS FIXES with 12 findings, round 2 APPROVE; record
  `docs/baselines/reviews/R4-P4.md`). OQ4a (signature text) and OQ4b
  (symbol-slice `search` deferral lift) approved by the owner 2026-11-01.
  Gates G13-G16 registered in the spec's Gate Registry. Implementation
  follows (F3 -> F1 -> F2 -> F4), one schema v4 bump for whatever survives
  the gates.

### Added — `kg search` (Amendment 12b, F2)

- New `kg search <terms>`: symbol-level FTS5 lookup — OR of prefix terms over
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
  `docs/baselines/g14-search-2026-11-01.md`.

### Added — `kg clones` (Amendment 12d, F4)

- New `kg clones [--min-lines N]` (default 5): duplicate-block candidates —
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
  (`docs/baselines/g16-clones-2026-11-01.md`).

### Changed — performance ceiling pass (Amendment 11, G12)

Four changes, all measured on a 2,025-file / 7.9 MB synthetic Python corpus
(index 4.6 MB, same seed as the review baseline) plus the kg self-index; gates
`cargo test --locked` (91 tests), `cargo clippy -D warnings`, G2 self
precision/recall PASS.

- Query path: `PRAGMA quick_check` removed from `open_for_query` (full B-tree
  scan, 0.71-1.10 ms/MB measured, ran on every CLI open and every MCP tool
  call). Corruption now exits 3 via SQLITE_NOTADB/SQLITE_CORRUPT mapping with
  the unchanged contract message; `kg doctor` keeps `integrity_check`.
  `kg callers` best-of-7: 15.1 ms -> 7.6 ms; `kg unresolved` 13.9 -> 7.6 ms;
  latency is now flat at spawn floor + query regardless of index size.
- Build inserts: one transaction per build (500-row prepared statement batches
  retained; per-chunk BEGIN/COMMIT fsyncs dropped). Insert phase measured on
  identical row volume: 154 ms -> 55 ms; db phase 0.39 s -> 0.09 s.
- Parse+extract: parallel on `std::thread::scope` workers
  (`available_parallelism`, capped 8; no new deps, no unsafe, order-preserving
  merge). Parse phase 0.66 s -> 0.11 s; `kg build --force` wall 1.16 s ->
  0.27 s on the corpus, 0.43 s -> 0.13 s on kg self.
- Schema v3 (additive): `files.loc`, `files.cyclomatic` computed during the
  build parse phase; `kg hotspots`/`kg report` read the index instead of
  re-parsing every file per call. hotspots 409-781 ms -> 31 ms; report
  560 ms -> 44 ms. Behavior deltas: files deleted from disk are no longer
  silently skipped by hotspots; metrics are frozen at each file's last fresh
  extraction (changed files are always re-extracted by the fast-path rules).

### Added — P0 baseline (f9d57b9..1ca1d0e)

- Frozen A/B baselines: 5 navigation task definitions (T1-T5), 30 verified
  ground-truth call sites on vihap, provisional top-10 pain list.
- D0 harness: `g` call logger (byte/call accounting both directions),
  `count.py`, `check.py` (precision/recall with AMBIGUOUS-candidate and
  dynamic-dispatch rules), derivation scripts, cold-run equivalence check.
- Grep-arm baseline: 12 tool calls / 49,224 bytes; P0 exit criterion passed
  (premise not weak).
- R0 independent review record (APPROVE after 1 fix cycle).

### Added — P1 core (9f75656..e72b9b0)

- `kg` Rust CLI (single binary, 5 pinned deps, `#![forbid(unsafe_code)]`):
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

### Added — P1.5 extensions (d9951ce..5034939)

- `kg stats`, `kg hotspots` (per-metric min-max normalization of
  cyclomatic + LOC + churn-90d + fix-density), `kg report` (4 sections).
- G7 rank-overlap 8/10 vs pain list (degraded: pain list provisional).
- G8 -67% tool calls on architecture questions (via `callees --depth`).
- `code-index` skill wrapper in the agent-skills repo (OQ2).
- R1.5 independent review record (APPROVE after 1 fix cycle).

### Added — P2 multi-language (2aeeb9d..7a99343)

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
  with the old destination ids and `kg callers`/`kg impact` silently lost
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

- Result-payload journal: `.kg/usage-results.jsonl` — one self-contained
  JSONL line per successful pointer query (callers/callees/impact/dead/
  symbols) carrying the displayed set byte-identical to the stdout data
  lines plus index `built_at`/`source_rev`. Non-pointer tools and
  empty/ambiguous/error outcomes write none. Rotation (5 MB), opt-out
  (`KG_NO_JOURNAL` / `--no-journal`), and best-effort contract mirror
  usage.log.
- Session attribution: `KG_SESSION`/`KG_ARM` env tags ride every usage.log
  and payload line as additive fields — R1/R2 A/B attribution per task
  without transcript parsing.
- `kg usage --verify [--limit N]`: deterministic even-stride sample over
  payload entries; the index's staleness check separates CHANGED
  (unverifiable, never counted wrong) from OK/FAIL; unchanged files get
  structural checks (line bounds; `callers` also requires the queried
  name's bare segment on the pointer line). Report-only exit 0; exit 1 on
  an empty payload log. The MCP `usage` tool accepts optional
  verify/limit args (G10 byte parity preserved per invocation).
- `kg usage` payload section: entries/pointers totals and top-5 queried
  names by call count (demand map). `kg doctor` reports payload journal
  volume or its explicit absence.
- G11 gate (payload conformance) added to the Gate Registry: one payload
  line per successful pointer call, byte-equal pointers, verify FAIL=0 on
  fresh builds. Conformance run on the 3 GT repos (kg self / vihap /
  finguard): 594 pointers verified, FAIL=0; byte-parity payload-vs-stdout
  confirmed. Tests 91/91; clippy -D warnings clean.
- Support changes: index meta `built_at` now carries millisecond precision
  (the verify epoch comparison must not collide on same-second rebuilds);
  kg-self GT rows G01/G02/G07/G09/G10 refreshed after the Amendment 10 line
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

- `kg mcp`: newline-delimited JSON-RPC 2.0 server on stdin/stdout; 11 tools
  mapping 1:1 to CLI subcommands (doctor excluded — `--invalidate` stays a
  human-CLI maintenance action). Spawned by the agent host; kg still spawns
  no daemons. Zero network; writes confined to `.kg/`.
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
  `KgError::Ambiguous` instead of being printed mid-query (stdout stays a
  pure protocol/data channel on every surface).
- kg-self GT rows G02-G10 refreshed after the M3 line shifts (provenance in
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
- kg-self GT rows G02-G10 refreshed after M1's line shifts (provenance noted
  in the GT file); G01 unchanged.

### Added — J1 usage journal (Amendment 6)

- Every journaled subcommand appends one JSONL line to `.kg/usage.log`:
  ts, surface, tool, args summary (names/paths/scalars only — never file
  contents), result/shown/total, ambiguous count, exit code, latency, stale
  flag, kg version. Opt-out: `KG_NO_JOURNAL=1` or `--no-journal`. Rotation
  at 5 MB (one prior generation). Best-effort writes — journal failures can
  never alter output or exit codes.
- `kg usage` — journal summary: calls per tool, error/empty/stale rates,
  ambiguous pointer counts, p50/p95 latency. Read-only; no index required;
  exit 1 on an empty journal.
- QueryOutput carries `stale` (header fired on the displayed set) for
  journaling; `unresolved`/`symbols` record their own result counts.

### Added — M1 query surface (Amendment 5)

- `kg dead` — dead-code CANDIDATES (zero inbound call edges, AMBIGUOUS-name
  aware) with a permanent first-line caveat: candidates, never proof.
- `kg unresolved` — per-file unresolved-callee counts, descending; exit 1 on
  full coverage.
- `kg symbols <file>` — one file's symbols as `path:line kind qualname`.
- Resolution fix: `from .x import y` inside a repo-root plain module resolved
  to ".x" (stray leading dot) and the call was misclassified as dropped
  external; root-level relative imports now resolve to the sibling module
  (fixture regression test added).
- Test-fixture fix: the shared fixture constants lost their indentation to
  Rust `\`-line continuations, so class bodies collapsed to top-level defs
  and method resolution was never exercised; fixtures are now raw strings
  (all 63 tests pass against the semantic fixtures).

### Changed — M0 re-baseline (Elephant/Goldfish cycle)

- All measurable gates re-run on the post-fix binary (`docs/baselines/
  rebaseline-m0.md`): G1 0.47s/22.8MB/1.98MB, G2 vihap 29/29 + 493/495,
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
