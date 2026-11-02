# R9 — P8 Implementation Review (Inheritance, Imports, Tests, Metrics, Latency, Utility)

- Target: P8 worktree at `/Users/quy.doan/Workspace/kg` (binary
  `target/release/kg` v0.2.1, schema v6).
- Date: 2026-11-02.
- Reviewer set: six independent stateless Goldfish subagents, one lens each,
  two concurrent per wave. No reviewer received author rationale or another
  review's findings.
- Lenses: inheritance/dead accuracy; import-graph accuracy; test-impact and
  unresolved accuracy; metrics/API/generated accuracy; latency/query plans;
  agent utility/contract/safety.
- Verdict: **NOT AT CEILING — NEEDS ACCURACY AND LATENCY FIXES.** The current
  test suite is green, but most new tests are `contains`-based and do not
  catch the defects below.

## 1. What is at the ceiling

| Surface | Evidence | Verdict |
|---|---|---|
| Self-index query latency | 5-7 ms per query on 412 symbols | pass; spawn floor dominates |
| `imports`/`implementers`/`resolve` on self index | low-ms range | pass |
| `code-index` skill budget | 2,905 / 3,000 bytes | at ceiling (96.8%) |
| Default result cap | 50; `hotspots` max 100 | intentional ceiling |
| Static coverage | 10-48% resolved call sites; import/inherit coverage unmeasured | documented ceiling, not a defect |
| Schema additive/carry design | v6 columns, partial-index probe, `metrics_version` | sound; tests need expansion |

Everything else below is below the intended bar.

## 2. Critical findings by feature

### A. `dead` regressed to O(symbols²) and takes 15.6 s on the 2,025-file corpus

- Source: `src/query.rs:1474-1508`.
- Cause: the new trait/interface parent exclusion is a correlated
  `NOT EXISTS` with `substr(s.qualname, ...) = p.qualname || '.'`; no index
  can serve it, so every candidate scans all symbols.
- Measured: `kg dead` 71 ms -> **15.57 s** on the 2,025-file corpus;
  1M symbols COUNT **>120 s** (killed). `EXPLAIN`: correlated `SCAN p`.
- Fix: materialize trait/interface qualnames once, use a range prefix test,
  or filter in Rust. The reviewer measured **95 ms** on the same corpus.

### B. Reverse BFS uses unbounded `IN` lists and fails hard at scale

- Source: `src/query.rs:105-110` (`test_impact`), `1391-1403`
  (`level_impact`).
- Cause: one placeholder list per frontier; no 500-id chunking, no reuse of
  the existing `path_bfs` chunking.
- Measured: 500k fan-in -> `kg: internal: database error: too many SQL
  variables`, with **977 KB** of placeholder SQL in the error. 990k-edge
  DAG: `impact --depth 3` **364 ms** (budget 150 ms), `tests --depth 10`
  **496 ms** (budget 200 ms).
- Fix: chunk by 500 ids, reuse one visited set, dedupe R1 in SQL, avoid
  per-edge String allocation.

### C. `complexity` is 60x over budget and materializes every callable

- Source: `src/query.rs:1586-1597`.
- Cause: no SQL `LIMIT`; all rows formatted/sorted then truncated. The pinned
  ORDER BY (`qualname, f.path, line_start`) exceeds the index
  (`qualname, file_id, line_start`), so SQLite builds a temp B-tree.
- Measured at 1M symbols: **3.0 s / 126 MB**, `--limit 1` **2.92 s**,
  `--path` **2.2 s**; budgets 50/100 ms. EXPLAIN: temp B-tree for last two
  order terms.
- Fix: add `LIMIT` + COUNT; reconcile index/order (denormalize path or accept
  and re-baseline); add the EXPLAIN gate.

### D. Import edges are duplicated, parent-package edges are missing, and resolved imports remain unresolved

- Source: `src/extract.rs:1132-1241`, `2060-2111`;
  `src/query.rs:215-219`.
- Symptoms:
  - `from X import Y` emits module + module-qualified-symbol + symbol
    candidates; two resolve to the same destination. `kg imports` prints
    duplicate rows; `impact --edges imports` inflates `(N sites)`.
  - Star imports emit both `EXTRACTED` and `AMBIGUOUS` edges to the module.
  - `import a.b` emits only `a.b`, not parent `a`.
  - A module candidate miss writes an unresolved row even when the symbol
    candidate resolves; the row survives gap healing.
  - Self-imports (`pkg/selfimp.py` importing its own module) create self
    edges.
