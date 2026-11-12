# Prevent-Reinvention Phase 1 — Scan Truthfulness, Guards, Staleness, R29

- Author: quy.doan
- Status: Approved (Phase 1a/1b; owner approval 2026-11-12; all 10 Critical findings closed; targeted closure check PASS)
- Date: 2026-11-12
- Predecessor design: `.tmp/prevent-reinvention/design-r4-b.md` (historical evidence only)
- Final audit: `.tmp/prevent-reinvention/audit.md` (`needs-one-more-revision`)
- Gate 2 report: `docs/reports/2026-11-12_goldfish-review_reuse-prevention.md`
- Scope: Phase 1a and Phase 1b only. Phases 2–5 are a gated roadmap in §Roadmap.
- Constraints: local-first, zero network, no schema change, no new dependency, MSRV 1.90, module ≤500 LOC, CLI/MCP byte parity, no hooks/daemons.

## Overview

`eg reuse` is a post-edit duplicate audit with four detectors. Phase 1 fixes four truthfulness defects: an unmatched path can look clean, truncation is invisible, confidence is undefined, and staleness is not shown. It also hardens caller counting, composition guards, and `mem touch`. It does not ship proposal, allowlist, coverage, judge, or the numeric confidence model.

## The Problem

An agent reads `0` and cannot tell whether the repository is clean, the path was wrong, the index is stale, or the limit hid results. Caller counts include test, self, and duplicate edges. `provisional` confidence is not in the CLI/MCP contract. `mem touch` updates `anchored_at` but `mem query` compares against `created_at`, so the stale tag never clears. Phase 1 closes these defects with one status table, one confidence enum, one staleness state machine, and one caller definition.

## Goals

- GOAL-1: A path typo, an existing-but-unindexed path, an absolute path, and an ambiguous target have distinct, tested outcomes.
- GOAL-2: Truncation is visible as `showing N of M`; `total` is pre-truncation; the journal records `truncated`.
- GOAL-3: Phase 1 prints only `provisional` or `low`; `medium` and `high` are rejected by `--min-confidence`.
- GOAL-4: Composition and subset guards have one disposition; `distinctive` has one definition.
- GOAL-5: Staleness is `fresh | stale | unknown`; the warning appears before the empty return.
- GOAL-6: `mem touch` clears `[stale-ref] [modified-ref]`; `mem query` uses the same anchor.
- GOAL-7: Phase 1 performance is gated by one pinned Rust bench.
- GOAL-8: No Phase 1 behavior depends on `.tmp` artifacts or a memo-gated module.

## Non-Goals

- No `--propose`, `--callees`, allowlist, suppression, coverage, D5, D1L, or tests coverage.
- No `--judge`; Phase 5 reintroduces it with a hardened protocol.
- No numeric confidence model, High tier, or oracle gate. Those are Phase 2.
- No schema migration, new dependency, network path, daemon, or hook.
- No change to the D1–D4 detector count.

## Gate Decisions (normative)

1. **Composition.** A resolved call edge in either direction rejects the pair (D4 exempt). An unresolved bare-name-only overlap is `low` with reason `caller-callee`. A strict name-token subset is rejected when `d=0`; when `d>=1` it is `low` with `same-name-subset`.
2. **Confidence.** Phase 1 has exactly two levels: `provisional` and `low`. `medium` and `high` are reserved and rejected by `--min-confidence`.
3. **Distinctive.** `distinctive(c) = project(c) AND NOT static_stoplist(c) AND df_all_excl(c) <= 2`; `d` is the count of shared distinctive callees.
4. **Staleness.** Hash budget is 16 MiB per invocation. States are `fresh`, `stale`, `unknown`; there is no `partial` state.
5. **Judge.** `--judge`, the MCP `judge` parameter, `src/query/judge.rs`, and `tests/cli/reuse_judge.rs` are deleted from Phase 1. Phase 5 reintroduces them only after the trust boundary closes.
6. **R29.** `anchor = max(anchored_at, created_at)` for `mem query`; `mem touch` updates `anchored_at`; `mem write --touch` updates both. Phase 1 has no allowlist/suppression matcher; Phase 3 will use the same anchor.

## Definitions

### Detectors (closed set, D1–D4)

- **D1 Callee footprint overlap.** `C_t = C_in(target)`, `C_c = C_in(candidate)`, `shared = C_t ∩ C_c`. Candidate when `candidate.id != target.id`, the pair is not call-related, `|C_t| >= 3`, `|C_c| >= 3`, `|shared| >= 2`, `containment >= 0.80`, and `bal >= 0.30`.
- **D2 Interface token overlap.** `candidate.id != target.id`; not call-related; name-token Jaccard `J_name`; type-token Jaccard `J_type`; shared name tokens `>= 2`; `J_name >= 0.60` or (`J_name >= 0.40` and `J_type >= 0.50`). Reject same-file overlapping spans and same-file numbered siblings.
- **D3 Raw-client bypass.** `candidate.id != target.id`; not same-file/overlapping; `is_more_canonical(candidate, target)`; target calls a raw client from `RAW_CLIENTS` that the candidate also calls (normalized by `raw_client_eq`); candidate `callers >= 2`. `raw_client` is the matching name.
- **D4 Pass-through wrapper.** `candidate.id != target.id`; `line_end - line_start <= 4`; exactly one distinct resolved callee id and one distinct callee name; `inbound_callers <= 1`; callee is in a different file; same language. The callee is the canonical; `is_more_canonical` is not required.
- **Selection.** D1→D2→D3→D4; one candidate per target. Production keeps first-hit; `censored` is a bench-only replay metric, not a production counter or CLI output.

### Sets and metrics

