# Changelog

All notable changes to kg are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning is
[Calm SemVer](https://github.com/calver?ad=calver) 0.x — breaking contract
changes bump the minor while 0.x is unstable.

## [Unreleased]

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

- P3 stdio MCP server: trigger condition ("CLI discoverability measurably
  fails") not met. Re-evaluable on request.

[Unreleased]: no tagged releases yet — history starts at f9d57b9 (repo init).