- Fix: resolve all candidates for one statement, emit one edge per
  `(site, destination)`, module-only matching for `Module`, parent-prefix
  edges for dotted imports, unresolved only when none resolve, dedupe/unique
  `(src_id,dst_id,kind,site_line)`.

### E. `kg api` does not implement F10; `exported` is never populated

- Source: `src/query.rs:1705-1766`; `src/extract.rs:239`.
- Symptoms: Python `__all__` ignored; Rust `pub(crate)`/private and TS local
  symbols listed as public; `symbols.exported=1` count is 0 on every fixture.
  `--include-private` only toggles the underscore filter.
- Fix: populate `exported` during extraction with the pinned language rules
  and make `api` read it; otherwise relabel the command as a name-prefix
  inventory and remove the public-surface claim.

### F. `kg tests` reports module/class symbols as tests and includes seed tests

- Source: `src/query.rs:53-56`, `107-135`.
- Symptoms: a module named `test_service` is reported as a test; a test
  symbol in the target file is reported even though it is a seed; unittest
  names `test`, `test1`, `testing` are missed by the pinned rule.
- Fix: require `s.kind IN ('function','method')`; skip seeds when recording;
  widen the unittest prefix or caveat it; add exact fixtures.

### G. `report --path` and `report --exclude-generated` crash

- Source: `src/insights.rs:232-235`, `298-304`.
- Cause: section-4 query joins `files df` but the injected scope predicate
  uses alias `f`.
- Evidence: both flags exit 6 with `no such column: f.path` /
  `f.generated`; zero stdout.
- Fix: build the predicate against `df`, add byte tests.

### H. `resolve` diverges from its contract

- Source: `src/cli.rs:204-205`; `src/main.rs:532-546`;
  `src/mcp.rs:495-507`.
- Symptoms: no `--stdin` flag (documented); blocks on terminal stdin;
  no 1,000-name/4,096-byte/1 MiB caps; empty input exits 0; journal args
  empty and lost on non-zero exit; MCP always `isError:false`.
- Fix: add `--stdin`, enforce caps, `no names on stdin` exit 1, journal
  `count`/`candidate_limit`, propagate batch error to MCP.

## 3. Important findings