- `P` = production function/method symbols (`is_test = 0`).
- `C_res(s)` = one-hop project symbols reached by resolved `calls` edges from `s` (`dst_id` non-null).
- `C_in(s)` = `C_res(s)` filtered to `P`, same language family as `s`, and not in the static `STOPLIST`.
- `C_t = C_in(target)`, `C_c = C_in(candidate)`.
- `shared = C_t ∩ C_c` (project symbol IDs only).
- `containment = max(|shared| / |C_t|, |shared| / |C_c|)`; `0` if either set is empty.
- `bal = min(|C_t|, |C_c|) / max(|C_t|, |C_c|)`; D1 requires `|shared| >= 2`.
- `J_name` = Jaccard of `name_tokens` from `tokenize_name_sig`; `J_type` = Jaccard of type tokens. If either name-token set is empty, D2 rejects.
- `callers(s)` = `COUNT(DISTINCT src_id)` over `edges` where `kind='calls'`, `dst_id=s`, `src_id != s`, and `symbols.is_test(src_id) = symbols.is_test(s)`. No site-line dedupe is needed. `inbound_callers = callers(s)`; `CALLER_COUNT_SQL` is updated to this exact query.
- `canonical`: for D1/D2/D3 the candidate with more `callers`, tie-break `(file_path, line_start, qualname)` ascending; for D4 the callee. `canonical_callers = callers(canonical)`.
- `df_all_excl(c)` = one prefetch `SELECT dst_id, COUNT(DISTINCT src_id) FROM edges WHERE kind='calls' AND dst_id IS NOT NULL GROUP BY dst_id` including test callers; it excludes nothing despite the name. Loaded once per scan into a map.
- `project(c)` = `c` has a resolved `dst_id` in `P`.
- `generic(c)` = `c` is in the static `STOPLIST` or `df_all_excl(c) > 2`.
- `distinctive(c) = project(c) AND NOT generic(c)`.
- `d = |{x in shared : distinctive(x)}|`.
- `g_ratio = (|shared| - d) / |shared|`; `0` when shared is empty.
- `--deep-callee`: `C_t`/`C_c` are weighted closures (direct 1.0, second-hop 0.5); `containment` and `bal` use weights; D1 requires weighted `containment >= 0.85`; `d`/`g_ratio` count distinct shared distinctive callees by weight > 0.

### ReasonCode (closed Phase 1 set)

| Variant | Wire string | Emitted when | Visible |
|---|---|---|---|
| `CallerCallee` | `caller-callee` | unresolved bare-name-only composition | candidate |
| `SameNameSubset` | `same-name-subset` | strict name-token subset at `d>=1` | candidate |
| `ZeroName` | `zero-name` | D1 has no shared name token | candidate |
| `SizeImbalance` | `size-imbalance` | `0.30 <= bal < 0.50` | candidate |
| `StaleIndex` | `stale-index` | pair stale | candidate |
| `FreshnessUnknown` | `freshness-unknown` | pair unknown | candidate |
| `UnsafePath` | `unsafe-path` | index path fails containment/symlink check | candidate |
| `BelowThreshold` | `below-threshold` | filtered by `--min-confidence` | summary count only |
| `CrossLanguage` | `cross-language` | cross-language rejection | counter only |
| `NoCanonical` | `no-canonical` | no canonical direction | counter only |

`guarded_out` = sum of `cross-language` + composition rejects + bal rejects + same-file/self/overlap rejects + no-canonical. `censored` is bench-only and not a ReasonCode.

## Critical User Journeys

### CUJ 1: Path status
- Given: an indexed repo.
- When: `eg reuse src/query` / `eg reuse src/query/missing.rs` / `eg reuse /etc/passwd` / `eg reuse ambiguous_name`.
- Then: directory scan with `path_matches` / exit 1 `path not found` / exit 2 `path must be repo-relative and contained` / exit 2 with candidate lines.

### CUJ 2: Truncation
- Given: 40 candidates and `--limit 10`.
- When: `eg reuse src/query --limit 10`.
- Then: `showing 10 of 40`; summary `10 omitted`; `truncated=true`, `shown=10`, `total=40`.

### CUJ 3: Provisional confidence and stale index
- Given: a file changed after the last build.
- When: `eg reuse`.
- Then: the stale warning prints before any empty return; every candidate prints `confidence=provisional` or `confidence=low`; stale/unknown downgrade `provisional→low` with a reason.

### CUJ 4: R29 memory touch
- Given: a note with `[stale-ref] [modified-ref]`.
- When: `eg mem touch <key>`.
- Then: `anchored_at=now`; `anchor=max(anchored_at,created_at)`; `eg mem query` no longer prints the modified stale tag. A `[broken-ref]` remains.

### CUJ 5: Build hint
- Given: an incremental build that re-extracts a changed file.
- When: `eg build`.
- Then: `detect_hints` prints at most three `eg: reuse-hint:` lines, sorted deterministically, without `confidence=`, and skips freshness I/O.

### CUJ 6: Invalid limit on an unbuilt repo
- Given: no index.
- When: `eg reuse --limit 0`.
- Then: exit 2 with `eg: usage: limit must be 1..=100` before any index open.

### CUJ 7: Multiple paths
- Given: two path arguments, one valid and one missing.
- When: `eg reuse src/query src/missing`.
- Then: exit 1 with `path not found: src/missing`; no partial candidate output.

## Technical Plan

### Pipeline

```
CLI/MCP -> ReuseOpts + ResolvedScope validation
  -> run_reuse(ctx, opts, scope)
  -> analyze_corpus_with(ctx, opts, scope) -> ReuseReport { pool, counts, freshness, path_matches }
  -> finalize(ctx, report, opts) -> LineQuery { lines, stderr, shown, mode, truncated }
  -> journal
```

There is no judge and no allowlist. `analyze_corpus_with` returns an unsorted, untruncated pool with per-candidate freshness and fills `total`, `guarded_out`, and `path_matches`. `finalize` owns confidence filtering, sort, truncation, output cap, rendering, and the remaining counts. `analyze_corpus(ctx, path_filter, target_names, limit, deep)` is a compatibility wrapper: it builds a scope from the filters, calls `analyze_corpus_with` with `FreshnessMode::Full`, sorts by the legacy key, and truncates. `detect_hints(ctx, new_names)` calls the internal wrapper with `FreshnessMode::Skip` and limit 3.

### Types and signatures (normative)