| Area | Finding | Evidence |
|---|---|---|
| Inheritance | `collect_heritage` flattens every generic/path leaf into a base; `Base<Arg>` creates `Arg` edge; TS mixin uses every argument; `std::error::Error` creates `std`/`error` edges; `dead` then hides real symbols | reviewer fixtures; `implementers std` returns `AppError` |
| Inheritance | Rust impl target matched by bare name; cross-file impl can attach to a same-named local type; tuple/pointer impls create false edges; duplicate generic impls duplicate edges | `impl T for (A,B)` -> `A -> T`; `impl T for Foo<u32>` twice |
| Inheritance | AMBIGUOUS inherits/unresolved inherit rows are not implemented; ambiguous base silently picks first candidate; `dead` has no AMBIGUOUS imports/inherits arms | dual `Base` fixture; `unresolved --names` empty |
| Inheritance | Lexical scope ignored: namespace/inline-mod bases and qualified TS bases are missed; `mod.rs` modules invisible | `namespace ns { class Base; class Child extends Base }` zero edges |
| Inheritance | Rust impl-trait pairs enter `class_bases` and change call resolution, contrary to F1 | `callers default_hi` changes when an unrelated impl is added |
| Inheritance | TS interface method signatures are never extracted | `interface I { run(): void }` has no `run` symbol |
| Inheritance | `dead --exclude-tests` omits filename globs (`*_test.py`, `*.spec.ts`, etc.) | `test_util.py` and `util.spec.ts` remain |
| Inheritance | Empty `implementers` omits its caveat on stdout/MCP | exit 1 with empty stdout |
| Imports | TS `export from`, `export *`, dynamic `import()`, CommonJS `require` are not extracted; `require` pollutes unresolved calls | four fixtures return 0 imports; `unresolved` has `call require` |
| Imports | Rust grouped/as/self/super/wildcard/single-segment use misresolved; `mod.rs` path wrong; aliased calls unresolved | `use crate::helper::{h,Thing}` -> unresolved `src`, `src.helper` |
| Imports | `.mts/.cts/.mjs/.cjs` and `index.*` module paths not handled; extension precedence collapses to first module | explicit `.tsx` import binds `.ts` module |
| Imports | Deleting an imported target drops carried edges and writes no unresolved row; restoring the file does not heal | `kg imports app.py` returns 0 after target deletion and restore |
| Tests/unresolved | Default `unresolved` distinct/top-name counts include import rows, while `total` is call-only; `--path` distinct count unscoped | `unresolved=1 ... 8 distinct names` |
| Tests/unresolved | `unresolved --names` has no staleness check, hardcodes `stale=false`, materializes all rows | edited-file probe; 200k rows 207 ms/38 MB |
| Tests/unresolved | `site_line=0` has no build error/stderr count; default and `--names` disagree | injected ghost row |
| Metrics | `effective_loc` ignores docstrings/block comments; Rust `non_comment_loc` misses `line_comment`/`block_comment` | Python loc=11 vs pinned 8; Rust loc=26 vs 24 |
| Metrics | Python `match_statement` double-counted with `case_clause`; phantom Rust kinds still in `decision_kinds`; `generator_function` leaks | `cyc=4` vs pinned 3; generator body counted |
| Metrics | Trait signature methods appear in `complexity` with cyc=1/loc=1 | `trait.T.required` listed |
| Metrics | `complexity`/`api`/`files`/`unresolved --names`/`symbols <dir>` materialize unbounded sets; `symbols <dir>` is N+1 | 1.74 s/190 MB; 94-112 ms vs 50 ms |
| API | `api` ignores signatures; `--include-private`/`--exclude-generated`/`--path` inconsistent; `.`/`..` invalid paths exit 1 | `api .` exit 1; no signature |
| Generated | marker scan caps at 4096 bytes; `This file was generated` tags hand-written prose; Dart suffixes active before P9 | `/tmp` fixtures |
| Report | section 1 still counts generated files under `--exclude-generated`; `report` default bytes changed via `stats generated=` without a contract row | code evidence |
| Latency | `dead` O(S²); BFS unbounded; `complexity` 60x; `symbols <dir>` N+1; `api`/`files`/`unresolved` unbounded | §2/§3 measurements |
| Contract | MCP `tool_journal_args` misses every new tool and many flags; required args missing for new tools; `symbols`/`tests` file-or-dir schemas incomplete | MCP journal probes |
| Contract | `--path` validation absent; empty/`.`/`..`/absolute/backslash exit 1 not 2 | probes |
| Contract | Empty caveats missing for `implementers`, `complexity`, `api`, `files`; `impact` empty strings/depth footer deviate | probes |
| Utility | `search --exact` lowercases input via `split_ident`, matches module `base` instead of class `Base`; qualified exact falls through to FTS | `kg search Base --exact` |
| Utility | Skills/docs overclaim `api` public surface, `tests` test candidates, `search --exact`; consumer skill lines from §10.3 absent | agent-skills grep |
| Tests | Gate names `p8a_`, `p8c_`, `tests_impact_seed`, `resolve_batch` match zero tests; GT JSONL/recall/timing harness absent; tests are contains-based | `grep`, `ls tests/scripts` |

## 4. Latency ceiling table

| Surface | Shape | Measured | Budget | Verdict |
|---|---|---|---|---|
| `dead` | 2,025 files | 15.57 s | none | regressed ~100x |
| `complexity` | 1M symbols | 3.0 s / 126 MB | 50 ms | 60x |
| `symbols <dir>` | 2,025 files | 94-112 ms | 50 ms | ~2x |
| `impact --depth 3` | 990k edges | 364 ms | 150 ms | 2.4x |
| `tests --depth 10` | 990k edges | 496 ms | 200 ms | 2.5x |
| `api` | 1M symbols | 1.35 s / 114 MB | none | unbounded |
| `unresolved --names` | 200k rows | 207 ms / 38 MB | none | unbounded |
| `files` | 2,025 files | 10 ms | 50 ms | pass |
| Self-index queries | 412 symbols | 5-7 ms | 20-50 ms | pass |