```rust
pub enum ReuseConfidence { Low, Provisional, Medium, High } // Medium/High reserved
pub enum Freshness { Fresh, Stale, Unknown }
pub enum FreshnessMode { Full, Skip }
pub enum ReuseMode { Scan } // Propose is Phase 2
pub enum UnknownReason { NewFile, HashBudget, WalkError, UnsafePath, StatError, DeletedFile }
pub enum ScopeKind { Repo, Files(Vec<String>), Symbols(Vec<i64>) }

pub struct ResolvedScope { pub kind: ScopeKind, pub path_matches: usize }

pub struct ReuseOpts {
    paths: Vec<String>,
    limit: usize,                    // 1..=100
    deep_callee: bool,
    min_confidence: ReuseConfidence, // Low | Provisional
    explain: bool,
    freshness_mode: FreshnessMode,
}

impl ReuseOpts {
    pub fn new(paths: Vec<String>, limit: usize, deep_callee: bool,
               min_confidence: ReuseConfidence, explain: bool) -> Result<Self> {
        if !(1..=100).contains(&limit) {
            return Err(EgError::Usage("limit must be 1..=100".into()));
        }
        if matches!(min_confidence, ReuseConfidence::Medium | ReuseConfidence::High) {
            return Err(EgError::Usage("min-confidence must be low|provisional".into()));
        }
        Ok(Self { paths, limit, deep_callee, min_confidence, explain,
                  freshness_mode: FreshnessMode::Full })
    }
    pub fn for_hints(paths: Vec<String>) -> Self {
        Self { paths, limit: 3, deep_callee: false,
               min_confidence: ReuseConfidence::Low, explain: false,
               freshness_mode: FreshnessMode::Skip }
    }
}

pub struct CheckResult { pub state: Freshness, pub reason: Option<UnknownReason> }

pub struct FreshnessReport {
    pub state: Freshness,
    pub changed_files: usize,
    pub checked_files: usize,
    pub unknown_count: usize,
    pub unknown_reason: Option<UnknownReason>,
}

pub struct ReuseCounts {
    pub shown_provisional: usize,
    pub shown_low: usize,
    pub guarded_out: usize,
    pub below_threshold: usize,
    pub omitted: usize,   // limit-omitted + capped
    pub capped: usize,    // subset of omitted
    pub shown: usize,     // shown_provisional + shown_low
    pub total: usize,     // guard-passing pool before confidence/limit
    pub truncated: bool,  // omitted > 0
}

pub struct ReuseReport {
    pub pool: Vec<ReuseCandidate>,
    pub counts: ReuseCounts,
    pub freshness: FreshnessReport,
    pub path_matches: usize,
}

pub struct ReuseCandidate {
    pub detector: DetectorId,
    pub symbol: String,
    pub file: String,
    pub line: i64,
    pub canonical: String,
    pub canonical_file: String,
    pub canonical_line: i64,
    pub canonical_callers: i64,
    pub overlap_pct: Option<u64>,
    pub shared_items: Vec<String>,
    pub raw_client: Option<String>,
    pub confidence: ReuseConfidence,
    pub reasons: Vec<ReasonCode>,
    pub freshness: Freshness,
    pub mode: ReuseMode,
}
```

`LineQuery` gains `mode: ReuseMode` and `truncated: bool`; `LineQuery::new(lines, stderr, shown)` sets `Scan/false`. Update all nine literal sites: `src/query.rs:234`, `src/query/reuse.rs:144,170`, `src/query/analysis/symbols.rs:76,133`, `src/query/analysis/unresolved.rs:59,161`, `src/query/callgraph/context.rs:112`, `src/query/callgraph/path.rs:88`. Add an `rg 'LineQuery \{'` guard outside `query.rs`.

`analyze_corpus_with(ctx: &QueryCtx, opts: &ReuseOpts, scope: &ResolvedScope) -> Result<ReuseReport>` and `finalize(ctx: &QueryCtx, report: ReuseReport, opts: &ReuseOpts) -> Result<LineQuery>` are the only Phase 1 entry points. `analyze_corpus(ctx, path_filter, target_names, limit, deep) -> Result<(Vec<ReuseCandidate>, usize)>` preserves the legacy return for dashboard and hints.

Counts ownership: `analyze_corpus_with` sets `total = pool.len()` (after guards, before confidence/limit), `guarded_out`, and `path_matches`. `finalize` sets `below_threshold`, `shown_provisional`, `shown_low`, `shown`, `omitted`, `capped`, `truncated`. `shown = shown_provisional + shown_low`; `omitted = eligible - shown` where `eligible = total - below_threshold`; `capped` is the subset of `omitted` cut by the 30 KiB cap; `truncated = omitted > 0`.

### Target grammar and status table

Parse order per argument:

1. Empty string → exit 2 `path must be non-empty`.
2. Absolute, `..`, backslash, or symlink → exit 2 `path must be repo-relative and contained`.
3. `<file>:<positive line>` where the prefix is an indexed file → symbol containing the line; no symbol → exit 1 `no symbol at <input>`.
4. Existing file: indexed → file scope; not indexed → exit 1 `path is not indexed: <input>; run eg build`.
5. Existing directory: at least one indexed file under the prefix boundary → directory scope; none → exit 1 `no indexed files under path: <input>`.
6. Exact or unique suffix qualname, or unique bare name → symbol scope.
7. More than one symbol match → exit 2 with candidate lines.
8. Otherwise: input contains `/` or `.` → exit 1 `path not found: <input>`; else exit 1 `symbol not in index: <input>`.