## 5. What must change before "best in utility, accuracy, latency"

1. Fix `dead` trait-parent predicate (restores the default command).
2. Deduplicate import edges, remove false unresolved rows, add parent-prefix
   edges, and fix Rust use-tree/TS require/export-from.
3. Fix inheritance leaf flattening, lexical scope, ambiguous/unresolved
   inherits, and the `class_bases` regression.
4. Add SQL `LIMIT` + 500-id chunking to complexity/BFS/list queries; fix the
   complexity ORDER BY/index.
5. Enforce `tests` kind filter; add unresolved staleness, caps, path
   validation, and empty caveats.
6. Implement `api` visibility or withdraw the public-surface claim.
7. Add the gate harness and exact-byte negative controls; current green tests
   do not cover these defects.

The largest utility win is not a new command: it is removing false confidence
from `api`, `tests`, and import counts, then restoring `dead` latency.

## 6. Fix pass applied (2026-11-02)

The review findings above were addressed in the worktree:

| Finding | Fix | Evidence |
|---|---|---|
| `dead` O(S²) | Materialized trait/interface qualnames + range prefix test; AMBIGUOUS imports/inherits arms; filename globs | 2,025-file synth corpus `dead` **0.14 s** (was 15.57 s) |
| BFS unbounded IN | 500-id chunking in `test_impact` and `level_impact`; shared visited set | code; hard-fail path removed |
| `complexity` 60x budget | SQL LIMIT + COUNT; ORDER BY aligned to the index | 85k-symbol synth `--limit 1` **0.01 s**; EXPLAIN uses `symbols_complexity_idx`, no temp B-tree |
| Import duplicates/false unresolved | Module candidate matches only `kind='module'`; one edge per destination/site; parent-prefix edges; unresolved reconciled against resolved edges; star/self skipped | `imports` fixture prints one `pkg.mod.b`; `unresolved --names` exit 1 after a resolved import |
| Heritage leaf flattening | `collect_heritage` stops at outer type nodes; generic args/scoped paths/mixins handled | `Base<T>` no longer creates `T`; `std::error::Error` no leaf edges |
| Lexical/ambiguous/unresolved inherits | `resolve_base_chain` tries enclosing qualnames; AMBIGUOUS/unresolved `inherit` rows; Rust impl edges split from `class_bases` | namespace/inline-mod/interface probes; unresolved `MissingBase` row |
| Rust impl target | Full qualname match; tuple/pointer targets rejected | cross-file/nested probes |
| `tests` kind/seed | `kind IN ('function','method')`, seed skip, `./` normalization, 500 chunking | module `test_service` exit 1 |
| `report` alias crash | Section 4 scope uses `df`; generated filter | `report --path/--exclude-generated` test pass |
| API visibility | `exported` populated per language; `api` reads it; signature column; repo-root `.` | Python `__all__`, Rust `pub(crate)`, TS local probes |
| Resolve contract | `--stdin`, caps, journal count/candidate_limit, MCP `isError` | CLI/MCP code; caps enforced |
| Search exact | Untokenized, case-sensitive exact path | `search Base --exact` returns class, not module |
| List surfaces | SQL LIMIT + COUNT for complexity/api/files/unresolved names; symbols dir one query | 85k synth `symbols synth --limit 50` **0.02 s** |
| Path/empty caveats | Shared validation (exit 2); `NoResultsCaveated` on empty implementers/complexity/api/files | code |
| Metrics/generated | Docstring/block-comment LOC; Rust comment kinds; match double-count removed; trait signatures zero; marker scan not 4 KB-capped | code |
| MCP args | Required args and journal args mirrored for new tools/flags | code |

Verification after the fix pass: `cargo fmt --check` clean,
`cargo clippy --all-targets --locked -D warnings` clean,
`cargo test --locked` 23 unit + 75 CLI + 26 fixture + 6 MCP pass,
release build succeeds. `tests/scripts/timing_assert.sh` now exists and asserts a best-of-N
millisecond budget. Remaining scope: full GT sets and recall scripts
(`import_recall.py`, `tests_recall.py`, JSONL) and the `api`
re-export/TS default edge cases are still follow-ups.