| Input | Disk | Index | Exit | stdout / stderr |
|---|---|---|---|---|
| no paths, index exists | — | — | 0 | repo scope; `path_matches=0` |
| no paths, no index | — | — | 1 | stderr `eg: no-results: no index; run eg build` |
| file | exists | indexed | 0 | file scope; `path_matches=1` |
| file | exists | not indexed | 1 | stderr `eg: no-results: path is not indexed: <input>; run eg build` |
| file | missing | — | 1 | stderr `eg: no-results: path not found: <input>` |
| absolute / `..` / `\` / symlink | — | — | 2 | stderr `eg: usage: path must be repo-relative and contained: <input>` |
| dir | exists | ≥1 indexed | 0 | prefix scope; `path_matches=N` |
| dir | exists | 0 indexed | 1 | stderr `eg: no-results: no indexed files under path: <input>` |
| file:line | file exists | symbol | 0 | symbol scope; `path_matches=1` |
| file:line | file exists | no symbol | 1 | stderr `eg: no-results: no symbol at <input>` |
| file:line | file missing/unindexed | — | 1 | stderr `eg: no-results: path not found/not indexed: <input>` |
| qualname/bare | — | 1 match | 0 | symbol scope; `path_matches=1` |
| qualname/bare | — | 0 match | 1 | stderr `eg: no-results: symbol not in index: <input>` |
| qualname/bare | — | >1 match | 2 | stderr `eg: usage: ambiguous name '<input>'`; stdout candidate lines |
| any | — | limit 0 or >100 | 2 | stderr `eg: usage: limit must be 1..=100` |
| any | — | tracked `.engram` | 2 | stderr `eg: security: tracked .engram index rejected` |
| any | — | internal error | 6 | stderr `eg: internal: <detail>` |

- Directory prefix boundary: `src/query` matches `src/query/**`, not `src/query2/**`.
- Multiple paths: validate all first; any invalid path aborts with no partial candidates. Valid paths form a union; `path_matches` is the number of unique indexed files in the union (symbol targets count 1; overlapping targets dedupe).
- MCP: exit 0 text is stdout; exit 1/2/6 text is stderr; `is_error=true` for nonzero. Ambiguous MCP text is stderr + `\n` + stdout candidate lines.

### Output grammar (Phase 1)

Streams:

- **stdout:** result header, summary, freshness warning, candidate lines, cap footer, ambiguous candidate lines.
- **stderr:** error headers/messages (`eg: usage:`, `eg: no-results:`, `eg: security:`, `eg: internal:`) and the ambiguous header.

```
eg: reuse — {shown} duplication candidate(s) found
eg: reuse: {p} provisional, {l} low, {g} guarded out, {b} below threshold[; {o} omitted][; {c} capped][; showing {shown} of {total}][; path_matches={k}]
eg: {changed} of {checked} result files changed since build — run eg build
eg: freshness unknown ({reason}) — run eg build
```

- `{o} omitted` prints when `omitted > 0`; `{c} capped` prints when `capped > 0`; `showing` prints when `omitted > 0`; `path_matches` prints when `paths` is non-empty.
- Empty valid scope prints exactly `eg: reuse — 0 duplication candidate(s) found`; the freshness warning follows if applicable. There is no separate empty literal.
- Candidate lines:
  - D1/D2: `symbol '{target}' shares {pct}% {noun} with existing '{canonical}' ([callers={n}] in {file}:{line}) [confidence={level}]` plus `  {shared label}: [{items}]`.
  - D3: `symbol '{target}' bypasses canonical client '{canonical}' to call raw external client {raw} ([callers={n}] in {file}:{line}) [confidence={level}]`.
  - D4: `symbol '{target}' is a pass-through wrapper around existing '{canonical}' ([callers={n}] in {file}:{line}) [confidence={level}]`.
- `{noun}` is `callee footprint` for D1 and `interface token overlap` for D2; `{shared label}` is `shared callees` for D1 and `shared tokens` for D2. D3/D4 have no shared-items line.
- `--explain` appends ` detector={D#}; reasons=[{reason1},{reason2}]` to the first line; reasons are sorted by wire string; empty is `reasons=[]`.
- Output cap: 30 KiB of stdout bytes. Stop before a candidate block (candidate line + shared-items line) would exceed the cap; append `eg: output capped at 30 KiB; narrow the path or raise the limit`; increment `capped`, `omitted`; `truncated=true`. The cap runs after the `--limit` truncation; the limit and cap omissions are disjoint and both count in `omitted`.
- Sanitize every index-derived and echoed user string: escape `\` as `\\` first, then `\n`→`\\n`, `\r`→`\\r`, `\t`→`\\t`, ESC→`\\x1b`, C0/C1/DEL→`\\xNN`. The same representation is used for CLI, MCP, dashboard JSON, and journal candidate IDs.

### Guards

- **GUARD-LANG.** Families: Python (`.py`, `.pyi`), TsJs (`.ts`, `.tsx`, `.js`, `.jsx`, `.mjs`, `.cjs`), Rust (`.rs`). `Other` compares the exact lowercase extension. Reject with `cross-language`.
- **GUARD-COMP.** Resolved call edge either direction rejects (D4 exempt). Unresolved bare-name-only overlap is `low` with `caller-callee`. A strict name-token subset is rejected when `d=0`; when `d>=1` it is `low` with `same-name-subset`.
- **GUARD-CALLERS.** `callers(s)` exactly as defined; `inbound_callers = callers(s)`; `CALLER_COUNT_SQL` uses the same query. Fixture: self edge, duplicate edge, test caller; expected `N` asserted on both `search` and `reuse`.
- **GUARD-NAME.** D1 with no shared name token is `low` with `zero-name`; it is not rejected.
- **GUARD-BAL.** Applies only to D1. `bal < 0.30` rejects; `0.30 <= bal < 0.50` is **low** with `size-imbalance`; `bal >= 0.50` is normal. The detector-local `min_len * 2 < max_len` check is removed.
- **GUARD-KIND.** The corpus is already restricted to production `function`/`method`; no separate counter exists.

### Confidence

Base confidence before guards/freshness:

| Detector | Provisional | Low | Reject |
|---|---|---|---|
| D1 | `candidate != target`; GUARDs pass; `containment >= 0.80` (`>=0.85` deep); `|C_t|,|C_c| >= 3`; `|shared| >= 2`; `d >= 1`; `g_ratio <= 0.75`; `bal >= 0.50` | same D1 gates with `bal >= 0.30` and (`d=0` or `g_ratio > 0.75` or `0.30 <= bal < 0.50`) | `containment < 0.80` (`<0.85` deep); `|C_t| < 3`; `|C_c| < 3`; `|shared| < 2`; `bal < 0.30` |
| D2 | `candidate != target`; GUARDs pass; shared name tokens `>= 3`; Jaccard condition | shared name tokens `= 2`; Jaccard condition | empty name tokens; shared `< 2`; Jaccard fails |
| D3 | `candidate != target`; GUARDs pass; raw-client match; canonical `callers >= 2` | none (base) | otherwise |
| D4 | `candidate != target`; GUARDs pass; wrapper predicate | none (base) | otherwise |

Final level algorithm:

1. Base level from the table.
2. Guard-forced Low: `zero-name`, `caller-callee`, `same-name-subset`, `size-imbalance` set Low regardless of base.
3. Freshness: `provisional → low` for `stale` or `unknown`; `low` stays `low`.
4. Final level = the lowest of base, guard, and freshness.
5. `reasons` lists every applicable candidate-visible reason, sorted by wire string.

- `--min-confidence` default `low`; accepts `low|provisional`; `medium|high` exit 2. A custom CLI parser accepts only the two spellings; MCP validates the string before dispatch.
- Sort: confidence rank (`provisional > low`), then `sort_rank` (`overlap_pct`, D3=90, D4=100, descending), then `canonical_callers` descending, then `symbol` ascending.
- `--min-confidence provisional` filters `low` into `below_threshold`; it does not change exit status.

### Staleness

- `FreshnessChecker { ctx, remaining_bytes: u64 }` with `check(file: &str) -> CheckResult`.
- `check` order:
  1. `safe_index_path` fails or `symlink_metadata` is a symlink → `Stale` reason `UnsafePath`.
  2. `symlink_metadata` NotFound: if a `files` row exists → `Stale` reason `DeletedFile`; if no row → `Fresh` (not on disk, not indexed).
  3. Other stat error → `Stale` reason `StatError`.
  4. No `files` row but file exists → `Unknown` reason `NewFile`.
  5. Size differs → `Stale`.
  6. mtime differs: if `remaining_bytes < size` → `Unknown` reason `HashBudget`; else read with the 1 MiB cap, subtract size, hash; mismatch → `Stale`; match → `Fresh`.
  7. mtime and size match → `Fresh`.
- New-file walk: `walk` only `ResolvedScope` with repository ignore rules; exclude `.git`, `.engram`, `target`, `.tmp`; supported extensions; no symlink following. A file absent from `files` sets `Unknown` reason `NewFile`; walk error sets `Unknown` reason `WalkError`.
- Checked file set: when the pool is non-empty, the unique target/canonical files of the pool; when empty, the indexed files in `ResolvedScope`, up to the 16 MiB budget. `checked_files` = unique files with `Fresh` or `Stale`; `unknown_count` = unique unknown files; `changed_files` = unique `Stale` files.
- Pair aggregate: `Unknown` if either file unknown, else `Stale` if either stale, else `Fresh`. Reason precedence: `WalkError > NewFile > HashBudget > UnsafePath > StatError > DeletedFile`. Only affected pairs are downgraded; the aggregate header is informational.
- Header prints before the empty return. `stale`: `eg: {changed} of {checked} result files changed since build — run eg build`. `unknown`: `eg: freshness unknown ({reason}) — run eg build`.
- `FreshnessMode::Skip` (build hints) does not walk, stat, or hash; candidates are `Fresh`.

### R29 memory touch

- `MemRow` adds `anchored_at: i64`; `MemRow::anchor() = max(anchored_at, created_at)`.
- `MEM_ROW_SQL` becomes `fn mem_row_sql(conn: &Connection) -> Result<String>`: probe `PRAGMA table_info(memories)`; if `anchored_at` exists, select it as the last column; else `0 AS anchored_at`. `mem_row` reads it at the final index. Probe failure → `EgError::Internal`.
- Update `memory_write.rs`, `memory_query.rs`, `memory_tests.rs`, and every `MemRow` literal; if the working tree contains the uncommitted `src/service/explain.rs`, update its literal too.
- `check_ref_status(conn, row, ref)` uses `row.anchor()` for the mtime comparison. `check_note_ref_status` returns `Broken` if any ref is broken, else `Modified` if any ref is modified, else `Valid`.
- `[stale-ref]` is emitted for `Modified` or `Broken`; `[modified-ref]` for `Modified`; `[broken-ref]` for `Broken`.
- `mem touch` executes `UPDATE memories SET anchored_at = ?1 WHERE id = (current revision id)`; `mem write --touch` executes `UPDATE memories SET created_at = ?1, anchored_at = ?1 WHERE id = (current revision id)`. Because `created_at` is the recency/expiry ranking key, `--touch` intentionally moves the note to the top; this is documented in `docs/architecture/memory-store.md`.
- A modified ref clears after touch; a broken ref remains broken. Phase 1 has no allowlist/suppression matcher; the dashboard/audit anchor display uses `anchor()`.

### CLI and MCP

- CLI: `eg reuse [PATHS]... [--limit N] [--deep-callee] [--min-confidence low|provisional] [--explain]`.
- `--limit` default 10, range 1..=100; 0 or >100 exits 2 before opening the index. This narrows the previous CLI max 10,000; it is a breaking change recorded in CHANGELOG.
- MCP `reuse` params: `paths` (StrArray), `limit` (1..=100, default 10), `deep_callee` (Bool), `min_confidence` (Str: `low|provisional`, default `low`), `explain` (Bool). No `judge`.
- `--judge`, `--propose`, `--callees`, `--coverage`, `--show-suppressed` are absent.
- MCP: exit 0 text = stdout; exit 1/2/6 text = stderr; `is_error=true` for nonzero; ambiguous error text = stderr + `\n` + stdout candidate lines.

### Journal and telemetry

- `LineQuery { mode: ReuseMode, truncated: bool }`; `LineQuery::new` defaults `Scan/false`.
- `journal::set_reuse_counts(ReuseJournalCounts { shown, total, truncated, provisional, low, guarded_out, below_threshold, omitted, capped })`; `ReuseJournalCounts` lives in `src/journal/reuse.rs`.
- `result_count = shown`; `total` is recorded, not a KPI numerator. `reuse_hits += shown.unwrap_or(result_count)`; `UsageFields.shown` becomes `Option<i64>` so an old absent key is distinguishable from zero. Old lines: `mode=scan`, `truncated=false`.
- `set_mode(ReuseMode)` serializes `as_str()`.
- `estimated_tokens_saved` uses `shown`.
- `judged` remains an inert legacy journal field (always false in Phase 1); `set_judged` and the dashboard judge badge are removed.
- No proposal/uptake fields in Phase 1.

### Security

- `db::open_for_query` calls `reject_tracked_eg_dir` before opening the index; a tracked `.engram` index returns `EgError::Security` (exit 2) with `eg: security: tracked .engram index rejected`. The check may run `git ls-files`; this is not a judge process and is cached per invocation.
- Every index-derived path used for a read passes `safe_index_path`; a failure or symlink marks the pair `Stale` with `unsafe-path`, not a process exit.
- User-supplied absolute/`..`/backslash/symlink paths exit 2 as usage errors.
- `sanitize_field` in `src/query/reuse/finalize.rs` applies the escape map from §Output to every index-derived and echoed field on CLI, MCP, dashboard JSON, and journal candidate IDs.
- `src/query/judge.rs` is deleted; no judge process spawns in Phase 1.

### Performance

- Harness: `tests/bench/reuse_bench.rs`, release binary, `EG_BENCH_BIN` default `target/release/eg`, records `eg --version` and `git rev-parse HEAD`.
- Fixtures: generated 5k and 10k symbol repos; 16 files of 1 MiB each for the 16 MiB hash wall; each bench test is `#[ignore = "release bench"]`.
- Method: 10 warmups + 30 measured runs; p95; same host; no journal fold. Deep-callee baseline is the same binary/fixture/limit without the flag, 30 runs, median.
- Gates: 5k p95 ≤200 ms; 10k p95 ≤500 ms; hash wall ≤25 ms at 16 MiB; `--deep-callee` median ≤1.10× baseline.
- The old 100 ms gate is replaced by the 200/500 ms gates; no mixed-version baseline. A `cargo test --test reuse_bench -- --list` guard ensures the bench tests are present.

### Compatibility

- Existing v10 indexes remain readable; no schema change.
- `CALLER_COUNT_SQL` and reuse caller counts change together; `search` and `reuse` agree; CHANGELOG records the change.
- Empty output changes from `0 reuse candidates found` to `eg: reuse — 0 duplication candidate(s) found`; tests/docs update.
- `--judge`/MCP `judge` removal and CLI `--limit` max 100 are breaking changes; CHANGELOG marks them.

## Alternatives Considered

### A. Build-hint-first
- Pros: existing agent-facing build hint; fewer new surfaces.
- Cons: incremental-only; cannot answer an explicit path; no status/truncation contract.
- Rejection: build hints stay (CUJ 5) but do not replace the scan contract.

### B. Full design in one spec
- Pros: preserves the full scope.
- Cons: Gate 2 found Critical blockers in proposal, allowlist, coverage, judge, oracle, and the detailed plan.
- Rejection: Phase 1 alone; Phases 2–5 are gated roadmap specs.

### C. Phase 1a only
- Pros: smallest slice.
- Cons: leaves confidence/summary undefined.
- Rejection: 1a+1b is the smallest coherent user-facing contract.

## Non-Functional Requirements

- Security: no network; no judge; tracked-dir rejection; `safe_index_path`; one sanitizer.
- Performance: one pinned Rust bench; 5k/10k/hash/deep-callee gates.
- Compatibility: v10 index; CLI/MCP parity for shared ranges; output/flag changes documented.
- Rollback: 1b cannot be cut without re-scoping GOAL-3; 1a remains.

## Phase 1 Critical Closure

| # | Critical finding | Resolution in this spec |
|---|---|---|
| 1 | Cap/count partition unresolved | `ReuseCounts` has `total`, `below_threshold`, `omitted`, `capped`, `shown`, `truncated`; exact owners and `total = shown + below_threshold + omitted`; candidate-block atomic cap. |
| 2 | `check` cannot return `unknown_reason` | `CheckResult { state, reason }`; `UnknownReason` enum; `FreshnessReport.unknown_count`/`unknown_reason`. |
| 3 | Freshness scope/aggregation/empty warning | Checked file set, pair aggregate, reason precedence, scope-limited walk, empty-scope check before empty return. |
| 4 | Caller semantics undefined | Exact SQL, `src_id != s`, same `is_test`, no site-line dedupe, `inbound_callers = callers(s)`, search/reuse fixture. |
| 5 | `C_res`/`C_in`/raw client undefined | One-hop `C_res`, `C_t=C_in(target)`, `C_c=C_in(candidate)`, `|C_t|,|C_c|>=3`, project-only `shared`, raw client only D3. |
| 6 | Guard precedence undefined | Final level = min(base, guard, freshness); guard-forced Low; reasons list all. |
| 7 | Streams/MCP parity undefined | Per-message stdout/stderr table; success stdout, errors stderr, exit 6 `is_error`, ambiguous text specified. |
| 8 | Raw client/wrapper/canonical/self-match | `RAW_CLIENTS`/`raw_client_eq`; `candidate != target`; D3 exclusions; D4 callee exception; `raw_client` field; canonical/tie-break. |
| 9 | Unsafe-path dispositions | User input exit 2; index path `Stale` `unsafe-path`; tracked `.engram` `Security` exit 2; no judge process. |
| 10 | Self-match | Explicit `candidate.id != target.id` in all detectors; same-file/overlap and numbered-sibling exclusions. |

Important implementation notes: nine `LineQuery` literal sites; `src/service/explain.rs` only if present in the uncommitted working tree; no `GUARD-KIND` counter; `censored` bench-only; `df_all_excl` one prefetch; `MEM_ROW_SQL` probe/column order; MCP default limit 10; journal `shown: Option<i64>`; docs `getting-started.md`/README/usage/reuse module/install.sh stale-claim removal.

## Roadmap (Phases 2–5, separate specs)

No Phase 2–5 code starts until its own spec closes the deferred findings and its gate is approved.

### Deferred findings (not closed by this spec)

| Phase | Findings | Reason |
|---|---|---|
| Phase 2 — proposal + oracle | C-P0-06, C-P0-07, CONS-P0-02, C-P1-01, C-P1-02, C-P1-06, C-P1-07, C-P1-10, C-P1-23, C-P1-24, C-P1-26, C-P1-27, C-P1-28, C-P1-29, C-P1-30, C-P1-41, C-P1-42, C-P1-43, C-P1-44, C-P1-45, C-P1-50 | Proposal, numeric confidence, uptake, and oracle are not in Phase 1. |
| Phase 3 — allowlist | C-P1-16, C-P1-17 | Allowlist is not in Phase 1. |

- **Phase 2 — Proposal + oracle.** Indexed-target proposal; 160-label oracle (50 tune, 60 confirmation positives = 30 eligible + 15 D4 + 15 D3, 30 negatives, 20 separate holdout); SAP with McNemar power/MDD; numeric Medium/High.
- **Phase 3 — Allowlist.** Opt-in suppression; `anchor()` freshness; MCP confirm gate; no High suppression.
- **Phase 4 — Coverage.** D5 classes, D1L unresolved, tests stopset; default off; owner R53-R56 confirmation.
- **Phase 5 — Judge.** eg-judge v2 with stdin, containment, sanitized env, process-group kill, verdict whitelist; `--judge` restored.

## Detailed Implementation Plan

### File ledger

**Create**

| Path | Phase | Change | Rationale |
|---|---|---|---|
| `src/query/reuse/types.rs` | 1a | Types, enums, `CheckResult`, `ResolvedScope`, `ReuseCounts` | Keep `reuse.rs`/`detectors.rs` under 500 |
| `src/query/reuse/status.rs` | 1a | Target parsing, status table, `path_matches`, multi-path validation | One address grammar |
| `src/query/reuse/guards.rs` | 1a | GUARD-LANG/COMP/CALLERS/NAME/BAL; self/overlap exclusions | One guard pass |
| `src/query/reuse/finalize.rs` | 1a | Filter, sort, truncate, 30 KiB cap, `sanitize_field`, rendering | One finalize owner |
| `src/query/reuse/confidence.rs` | 1b | Provisional mapping, final-level algorithm, reasons | One confidence source |
| `src/journal/reuse.rs` | 1a | `ReuseJournalCounts`, `set_mode`, `set_reuse_counts` | Keep `journal.rs` under 500 |
| `tests/cli/reuse_phase1.rs` | 1a/1b | Phase 1 tests; split into `reuse_phase1/{path,confidence,memory}.rs` if >480 lines | One declared test module |

**Modify (HEAD baseline)**

`src/query/reuse.rs`, `src/query/reuse/detectors.rs`, `src/query/reuse/corpus.rs`, `src/query/reuse/tokens.rs`, `src/query/staleness.rs`, `src/query.rs`, `src/query/analysis/symbols.rs`, `src/query/analysis/unresolved.rs`, `src/query/callgraph/context.rs`, `src/query/callgraph/path.rs`, `src/db/query_state.rs`, `src/query/analysis/search.rs`, `src/db.rs`, `src/db/security.rs`, `src/cli.rs`, `src/service.rs`, `src/service/from_cli.rs`, `src/service/args.rs`, `src/service/run.rs`, `src/service/tools.rs`, `src/mcp.rs`, `src/mcp/format.rs`, `src/journal.rs`, `src/journal/render.rs`, `src/journal/tokens.rs`, `src/db/memory_query.rs`, `src/db/memory_write.rs`, `src/db/memory_tests.rs`, `src/service/mem_query.rs`, `src/service/mem_ops.rs`, `src/memory.rs`, `src/memory/guard.rs`, `src/dashboard/data/reuse.rs`, `src/dashboard/data/summary.rs`, `src/dashboard/data/memories.rs`, `src/dashboard/assets/app.js`, `src/audit/memory.rs`, `src/build/plan.rs`, `tests/cli/main.rs`, `tests/cli/reuse.rs`, `tests/cli/reuse_deep.rs`, `tests/cli/global_usage.rs`, `tests/cli/dashboard.rs`, `tests/mcp/parity.rs`, `tests/mcp/memory_touch.rs`, `tests/bench/reuse_bench.rs`, `docs/cli-reference.md`, `docs/usage.md`, `docs/getting-started.md`, `docs/agent-integration.md`, `docs/architecture/memory-store.md`, `README.md`, `skills/engram-first/SKILL.md`, `scripts/install.sh`, `CHANGELOG.md`.

**Delete**

`src/query/judge.rs`, `tests/cli/reuse_judge.rs`; remove `pub mod judge;` from `src/query.rs`.

**Working-tree note:** If the uncommitted memory-parity refactor is present, also update `src/service/explain.rs`, `src/service/request.rs`, and `src/db/memory_v2_tests.rs` for `MemRow`/request literals. These files are not part of HEAD and are not required by this spec otherwise.

### Phase 1a — Guards, staleness, R29, scan grammar, security, journal

1. Add `types.rs`, `status.rs`, `guards.rs`, `finalize.rs`; move types out of `reuse.rs`/`detectors.rs`.
2. Implement target parsing/status, `ResolvedScope`, multi-path validation, `path_matches`, exact messages.
3. Implement GUARD-LANG/COMP/CALLERS/NAME/BAL; remove detector-local size check; add self/overlap/sibling exclusions.
4. Implement `FreshnessChecker`/`CheckResult`, per-candidate freshness, 16 MiB budget, scope-limited walk, empty-scope check, warnings.
5. Add `LineQuery::new`, `mode`/`truncated`; update all nine literal sites.
6. Add `journal/reuse.rs`; update render/tokens/mcp/dashboard consumers with `shown: Option<i64>` fallback.
7. Add `reject_tracked_eg_dir` to `open_for_query`; add `sanitize_field`; delete judge module/flag/tests.
8. R29: `MemRow.anchored_at`, `mem_row_sql` probe, `anchor()`, active-revision touch/write SQL, all call sites.
9. Remove `--judge` from CLI/MCP/request/run/tools; update docs/skill/install.
10. Tests/bench: status, truncation, guards, staleness, R29, security, journal, 5k/10k/hash.

### Phase 1b — Provisional confidence, summary, routing

1. Add `confidence.rs`: base table, final-level algorithm, reason emission, `--min-confidence` filter.
2. Add confidence/summary lines and `--explain` details.
3. Add dashboard JSON `confidence`/`reasons`; CLI/MCP parity.
4. Update routing/skill: “review only; confidence is provisional; no action tier”.
5. Tests: truth table, reason enum closure, stale downgrade, summary counts, MCP parity, dashboard JSON.
6. Exit: all Phase 1 Critical findings closed; no `medium`/`high` on Phase 1 surfaces.

### Test matrix

Fixtures: a temp repo with `a.py`/`b.py` duplicates, a `src/query`-like tree with a renamed duplicate, a self-call edge, a duplicate call edge, a test caller, a broken ref note, a modified ref note, a 5k/10k generated corpus, and 16 × 1 MiB files. Each test asserts exact stdout/stderr/JSON, not only exit codes.

| Test | File | Command | Expected |
|---|---|---|---|
| `scan_path_typo_exits_one` | `tests/cli/reuse_phase1.rs` | `cargo test --locked --test cli -- --exact reuse_phase1::scan_path_typo_exits_one` | exit 1, exact stderr |
| `scan_path_not_indexed_exits_one` | same | same pattern | exit 1 |
| `scan_absolute_path_exits_two` | same | same pattern | exit 2 |
| `scan_symlink_path_exits_two` | same | same pattern | exit 2 |
| `scan_file_line_resolves_symbol` | same | same pattern | exit 0 |
| `scan_file_line_no_symbol_exits_one` | same | same pattern | exit 1 |
| `scan_bare_ambiguous_exits_two` | same | same pattern | exit 2 + candidate lines |
| `scan_bare_zero_match_exits_one` | same | same pattern | exit 1 |
| `scan_multiple_paths_abort_on_invalid` | same | same pattern | exit 1, no candidates |
| `scan_limit_zero_exits_two_before_index` | same | same pattern | exit 2 |
| `scan_limit_101_exits_two` | same | same pattern | exit 2 |
| `scan_truncation_shows_n_of_m` | same | same pattern | `showing 10 of 40`; counts |
| `scan_output_cap_30k` | same | same pattern | cap footer, `capped`, atomic block |
| `scan_empty_literal` | same | same pattern | `eg: reuse — 0 duplication candidate(s) found` |
| `scan_confidence_only_provisional_low` | same | same pattern | no medium/high |
| `scan_min_confidence_medium_rejected` | same | same pattern | exit 2 |
| `scan_stale_warning_before_empty` | same | same pattern | warning |
| `scan_missing_files_row_unknown` | same | same pattern | unknown `new-file` |
| `scan_deleted_file_stale` | same | same pattern | stale `deleted-file` |
| `scan_hash_budget_unknown` | same | same pattern | unknown `hash-budget` |
| `scan_composition_resolved_rejects` | same | same pattern | no pair |
| `scan_composition_unresolved_low` | same | same pattern | low + reason |
| `scan_subset_d0_rejects_d1_low` | same | same pattern | low + reason |
| `scan_caller_count_distinct` | same | same pattern | exact N on search/reuse |
| `scan_bal_boundaries` | same | same pattern | 0.29 reject, 0.30 low, 0.49 low, 0.50 provisional |
| `scan_self_match_rejected` | same | same pattern | no self pair |
| `scan_sanitizes_echoed_input` | same | same pattern | escapes |
| `mem_touch_clears_modified_stale` | same | same pattern | no modified stale tag |
| `mem_touch_keeps_broken_ref` | same | same pattern | broken remains |
| `mem_write_touch_updates_both` | same | same pattern | both timestamps |
| `mcp_reuse_phase1_parity` | `tests/mcp/parity.rs` | `cargo test --locked --test mcp -- --exact parity::mcp_reuse_phase1_parity` | success/error text |
| `bench_scan_5k_p95` | `tests/bench/reuse_bench.rs` | `EG_BENCH_BIN=target/release/eg cargo test --locked --test reuse_bench -- --ignored --exact bench_scan_5k_p95` | ≤200 ms |
| `bench_scan_10k_p95` | same | same, exact name | ≤500 ms |
| `bench_hash_16mib` | same | same, exact name | ≤25 ms |
| `bench_deep_callee_ratio` | same | same, exact name | ≤1.10× baseline |

### Verification

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked --lib query::reuse
cargo test --locked --lib query::staleness
cargo test --locked --test cli -- --list | grep -q '^reuse_phase1::'
cargo test --locked --test cli reuse_phase1
cargo test --locked --test mcp -- --list | grep -q 'mcp_reuse_phase1_parity'
cargo test --locked --test mcp parity
cargo build --release
EG_BENCH_BIN=target/release/eg cargo test --locked --test reuse_bench -- --list | grep -q 'bench_scan_5k_p95'
EG_BENCH_BIN=target/release/eg cargo test --locked --test reuse_bench -- --ignored
cargo test --locked
git add -N src/query/reuse/types.rs src/query/reuse/status.rs src/query/reuse/guards.rs src/query/reuse/finalize.rs src/query/reuse/confidence.rs src/journal/reuse.rs tests/cli/reuse_phase1.rs
bash tests/scripts/allowlist_check.sh Cargo.lock
bash tests/scripts/write_path_check.sh
bash tests/scripts/hygiene_check.sh
rg -n 'calibrated|--judge|eg-judge|Sub-20ms|< 3ms|3ms' README.md docs skills src/query/reuse.rs scripts/install.sh || true
```

The `--list | grep` guards prevent zero-test green. `git add -N` includes new files in the hygiene scan.

### Module budget

- `src/query/reuse.rs` 290 → ≤390: facade only; types/status/guards/finalize moved out.
- `src/query/reuse/detectors.rs` 429 → ≤480: remove size check, call guards, no rendering.
- `src/query/reuse/types.rs` 120–180; `status.rs` 180–240; `guards.rs` 180–240; `finalize.rs` 180–240; `confidence.rs` 180–240.
- `src/journal.rs` 490 → ≤500: only `mod reuse;`; setters in `journal/reuse.rs` 160–220.
- `src/journal/render.rs` 470 → ≤490; `src/mcp.rs` 480 → ≤490; `src/dashboard/data/reuse.rs` 436 → ≤490; `src/db.rs` 456 → ≤480.
- `tests/cli/reuse_phase1.rs` target ≤480; split into a directory module if over. Do not add tests to the 474-line `tests/cli/reuse.rs`.

### Rollout and rollback

- 1a ships first; 1b ships after 1a is green.
- 1b cannot be cut without re-scoping GOAL-3; 1a alone does not claim confidence truthfulness.
- Rollback is a binary revert; no schema/journal migration. Old journal lines read `shown=result_count`, `mode=scan`, `truncated=false`.
- `CALLER_COUNT_SQL`, empty output literal, `--limit` max 100, and `--judge` removal are documented in CHANGELOG as intentional changes.

## Open Questions

None. The ten Critical findings are closed in §Phase 1 Critical Closure; deferred findings are in §Roadmap.
