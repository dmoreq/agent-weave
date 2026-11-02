# kg P8 Amendment Proposal — Structural Graph, Test Impact, and Symbol Quality

- Status: **Proposed (rev 4)** — six-lens Goldfish review verdict
  **NEEDS FIXES**; all Critical findings are fixed in this revision, and the
  open Important findings are recorded in
  `docs/baselines/reviews/R8-P8.md`. Owner approval pending. Not normative
  until merged as Amendment 19 in
  `docs/designs/2026-10-31_kg-rust-code-index.md`.
- Date: 2026-11-02
- Canonical base: Amendment 18 (R7 hardening, including the grouped `impact`
  default and `--sites`), Gate G22, and **schema v5**. Amendment 16
  (docstring search) was attempted and reverted; G20 failed its Stage 2c
  utility leg. This proposal does not depend on docstrings.
- Canonical target after approval: Amendment 19, Gate Registry G23-G28,
  schema v6. Post-review optimization: Amendment 20 (§6.1) adds schema v7
  `symbols.file_path` and the path-ordered complexity index.
- ID note: F4 extends Amendment 18(d)'s grouped `impact` contract; it does
  not re-claim it. The next free canonical IDs are Amendment 19 and
  G23-G28.
- Scope: F1-F11 for this amendment. F12 is a P9 charter, approved separately
  after a P9 addendum pins the grammar and node kinds; it is not part of
  this amendment's implementation approval. F13 (revision-aware index) and
  F14 (JSON output) are explicitly deferred; they are not silently dropped.
- Evidence source: the fixture and index probes in §2, the installed
  `agent-skills` set at `~/Workspace/personal/agent-skills`, and the current
  kg worktree. No third-party tool was cloned for this proposal.

## 1. Problem

kg answers call-graph questions. The agent-skills set asks several structural
questions adjacent to that graph. Some have no query surface; some have a
surface that is incomplete for the skill's decision.

| Question | kg today | Consequence |
|---|---|---|
| who inherits/implements X | no command; `callers` filters `kind='calls'` | `kg callers Base` returns 0 although `inherits` edges exist |
| what depends on this file | `impact` defaults to one row per caller file over `calls` only | import-only and inheritance-only consumers are invisible |
| is X dead | `dead` checks inbound `calls` and AMBIGUOUS call names only | inherited base classes and imported contracts are reported dead |
| which tests exercise X | no command | skills fall back to manual pytest/cargo selection |
| which functions exceed CC/nesting/length | `hotspots` is file-level | `code-review` cannot answer its per-function gate |
| what is exported | no visibility model | `code-review` and `codebase-contracts` use grep/read |
| which files are generated | no tag | Flutter/codegen files pollute `dead`, `clones`, `hotspots` |
| Dart/Flutter questions | no Dart grammar | seven Flutter skills plus `dart-modern-idioms` cannot use kg |

The goal is to make kg answer these questions with the existing honesty
rules: static edges only, deterministic pointers, explicit caveats, bounded
cost, and no new safety-invariant violations.

## 2. Evidence base

| # | Type | Measurement | Result | Command / source |
|---|---|---|---|---|
| E1 | measured | Four-file Python fixture: `Base <- Child`, `Base <- Uses`, one `from a import` consumer, one plain `import a` consumer | `kg stats`: `files=4 symbols=11 edges=14 (calls=2 imports=3 inherits=2)`. `kg callers Base` -> exit 1, `0 callers found`. `kg dead` -> `a.py:1` (Base), `a.py:5` (Child). `kg impact a.py` -> `b.py:4`, `d.py:5`; `c.py` (`import a`) and the `from a import` lines are absent | throwaway fixture, binary `target/debug/kg`, schema v5 worktree |
| E2 | measured | Self-index edge distribution | `files=38 symbols=414 edges=1294 (calls=914 imports=4 inherits=0)` at the review snapshot. Rust `use`/impl coverage is too sparse for inheritance/import questions | `KG_NO_JOURNAL=1 kg stats` on `~/Workspace/kg`, v0.2.1 |
| E3 | measured | Plain `import x` handling | `src/extract.rs:1811-1812` comment: "Plain `import x` emits no edge; resolution happens at call time." Only symbol imports and star imports emit `imports` edges | `src/extract.rs:1809-1826` |
| E4 | measured | Skill budget and drift gate | `skills/code-index/SKILL.md` is exactly 3000 bytes, the gate budget. The gate lives in the agent-skills repository at `tests/skill_contract_drift.py`; it passes because it checks command/flag existence only, and it skips when `KG_BIN` is unset | `wc -c`; `KG_BIN=target/debug/kg python3 tests/skill_contract_drift.py` from `~/Workspace/personal/agent-skills` |
| E5 | measured | Skill requirements that name structural checks | `code-review` uses `git grep` for callers/dangling refs; `code-simplification` requires callers; `deprecation-migration` requires internal callers, `__all__`, sibling repos; `codebase-contracts` walks directories; `python-patterns` requires module boundaries and exports; `python-testing` requires affected tests | `skills/code-review/SKILL.md:83`; `skills/code-review/references/goldfish-code-review-lenses.md:91-93,205-209`; `skills/code-simplification/SKILL.md:36-42`; `skills/deprecation-migration/SKILL.md:40,55-57`; `skills/codebase-contracts/SKILL.md:42-47`; `skills/python-patterns/SKILL.md:37-39`; `skills/python-testing/SKILL.md:36-38` |
| E6 | measured | Language support | Grammar string `py-0.25.0+ts-0.23.2+js-0.25.0+rs-0.23.3`; no Dart | `kg stats` |
| E7 | measured | Unresolved data model | Schema v5 stores `unresolved_calls(file_id, dst_name)` with no site line or kind. `kg unresolved` is name-level (distinct names, internal count, top-10 names), not per-file counts only | `schema.sql:56-60`, `src/query.rs:913-990` |
| E8 | measured | Existing performance envelope | Queries 5-11 ms best-of-7 on the self index; builds 0.06-0.46 s on 27-596-file repos and 2.33 s on the 2,025-file corpus | `docs/EVALUATION.md` |
| E9 | measured | Canonical base | Amendment 18 is R7 hardening with Gate G22; Amendment 17 is `kg context`/G21; schema is v5; Amendment 16 was reverted after G20(iii) failed | canonical spec `:96-97,339-340`; `src/extract.rs:21`; `schema.sql:1` |
| E10 | measured | Existing CLI surfaces that P8 must not re-propose | `dead` already has `--exclude-tests` (default includes tests); `impact` already groups one row per caller file with a site count and `--sites` restores per-site rows; `kg context` already returns definition + callers + callees; `search` already handles qualified input and exact single-token names | `src/cli.rs:57-105`; `src/query.rs:584-833`; canonical Amendment 18(d,f) |
| E11 | measured | Existing metric decision kinds | `decision_kinds` uses Python `if_statement`, `for_statement`, `while_statement`, `boolean_operator`, `elif_clause`, `except_clause`, `match_statement`, `case_clause`, `conditional_expression`; TS/JS adds `for_in_statement`, `do_statement`, `switch_case`, `ternary_expression`; Rust includes two kinds that do not exist in tree-sitter-rust 0.23.3 (`if_let_expression`, `while_let_expression`). F9 pins the per-symbol subset and drops the phantom kinds. | `src/parse.rs:90-123`; tree-sitter-rust node-types |
| E12 | measured | Carry-forward and partial-version guards | `carry_forward_unchanged` copies explicit column lists for `files`, `symbols`, `edges`, `clone_groups`, `unresolved_calls`; `probe_old` checks the version, `clone_groups`, `grammar_version`, and `signature_version` | `src/db.rs:156-210,426-474` |
| E13 | measured | Current source extension allowlist | `py ts tsx js jsx mts cts mjs cjs rs`; no Go, no Dart | `src/walk.rs:16-19` |

## 3. Scope and phases

| Phase | Features | Schema | Gate |
|---|---|---|---|
| P8a | F1 inheritance surface, F2 impact caller context, F3 path scope/inventory, F4 transitive impact | v6 additive (introduced here; columns populated across P8a-P8c) | G23 |
| P8b | F7 module import graph, F8 test impact, F6 batch resolve/search filters | uses v6, no new columns | G24, G25, G27 |
| P8c | F5 unresolved names, F9 per-symbol metrics, F10 public API, F11 generated tagging | uses v6, no new columns | G26 |
| P9 | F12 Dart/Flutter | none (uses the current schema) | G28 |
| Deferred | F13 revision-aware index, F14 JSON output | — | — |

P8a lands first because it corrects wrong answers with existing data. P8b
adds the dependency and test layers. P8c populates the remaining v6 columns.
P9 is a separate language phase and must not block P8a-P8c. The v6 bump is
introduced once in P8a so P8a/P8b can write `unresolved_calls.kind` and
`site_line`; P8c adds no second bump.

### 3.1 Command matrix

| Command | Change | Phase | Feature |
|---|---|---|---|
| `kg implementers <NAME>` | new | P8a | F1 |
| `kg dead` | predicate change; existing `--exclude-tests` extended to symbol-level | P8a | F1 |
| `kg impact` | `--depth`, `--edges`, `--context`, `--explain`; existing default grouping and `--sites` preserved | P8a | F2/F4 |
| `kg dead`, `clones`, `hotspots`, `unresolved`, `report` | `--path` | P8a | F3 |
| `kg files` | new | P8a | F3 |
| `kg symbols <dir>` | directory input | P8a | F3 |
| `kg resolve` | new, `--stdin` | P8b | F6 |
| `kg search` | `--kind`, `--path`, `--exact` | P8b | F6 |
| `kg imports <FILE>` | new | P8b | F7 |
| `kg tests <NAME\|FILE>` | new | P8b | F8 |
| `kg unresolved` | `--names`, `--path`; schema v6 `site_line` + `kind` | P8c | F5 |
| `kg complexity` | new | P8c | F9 |
| `kg symbols` | `--metrics` | P8c | F9 |
| `kg api <FILE\|DIR>` | new | P8c | F10 |
| `kg stats` | `generated=N` | P8c | F11 |
| Dart extraction | no new command | P9 | F12 |

## 4. Common contract for all new surfaces

1. **Read-only.** Every query writes only `.kg/usage.log` and
   `.kg/usage-results.jsonl` when the journal is enabled. No source writes, no
   hooks, no daemon, no network, no child processes.
2. **Pointer shape.** A pointer is `repo-relative/path:line`, 1-based.
   Symbol-identity results append ` qualname`; edge-kind results append
   ` kind` only when `--explain` is set. `dead` keeps its existing bare
   `path:line` shape. Line grammars are per feature; the feature section is
   authoritative for its surface. Default output of existing commands stays
   byte-identical unless a contract bump in §4.10 says otherwise.
3. **Caps.** Caps are per command, not global:

   | Command | Default | Max | Final line |
   |---|---|---|---|
   | `callers`, `callees`, `search`, `symbols`, `dead`, `clones`, `imports`, `implementers`, `api`, `tests`, `files`, `complexity` | 50 | 10000 | `showing N of M` only when `N < M` |
   | `context` | 10 | 50 | section headers `callers (showing N of M):` / `callees ...` |
   | `hotspots` | 10 | 100 | standard cap only when capped |
   | `path` | no limit | — | chain line only |
   | `unresolved` | 50 files | 10000 | standard cap only when capped |
   | `unresolved --names` | 50 rows | 10000 | `showing N of M rows` only when `N < M` |
   | `report` | fixed sections | — | no cap line |

4. **Staleness.** The post-cap result-set staleness check applies to pointer
   queries. `path`, `context`, `unresolved` (default), `report`, and `files`
   keep their current behavior: `path`/`context` carry the header;
   `unresolved`/`report`/`files` are report shapes with no staleness pass.
   `unresolved --names` prints source pointers and runs the staleness check
   on the displayed rows. New pointer surfaces print
   `caveat, then staleness header (if any), then data`, matching `dead`.
5. **Empty results.** Exit 1 with the full rendered lines in §4.9; the
   `kg: no-results:` prefix is emitted exactly once, never prepended twice.
   Ambiguous symbol input exits 2 with the existing candidate list. Corrupt
   index, lock, schema skew, and internal errors keep exit codes 3-7.
6. **Determinism.** Every ordering is pinned in the feature section.
   Parallel and serial builds must produce identical query output for an
   identical index.
7. **MCP parity.** Every new CLI query gets an MCP tool with the schema in
   §4.8 and byte-identical result text (G10). MCP carries `isError` and the
   CLI exit code; `resolve` is the only tool whose success text can carry a
   non-zero exit (see F6).
8. **Journal and payload.** The tables in §4.7 are normative. New pointer
   queries join the Amendment 10 payload list; report shapes do not. Any
   deviation is recorded.
9. **Path scope.** `--path <prefix>` matches `path == prefix` or
   `path.starts_with(prefix + "/")`; a file path matches exactly. Normalize
   one trailing `/`; reject empty, `.`, `..`, absolute paths, and `\` with
   exit 2. Positional directory arguments (`symbols <dir>`, `api <DIR>`)
   use the same containment rule against indexed paths; they never walk the
   filesystem outside the repo.
10. **Contract bumps.** Every changed default, new command, new flag, new
    caveat, or new line shape is listed in §4.10. A change not listed there
    is a spec defect.

### 4.7 Journal and payload table

| Surface | Journal `args` | Payload |
|---|---|---|
| `implementers` | `name`, `limit` | yes: rendered symbol pointers |
| `impact` | `file`, `sites`, `depth`, `edges`, `context`, `explain`, `limit` | yes: rendered data lines; `shown`/`total` are caller-file counts in default mode, site counts with `--sites` |
| `dead` | `exclude_tests`, `exclude_generated`, `path`, `limit` | yes |
| `files` | `path`, `limit` | no (file inventory, not pointers); recorded deviation |
| `symbols <dir>` | `file` (dir), `outline`, `limit` | yes: symbol data lines only; file and dir headers excluded |
| `imports` | `file`, `limit` | yes |
| `tests` | `name`, `depth`, `limit` | yes |
| `resolve` | `count`, `candidate_limit` (bounded count, never names) | no; recorded deviation |
| `search` | `terms`, `kind`, `path`, `exact`, `limit` | yes |
| `unresolved --names` | `path`, `names`, `limit` | no (pointer mode but report-shaped; recorded deviation) |
| `complexity` | `path`, `min_cyc`, `min_loc`, `min_branches`, `min_nest`, `limit` | yes: rendered lines are `path:line` pointers; bounds-only verify |
| `api` | `path`, `include_private`, `exclude_generated`, `limit` | yes |
| `stats` | unchanged | no |

### 4.8 MCP schema table

| Tool | Args | Required | Defaults |
|---|---|---|---|
| `implementers` | `name` string, `limit` integer 1-10000 | `name` | `limit=50` |
| `impact` | `file` string, `sites` bool, `depth` integer 1-10, `edges` string, `context` bool, `explain` bool, `limit` 1-10000 | `file` | `depth=1`, `edges="calls"`, `limit=50` |
| `dead` | `exclude_tests` bool, `exclude_generated` bool, `path` string, `limit` | none | `limit=50` |
| `clones` | `min_lines` integer, `exclude_generated` bool, `path` string, `limit` | none | `min_lines=5`, `limit=50` |
| `hotspots` | `exclude_generated` bool, `path` string, `limit` 1-100 | none | `limit=10` |
| `report` | `path` string, `exclude_generated` bool | none | — |
| `files` | `path` string, `limit` | none | `limit=50` |
| `symbols` | `file` string (file or directory), `outline` bool, `metrics` bool, `limit` | `file` | `limit=50` |
| `imports` | `file` string, `limit` | `file` | `limit=50` |
| `tests` | `name` string (symbol or indexed file path), `depth` integer 1-10, `limit` | `name` | `depth=10`, `limit=50` |
| `resolve` | `names` array of strings, max 1000 | `names` | — |
| `search` | `terms` string, `kind` string, `path` string, `exact` bool, `limit` | `terms` | `limit=50` |
| `unresolved` | `names` bool, `path` string, `limit` | none | `limit=50` |
| `complexity` | `path` string, `min_cyc` integer, `min_loc` integer, `min_branches` integer, `min_nest` integer, `exclude_generated` bool, `limit` | none | `min_cyc=1`, others 0, `limit=50` |
| `api` | `path` string, `include_private` bool, `exclude_generated` bool, `limit` | `path` | `limit=50` |

Amended existing tools keep their current schemas plus the new optional
fields. `stats` gains no argument.

### 4.9 No-results and caveat strings

| Surface | Full rendered empty/no-result line | Caveat |
|---|---|---|
| `impact` | `kg: no-results: 0 impacting files found` (calls); add `(unresolved imports are omitted)` / `(unresolved bases are omitted)` when those edges are selected | `kg: impact — resolved static edges only; unresolved imports/inherits are omitted` when selected; for calls-only `--depth > 1`/`--context`, `kg: impact — resolved static call edges only; dynamic dispatch may hide dependents` |
| `dead` | `kg: no-results: 0 dead-code candidates found` | `kg: dead-code candidates — dynamic dispatch, entry points, star imports, module-level imports, and external callers are not proven absent` |
| `implementers` | `kg: no-results: 0 implementers found (resolved inherits edges only)` | `kg: implementers — resolved static inherits edges only; dynamic inheritance and cross-file impls are omitted` |
| `imports` | `kg: no-results: 0 imports found (external and unresolved imports are omitted)` | `kg: imports — resolved static imports only; external and unresolved imports are omitted` |
| `tests` | `kg: no-results: 0 test candidates found (static call edges only)` | `kg: test candidates — static resolved call edges only; Rust #[test] names outside test_*, TS describe/it callbacks, fixtures, and indirect tests may be missed` |
| `files` | `kg: no-results: 0 indexed files in '<path>'`; without `--path`: `kg: no-results: 0 indexed files` | `kg: files — build-time inventory; files created after the build are absent` |
| `symbols <dir>` | `kg: no-results: 0 symbols in '<dir>' (indexed)` | none |
| `complexity` | `kg: no-results: 0 symbols match the thresholds` | `kg: complexity — static AST proxy, not cognitive complexity; not comparable to hotspots/file cyclomatic` |
| `api` | `kg: no-results: 0 exported symbols in '<path>'` | `kg: api — static export heuristics; re-exports, module visibility, and dynamic exports may be incomplete` |
| `unresolved --names` | `kg: no-results: 0 unresolved names (external/builtin calls are not counted)` | none |

For candidate-style commands, the caveat prints on stdout even when the
result is empty, matching `dead`/`clones` `NoResultsCaveated` behavior.
`impact` prints its caveat only when `--edges` includes imports or inherits,
and also on an empty result.

### 4.10 Contract-bump table

| Command | Phase | Delta |
|---|---|---|
| `impact` | P8a | new flags; default output unchanged; `--sites` restricted to `--depth 1`; `--explain`/`--context` tags; caveat when `--edges` includes imports/inherits |
| `dead` | P8a | predicate adds inbound `imports`/`inherits` and the trait/interface-parent exclusion; `--exclude-tests` stays default-off but its predicate expands to `tests`/`test`/`__tests__` segments, filename globs, and `test_*` names; output may shrink; caveat changes |
| `implementers` | P8a | new command |
| `files` | P8a/P8c | new command; P8c appends ` generated=0|1` after `bytes=N` |
| `symbols <dir>` | P8a | directory input and grouped output; unknown path uses the new `0 symbols` reason |
| `dead`/`clones`/`hotspots`/`unresolved`/`report` | P8a | new `--path` scope; default output unchanged |
| `trait`/`interface` symbol rows | P8a | default `symbols`/`search`/`stats` output may gain rows; `dead` kind set expands |
| `imports` | P8b | new command |
| `tests` | P8b | new command |
| `resolve` | P8b | new command |
| `search` | P8b | new filters; default unchanged |
| `unresolved` | P8c | `--names`, `--path`; default header unchanged; `--names` has a new shape |
| `complexity` | P8c | new command |
| `symbols --metrics` | P8c | additive columns |
| `api` | P8c | new command; signature column included |
| `stats` | P8c | `generated=N` inserted after `files=N` |
| `dead`/`clones`/`hotspots`/`report`/`complexity`/`api` | P8c | opt-in `--exclude-generated`; default output unchanged |
| `impact`/`callers`/`callees`/`symbols`/`imports` | P8a/P8b | `--context` only on `impact`; `callers --context` withdrawn to avoid duplicating `kg context` |

## 5. Features

### F1 — Inheritance/implementation surface, Rust trait impls, and `dead` correctness (P8a)

**Question answered:** which classes, interfaces, traits, or mixins inherit
from or implement this symbol?

**New symbols:** add `trait_item` and `function_signature_item` (Rust) and
`interface_declaration` (TypeScript) to extraction with kinds `trait` and
`interface`. Trait/interface methods are `method` symbols. Without these
rows, `target_ids` cannot resolve the very names the feature asks about.
Supertrait edges (`trait A: B`) are `inherits` edges from A to B.

**Surface:** new `kg implementers <NAME> [--limit N]`.

- Input: `target_ids` exact bare name or full qualname. Zero -> exit 1;
  multiple -> exit 2 with candidates.
- Edge set: inbound `edges WHERE kind = 'inherits' AND dst_id IS NOT NULL`,
  direct only. Transitive descendants are composed by re-querying.
- Deduplication: one row per `symbols.id`; `M = COUNT(DISTINCT s.id)`;
  order `path, line_start, qualname`.
- Output:
  - first line: `kg: implementers — resolved static inherits edges only; dynamic inheritance may hide implementers`;
  - data: `path:line qualname` at the inheriting symbol's declaration line;
  - `showing N of M` only when capped.
- Inherits confidence: exactly one destination -> `EXTRACTED`; more than one
  destination -> `AMBIGUOUS` with `dst_id` NULL plus an unresolved
  `kind='inherit'` row; zero -> unresolved `kind='inherit'` only.
- **Rust extraction:** add `impl Trait for Type` and `impl Trait for &Type`
  edges from the implementing type symbol to the trait symbol, in addition to
  the existing method scoping. Impl-trait pairs do **not** enter
  `class_bases`; call resolution is unchanged (regression fixture). A
  cross-file impl is out of scope in v1: the type symbol and impl must be in
  the same file; a cross-file impl is stored as unresolved
  `kind='inherit'` and named as a limitation in the `implementers` caveat.
- **TypeScript heritage:** read `extends_clause.value` and the `type` nodes
  of `implements_clause`. If the heritage value is a call expression
  (`extends mixin(Base)`), take the first identifier argument (`Base`) as the
  base; if it has no identifier argument, store unresolved
  `kind='inherit'` and emit no edge. A negative fixture covers
  `extends mixin(1)`.
- `dead` predicate (complete, preserving the current AMBIGUOUS suppression):
  - `s.kind IN ('function','method','class','trait','interface')`;
  - no resolved inbound `calls`, `imports`, or `inherits` edge;
  - no AMBIGUOUS `calls` edge naming the symbol under the current per-file
    bare-name rule;
  - no AMBIGUOUS `imports`/`inherits` edge naming the symbol; the same
    per-file bare-name rule applies;
  - a method whose parent symbol is a `trait` or `interface` is not a dead
    candidate; the parent lookup is by qualname prefix and kind;
  - `--exclude-tests` keeps its current default-off meaning and uses the
    exact predicate in F8 (path segments, filename globs, `test_*` names);
  - `--exclude-generated` is added by F11.
- `dead` default remains "tests included"; skills pass `--exclude-tests`.
  This avoids a default-output break for existing users. Star-imported
  public symbols and plain module-level imports remain known false-positive
  classes: the star/plain import edge targets the module, not each symbol;
  the caveat says so.
- `impact` gains inbound `inherits`/`imports` through F4, not by changing
  `dead`.
- Gate G23.

### F2 — Impact caller context (P8a)

**Question answered:** which function or method contains this impacting call
site?

**Surface:** `--context` on `impact` only.

- `callers` and `callees` do not gain `--context`: `kg context <symbol>`
  already returns the one-call symbol view (definition + callers + callees).
- Output line: `path:line [in qualname]` when the site is inside a
  function/method; module-level sites keep `path:line`. With `--sites`, each
  site line is annotated.
- Selection: innermost function/method symbol whose `[line_start, line_end]`
  contains `site_line`; tie-break smallest range, then `qualname`.
- Tag order with `--explain`: `[ambiguous] [in qualname] [kind depth=N]`.
- Journal/payload per §4.7; byte-identical default without `--context`.
- Gate G23.

### F3 — Path scope, file inventory, and directory outline (P8a)

**Question answered:** what is in this directory or package, and how do I
scope a repo-wide analysis to it?

1. `--path <prefix>` on `dead`, `clones`, `hotspots`, `unresolved`, `report`
   in P8a; `complexity` and `api` gain it in P8c. Rules in §4 item 9.
   - `report --path`: sections 2-4 and the overview totals are scoped to the
     prefix; section 1 keeps index totals and adds `scoped files=N
     symbols=N`.
   - `clones --path`: a pair is shown only when both regions are under the
     prefix; totals count only those pairs.
   - `unresolved --path`: `--names` and the default counts are scoped;
     header totals are scoped.
2. New `kg files [--path P] [--limit N]`:
   - one line per indexed file: `path symbols=N bytes=N`;
   - `symbols` includes the module symbol; `bytes` is build-time
     `files.size`; files skipped by the >1 MB cap or parse-status filters
     are absent, not zero-filled;
   - sorted by path; standard cap; no `showing` line unless capped;
   - build-time values: `bytes`/`symbols` are index-time, not a live
     filesystem stat; `files` is a report shape with no staleness pass;
   - `generated=0|1` is appended after `bytes=N` when F11 lands;
   - empty scope: exit 1 with the §4.9 string.
3. `kg symbols <dir> [--outline] [--limit N]`:
   - precedence: an exact indexed file path uses file mode and keeps the
     existing byte-identical output; otherwise a prefix match uses directory
     mode; otherwise exit 1 with the §4.9 string;
   - directory detection is index-based: files whose path starts with
     `dir + "/"`; the filesystem is not walked;
   - exact sequence: staleness header (if any), then
     `kg: dir <dir> — F files, S symbols`, then per file `path`, then
     `path:line kind qualname [signature]`, then `showing N of S` only when
     capped;
   - `F` and `S` count indexed files and symbol rows in scope; `M = S`;
     `--limit` counts data lines only; headers do not count;
   - sorted by path, `line_start`, `qualname`; a zero-symbol directory exits
     1 with the §4.9 string.
- Gate G23.

### F4 — Transitive impact and edge kinds (P8a)

**Question answered:** which files depend on this file, directly or through a
bounded chain?

**Base contract:** the worktree already groups `impact` by caller file with a
site count and `--sites` restores per-site rows. This proposal keeps that
base and records it as the P8a contract.

- `--depth N` (default 1, clamp 1-10): reverse BFS over the selected edges.
- `--edges <list>` (default `calls`): comma-separated `calls`, `imports`,
  `inherits`, `all`. `all` = calls + imports + inherits; `contains` is never
  included. Duplicates are removed in the fixed order calls, imports,
  inherits; `all` mixed with explicit kinds exits 2; empty/invalid values
  exit 2.
- Default (`--depth 1 --edges calls`, no `--sites`, no `--explain`, no
  `--context`) stays byte-identical to the current grouped `impact`. Depth 1
  uses the current SQL edge set exactly, so self-file callers keep their
  current bytes.
- `--sites` is valid only with `--depth 1`; combining `--sites` with
  `--depth > 1` exits 2. In per-site mode, one line per edge site, ordered by
  `path, site_line, dst.qualname, edge.id`; in default mode, one line per
  dependent file, keeping the smallest `site_line` and adding `(N sites)`.
- Transitive mode (depth > 1): one line per file at its minimum depth,
  first-hop site, sorted by `path`; ties by `site_line`, then source
  qualname. With `--explain`, append ` [kind depth=N]`; with `--context`,
  append ` [in qualname]`; ambiguity tag precedes both. `--explain` at depth
  1 implies per-site rows; at depth > 1 it annotates the per-file first-hop
  edge.
- Results in the target file: depth 1 keeps the current behavior (a
  self-file edge is included). Depth > 1 first computes R1 as the exact
  depth-1 result set, then expands from R1's source symbols; R1 is always
  included, so deeper search is monotone. A target-file caller present in R1
  is reported even if it is also a target symbol. This depth>1-only delta is
  recorded in §4.10.
- `--edges imports` includes module and symbol import edges. `--edges
  inherits` includes inbound inherits into symbols in the target file.
- Caveat when `--edges` contains imports or inherits:
  `kg: impact — resolved static edges only; unresolved imports/inherits are omitted`.
  It prints on stdout before data and on empty results. Empty strings:
  - calls only: `0 impacting files found`;
  - with imports: `0 impacting files found (unresolved imports are omitted)`;
  - with inherits: `0 impacting files found (unresolved bases are omitted)`.
- Bounds: reverse BFS uses level-batched indexed queries (500-id chunks),
  a symbol-id visited set, and `LIMIT` + `COUNT(*)` over output rows. `M` is
  the number of distinct dependent files reachable within depth N; the
  closure is traversed fully. A cap hit prints `showing N of M`; a depth-cap
  stop with a non-empty result prints
  `kg: depth cap D reached; deeper dependents may exist`.
- Gate G23.

### F5 — Unresolved names with locations and edge kind (P8c, uses v6)

**Question answered:** which callee, import, or base name failed to resolve,
and where?

- Schema v6 adds `unresolved_calls.site_line INTEGER NOT NULL DEFAULT 0`
  and `unresolved_calls.kind TEXT NOT NULL DEFAULT 'call'` (`call|import|inherit`).
  `site_line` is 1-based on fresh extraction; a fresh row with 0 is a
  controlled build error (exit 6 with a message, covered by a fixture), not
  a panic.
- Extraction stores unresolved calls, imports, and inheritance bases with
  their site line. The fast-path gap query extends from calls to all three
  kinds, so a new module/base class heals carried importers/subclasses.
- `kg unresolved --names [--path P]`:
  - the default `kg unresolved` report stays byte-identical:
    `kg: unresolved={total} callees in {n_files} files, {distinct} distinct names (internal={internal})`,
    followed by the existing top-10 names; `total` remains call-only
    (`files.unresolved`), not the sum of all kinds;
  - `--names` uses `kg: unresolved=N rows in M files (call/import/inherit)`;
    `N` = located rows in scope, `M` = distinct files with located rows;
  - data: `path:line kind name`, sorted by path, site_line, kind, name;
  - cap line for `--names`: `showing N of M rows`;
  - `--names` is a pointer mode: run the post-cap staleness check on the
    displayed rows and print the standard header; the default report keeps
    no staleness pass;
  - report shape default: no payload line; `--names` also has no payload
    (recorded deviation);
  - rows with `site_line=0` are not printed; a stderr line reports the count
    of unlocated rows (expected zero on fresh builds; controlled build error
    if a fresh row is 0).
- Default `kg unresolved` stays byte-identical.
- Gate G26.

### F6 — Batch resolve and search filters (P8b)

1. `kg resolve --stdin`:
   - one name per line; trim surrounding whitespace; skip blank lines and
     lines whose first non-space byte is `#`; names are case-sensitive;
   - each name uses `target_ids`; output per input line:
     `found <name> path:line qualname`,
     `ambiguous <name> path:line qualname` (one per candidate), or
     `missing <name>`;
   - caps: 1000 names, 4096 bytes per line, 1 MiB total input; exceeding a
     cap exits 2 with a usage message; empty input exits 1 with
     `no names on stdin`;
   - duplicates are resolved once and printed once per input occurrence;
   - per-name candidates capped at 50, with a `showing N of M candidates`
     line when capped;
   - output is buffered; a fatal exit 3-7 emits no partial stdout;
   - CLI exit: 0 all found, 1 any missing, 2 any ambiguous (ambiguous wins
     over missing), 3-7 environmental.
   - MCP `resolve` returns the same text and sets `isError` for non-zero;
     MCP has no exit-code field, so the exact CLI exit is not transmitted
     (recorded deviation). This is the only success-text/error case.
     Implement it as a batch outcome `(text, is_error)`; do not route it
     through the error renderer, which would prefix the error line and
     mangle successful per-name lines.
   - Journal `args`: `count` and `candidate_limit=50`; never the names.
   - MCP `names` uses the same 4096-byte per-name and 1 MiB total bounds.
2. `kg search <terms> --kind <k> --path <P> --exact`:
   - `--kind` filters symbol kind; `--path` uses §4 item 9;
   - `--exact` matches full qualname or bare name exactly, bypassing FTS;
     exact hits first, then path, line, qualname; the signature column stays
     (Amendment 14);
   - default output unchanged.
- Gate G27.

### F7 — Module import graph (P8b)

**Question answered:** what does this file import, and which files import it?

**Edge contract (all languages):**

| Field | Value |
|---|---|
| kind | `imports` |
| source | the importing file's module symbol |
| destination | the resolved destination module symbol, or the imported symbol when only a symbol resolves |
| confidence | `EXTRACTED` for one destination; `AMBIGUOUS` for a star import (existing module-symbol edge); no edge for zero destinations |
| site_line | the import statement line in the source file |
| unresolved | a `unresolved_calls(kind='import')` row when no destination resolves |

**Extraction:**

- Python:
  - `import a` -> module `a`; `import a.b` -> modules `a` and `a.b` (the
    parent package executes);
  - `import a.b as x` binds `a` for calls but emits the same two module
    edges; keep the binding path separate from the edge-target path in the
    extraction candidate (`ModuleRoot { bind, target }`);
  - `from . import a` -> the package child module (`pkg.a`);
  - `from .a import b` -> module `pkg.a` plus symbol `pkg.a.b` when it
    resolves;
  - `from pkg import mod` -> module `pkg.mod` when indexed as a module,
    otherwise symbol `pkg.mod` when indexed as a symbol;
  - `from a import *` keeps the existing AMBIGUOUS module edge; do not add a
    second edge.
- TypeScript/JavaScript:
  - `import ... from './x'`, side-effect `import './x'`,
    `export ... from './x'`, `export * from './x'`, static `import('./x')`,
    and CommonJS `require('./x')`;
  - relative resolution tries the exact file, then `.ts`, `.tsx`, `.js`,
    `.jsx`, `.mts`, `.cts`, `.mjs`, `.cjs`, then `/index.*` for each;
  - `require('./x')` and `import('./x')` emit exactly one import edge and no
    call edge, and add no unresolved-call row.
- Rust:
  - `use crate::a::b` -> module `src.a` plus symbol `src.a.b`;
  - grouped `use crate::a::{b, c}`, `use ... as alias`, `use self::`,
    `use super::`, and single-segment `use crate::a;` -> module `src.a`;
  - `mod a;` -> module `src.a`;
  - external crates, `std`, and builtins emit no edge.
- Unresolved modules are stored as `unresolved_calls(kind='import',
  site_line=<import line>)` for fast-path healing; they do not emit edges.

**Query:** new `kg imports <FILE> [--limit N]`.

- Output: `path:line target`, where `path` is the importing file, `line` is
  the import statement, and `target` is the resolved destination qualname;
  one row per `(line, target)`; sorted by line, target; standard cap.
- Output order: caveat, then staleness header (if any), then data.
- First line: `kg: imports — resolved static imports only; external and unresolved imports are omitted`.
- Inbound direction is `kg impact <FILE> --edges imports`.
- MCP/journal/payload per §4.7/§4.8.
- Known limits: `export *`/`export { X } from` do not mark re-exported
  symbols in the current file (F10); external packages are unresolved; a
  parent package edge is emitted only for Python dotted imports.
- Gate G24.

### F8 — Test impact (P8b)

**Question answered:** which tests reach this symbol or file?

**Surface:** new `kg tests <NAME|FILE> [--depth N] [--limit N]`.

- Input: an indexed file path uses every symbol in that file; otherwise
  `target_ids`. Zero/ambiguous follow the standard exits.
- Search: reverse BFS over `calls` edges, depth default 10 (clamp 1-10).
  Traversal identity is the symbol id; output dedup is by test symbol id.
- Test candidate rule: a symbol is reported only when its kind is
  `function` or `method` and its name matches `^test_` or `^test[A-Z]`
  (pytest/unittest style). Helpers inside test files do not match.
- Path classification is a separate policy used only by
  `dead --exclude-tests`, not by `kg tests`:
  - directory segments (not substrings) are `tests`, `test`, or
    `__tests__`;
  - filename matches `test_*.py`, `*_test.py`, `*.test.*`, `*.spec.*`, or
    `*_test.dart`.
- Known limits (pinned in the caveat): Rust `#[test]` functions whose names
  do not start `test_`/`testA` are missed; TS `describe`/`it` arrow callbacks
  are not symbols and are not reported; a test symbol in the target file is a
  seed and is not reported.
- Output: caveat line, then `path:line qualname` for the test symbol, sorted
  by path, line, qualname; standard cap. `M` = distinct test symbol ids
  reachable within depth N.
- A depth-cap stop with zero results exits 1 with
  `no test candidate within depth D; a longer route may exist (static call
  edges only)`. An exhausted search uses
  `0 test candidates found (static call edges only)`. A depth-cap stop with
  non-empty results prints `kg: depth cap D reached; deeper tests may exist`.
- Gate G25.

### F9 — Per-symbol metrics (P8c, schema v6)

**Question answered:** which functions exceed cyclomatic, nesting, branch, or
length ceilings?

**Schema:** `symbols.cyclomatic`, `symbols.branches`, `symbols.nesting`,
`symbols.effective_loc` (all `INTEGER NOT NULL DEFAULT 0`).

**Formulas (pinned; per-symbol metrics do not change the file-level
`hotspots` metric):**

- `cyclomatic` = 1 + decision nodes inside the callable, excluding nested
  callable bodies:
  - Python: `if_statement`, `for_statement`, `while_statement`,
    `boolean_operator`, `elif_clause`, `except_clause`, `case_clause`,
    `conditional_expression`;
  - TS/JS: `if_statement`, `for_statement`, `for_in_statement`,
    `while_statement`, `do_statement`, `switch_case`,
    `conditional_expression`, `ternary_expression`;
  - Rust: `if_expression`, `for_expression`, `while_expression`,
    `loop_expression`, `match_arm`.
- `branches` counts statement-level branch nodes only:
  - Python: `if_statement`, `for_statement`, `while_statement`,
    `elif_clause`, `except_clause`, `case_clause`;
  - TS/JS: `if_statement`, `for_statement`, `for_in_statement`,
    `while_statement`, `do_statement`, `switch_case`;
  - Rust: `if_expression`, `for_expression`, `while_expression`,
    `loop_expression`, `match_arm`.
  `boolean_operator`, `conditional_expression`, `ternary_expression`, and TS
  `?` are counted in `cyclomatic` only; Python comprehensions
  (`for_in_clause`, `if_clause`) and TS logical `&&`/`||` are not counted
  (stated limit). `match_statement` is not counted separately; its
  `case_clause` arms are.
- Nested callable bodies are excluded by grammar node kind: `lambda`,
  `arrow_function`, `function_expression`, `function_declaration`,
  `method_definition`, `generator_function_declaration`,
  `closure_expression`, and nested `function_item`/`function_definition`
  nodes. Do not exclude by symbol range; lambdas and callbacks are not
  symbols.
- `nesting` = maximum depth of the `branches`-list nodes; a branch directly
  in the callable body is depth 1. Non-branch containers (`with`, `try`,
  `unsafe`, block expressions) do not add depth.
- `effective_loc` = `line_end - line_start + 1` minus blank lines and
  comment-only lines. Comment node kinds: `comment` (Python/TS),
  `line_comment`/`block_comment`/`doc_comment` (Rust). A line is
  comment-only when every non-whitespace byte on it belongs to one of those
  nodes, or to the symbol's docstring (the first `expression_statement`
  containing a `string` in the body). Inline trailing comments do not make a
  line comment-only.
- Non-callable symbols store 0 and are excluded from `kg complexity`.
- Metrics are computed in the existing parse walk. The spec pins a single
  pre-order walk with a callable stack; if the implementation needs a second
  subtree walk, the 1.25x build budget in §7.5 must be re-measured.
- `meta.metrics_version` is added (a meta row, not a schema column). A
  metrics-rule change bumps it; `probe_old` treats a mismatch like a grammar
  mismatch and disables the fast path. It is written by the build, not
  carried forward.

**Surfaces:**

- `kg symbols <file> --metrics` appends ` cyc=N branches=N nest=N loc=N`.
- `kg complexity [--path P] [--min-cyc N] [--min-loc N] [--min-branches N]
  [--min-nest N] [--limit N]`:
  - output `path:line cyc=N branches=N nest=N loc=N qualname`;
  - filters are AND; defaults `min_cyc=1`, others 0; `min_cyc=1` includes
    every callable, so the default is a top-50 ranking, not a threshold;
  - sorted by `cyclomatic` desc, `effective_loc` desc, `qualname`, `path`,
    `line_start`;
  - `M` = callables matching the filters; `COUNT(*)` is a separate statement;
  - default cap 50; caveat line unconditional:
  `kg: complexity — static AST proxy, not cognitive complexity; not comparable to hotspots/file cyclomatic`.
- Gate G26.

### F10 — Public API/export inventory (P8c, schema v6)

**Question answered:** what is the public surface of this package or
directory?

**Schema:** `symbols.exported INTEGER NOT NULL DEFAULT 0`.

**Rules:**

- Only indexed symbol kinds can be exported: `function`, `method`, `class`,
  `trait`, `interface`. Module symbols are excluded from `api`.
- Python: a literal `__all__` restricts exports to its names; otherwise
  module-level functions/classes whose name does not start `_`, plus
  non-underscore methods of exported classes. A non-literal `__all__` uses
  the fallback and is marked `exported=1` with the `api` caveat.
- Rust: `pub` items are exported; `pub(crate)`, `pub(super)`, and `pub(in
  path)` are not; `pub` methods in inherent or trait impls are exported when
  the type is exported. `pub` inside a private module is still treated as
  exported (module-path visibility is not modeled); this is a named limit in
  the `api` caveat. A cross-file impl is a named limit (F1).
- TypeScript/JavaScript: `export` declarations and public class members are
  exported; `export default` uses the declaration name; `export { X } from
  './x'` and `export * from './x'` emit import edges but do not mark the
  re-exported symbols in the current file; they stay exported in their
  defining file.
- Dart (P9): non-underscore declarations are exported. `part` files keep
  their own file-level `exported` flags; `api` on a library includes the
  library's part files at query time. No cross-file stored export state.
  P9's addendum pins the node kinds and part resolution.

**Surface:** `kg api <FILE|DIR> [--include-private] [--exclude-generated] [--limit N]`.

- Output `path:line kind qualname`, sorted by path, line, qualname; standard
  cap.
- Generated files are included by default; `--exclude-generated` removes them.
  `--include-private` adds private symbols. The two flags are independent.
- `DIR` is recursive over indexed paths under §4 item 9.
- First line: `kg: api — export rules are static heuristics; re-exports, module visibility, and dynamic exports may be incomplete`.
- Gate G26.

### F11 — Generated-file tagging (P8c, schema v6)

**Question answered:** which indexed files are generated, and which rankings
must exclude them?

**Schema:** `files.generated INTEGER NOT NULL DEFAULT 0`.

**Detection:**

- path extension, exact suffix list: `.g.dart`, `.freezed.dart`,
  `.generated.ts`, `.generated.js`, `.generated.py`, `.generated.rs`,
  `.generated.mts`, `.generated.cts`, `.generated.mjs`, `.generated.cjs`,
  `_pb2.py`; the Dart patterns `*.g.dart`/`*.freezed.dart` are activated by
  P9;
- first five lines contain `@generated`, `Code generated .* DO NOT EDIT`,
  `This file was generated`, or `AUTO-GENERATED`. A bare `DO NOT EDIT` is
  not sufficient.
- A directory named `gen/` or `generated/` alone is **not** sufficient; it
  must also have a marker or a generated extension. A hand-written `gen/`
  fixture and a hand-written file with `DO NOT EDIT` are negative controls.

**Behavior:**

- `dead`, `clones`, `hotspots`, `report`, `complexity`, `api` exclude
  generated files when `--exclude-generated` is passed; default output is unchanged.
- `callers`, `callees`, `path`, `impact`, `symbols`, `imports` keep
  generated files navigable.
- `kg stats` gains `generated=N` after `files=N`; contract bump.
- Gate G26.

### F12 — Dart/Flutter support (P9)

**Question answered:** the call, inheritance, import, test, API, and
complexity questions for Dart/Flutter repositories.

**Dependency:** one Dart grammar crate (for example `tree-sitter-dart`) or a
vendored grammar. This is the only new dependency in P8/P9 and requires owner
approval plus the Cargo.lock allowlist update. If no maintained crate passes
the grammar-version rule, P9 is killed; no grammar is vendored without owner
approval.

**Implementation readiness:** F12 is a charter, not an implementation
contract. Before any P9 code, a separate P9 addendum must pin the grammar
crate and version, the actual tree-sitter node kinds for every symbol and
edge below, the pubspec discovery rule for monorepos (one `pubspec.yaml` per
package, nearest ancestor of the source file), and the Dart decision kinds
for metrics. G28 gates that addendum and its fixtures; it is not a license to
start coding from the concept list.

**Extraction:**

- symbols: `class`, `mixin`, `extension`, `enum`, `function`, `method`,
  `getter`, `setter`, `constructor`; qualnames scoped by class/mixin/
  extension; constructors scoped as `Class.Class`.
- edges: `contains`; `calls` for identifier calls, method invocations,
  constructor invocations, `this.`/`super.` calls, static calls; `imports`
  for relative imports, `package:<own_package>/` imports, `part`/`part of`;
  `inherits` for `extends`, `implements`, and `with` (one edge per base).
- resolution: relative imports map to module paths; `package:<own_package>/`
  maps through the nearest ancestor `pubspec.yaml` `name` + `lib/`; external
  packages are unresolved; `part` files map to the owning library. A
  monorepo may have multiple pubspecs; the nearest ancestor wins.
- non-goals: widget tree, route mapping, provider graph, build_runner
  semantics (generated files are tagged by F11), dynamic dispatch.

**Surface:** no new command; `callers`, `callees`, `path`, `impact`,
`imports`, `tests`, `dead`, `api` work through existing surfaces. Dart uses
schema v6. `complexity` for Dart is **out of scope in v1**: P9 must pin Dart
decision kinds and metric formulas in a separate P9 addendum before
`complexity` includes Dart. The P9 addendum also pins the grammar crate
version, node kinds, pubspec discovery, and `part` resolution. This section
is a charter until that addendum is approved.

**Gate G28.**

## 6. Schema v6 (introduced in P8a; populated through P8c)

One additive bump carries F5, F9, F10, and F11. The `ALTER` statements below
are the logical delta; `schema.sql` expresses the same columns in the
fresh-build `CREATE TABLE` statements and must also create the index.

```sql
ALTER TABLE symbols ADD COLUMN cyclomatic INTEGER NOT NULL DEFAULT 0;
ALTER TABLE symbols ADD COLUMN branches INTEGER NOT NULL DEFAULT 0;
ALTER TABLE symbols ADD COLUMN nesting INTEGER NOT NULL DEFAULT 0;
ALTER TABLE symbols ADD COLUMN effective_loc INTEGER NOT NULL DEFAULT 0;
ALTER TABLE symbols ADD COLUMN exported INTEGER NOT NULL DEFAULT 0;
ALTER TABLE files ADD COLUMN generated INTEGER NOT NULL DEFAULT 0;
ALTER TABLE unresolved_calls ADD COLUMN site_line INTEGER NOT NULL DEFAULT 0;
ALTER TABLE unresolved_calls ADD COLUMN kind TEXT NOT NULL DEFAULT 'call';
CREATE INDEX symbols_complexity_idx
  ON symbols(cyclomatic DESC, effective_loc DESC, qualname, file_id, line_start);
```

- `PRAGMA user_version = 6`; `src/extract.rs:21` `SCHEMA_VERSION` is the single
  stamp site. A fresh `schema.sql` carries the columns and index.
- Query opens exit 5 with the rebuild hint on a v5 index. `kg build` ignores
  the old index and rebuilds fresh (it does not abort on version mismatch).
- `probe_old` must verify the complete v6 column set (`PRAGMA table_info`
  for `symbols`, `files`, `unresolved_calls`) plus `clone_groups`,
  `symbols_fts`, and `symbols_complexity_idx`, not just the version stamp. A
  v6-stamped partial index rebuilds fresh. Test: drop one new column and
  expect a fresh rebuild with exit 0; drop the index and expect the same.
- Carry-forward must name every new column in the three `carry_forward
  _unchanged` statements: `files.generated`, `symbols.cyclomatic/branches/
  nesting/effective_loc/exported`, and `unresolved_calls.site_line/kind`.
  One assertion per column. `meta.metrics_version` is not carried; the build
  writes it fresh and `probe_old` rejects a mismatch.
- Determinism tests extend their SELECT lists with all new columns and
  `unresolved_calls ORDER BY file_id, kind, dst_name, site_line`.
- Release boundary: v6 is not released until all P8a-P8c gates pass. P8a and
  P8b are integration phases, not releases. If a P8c feature is killed during
  integration, its columns are removed before the first v6 release. If a
  feature is killed after a v6 release, the columns remain and only the
  command/flag surface is removed; an in-place column drop is forbidden, and
  a later cleanup is a v7 rebuild under the single normative version policy.

### 6.1 Schema v7 (Amendment 20, P8 optimization)

One additive bump after the P8 review cycle, limited to deterministic
`complexity` ordering:

```sql
ALTER TABLE symbols ADD COLUMN file_path TEXT NOT NULL DEFAULT '';
-- Replaces the v6 index; the old index is dropped with the table on rebuild.
CREATE INDEX symbols_complexity_idx
  ON symbols(cyclomatic DESC, effective_loc DESC, qualname, file_path, line_start);
```

- `PRAGMA user_version = 7`; `SCHEMA_VERSION = 7`. Query opens on v6 exit 5
  with the rebuild hint; `kg build` rebuilds fresh.
- `probe_old` adds `("symbols", "file_path")` to the required column set. A
  v7-stamped index missing the column or the index rebuilds fresh.
- `insert_symbols` fills `file_path` from the aligned fresh file paths;
  `carry_forward_unchanged` copies it for every carried symbol. The value is
  `files.path` (root-relative), never absolute.
- Gate G26 extends with: fresh and carried indexes return byte-identical
  `complexity` output for `pkg.py` vs `pkg/__init__.py` ties, ordered by path.

## 7. Algorithms, bounds, and determinism

### 7.1 Reverse BFS (`impact --depth > 1`, `tests`)

- Seed for `impact --depth > 1`: R1 is the exact depth-1 result set; the
  frontier starts from R1's source symbols at distance 1. Target-file
  symbols are distance 0 only for cycle detection; R1 is always included.
  Seed for `tests`: the resolved target symbol set.
- Frontier: FIFO; visited-on-enqueue by **symbol id**; neighbors ordered by
  `src.qualname, src.path, edge.site_line`. File deduplication applies to
  output rows only, never to traversal identity.
- Parent/first-hop: the edge that first discovers the symbol is the reported
  site; for a file, keep the discovery at the minimum depth, then the
  smallest `site_line`, then `src.qualname`. F4 and this section use the same
  rule; one wording is authoritative.
- Output totals: `M` = distinct dependent files or test symbols reachable
  within depth N, not edge count. The closure is traversed fully; cost is
  bounded by the selected edge rows plus the visited set.
- Depth cap: 1-10; an exhausted frontier below the cap is definitive for the
  selected edge set; a cap stop with results prints the depth-cap footer from
  F4/F8.
- Cost: level-batched indexed queries (500-id chunks), `LIMIT` +
  `COUNT(*)` for output, bounded visited set; measure at 1M edges.
- Tests in the target file: seeds are visited, so a test symbol that is also
  a target is not reported. This is a documented limitation of
  `kg tests <FILE>`.

### 7.2 Import resolution

- Module path: existing `module_path_for`; Python package markers and TS
  index files collapse as today. Extend the strip/package set to
  `.mts/.cts/.mjs/.cjs` and `index.mts/index.cts/index.mjs/index.cjs`.
- Python: resolve `import a`, dotted `import a.b` (both `a` and `a.b`),
  `from . import a` (package child), `from .a import b` (module plus
  symbol), `from pkg import mod` (module first, symbol fallback), and
  `from a import *` (existing AMBIGUOUS module edge). Unresolved targets are
  stored as `kind='import'`.
- TS: relative path resolution tries the exact file, `.ts`, `.tsx`, `.js`,
  `.jsx`, `.mts`, `.cts`, `.mjs`, `.cjs`, then `/index.*` for each;
  side-effect imports, `export from`, `export *`, dynamic `import()`, and
  `require` are included. `require`/`import()` are import edges only.
- Rust: parse `scoped_use_list`, `use_list`, `use_as_clause`,
  `use_wildcard`, `scoped_identifier`, and `mod_item`. `crate` maps to the
  crate-root dir segment; `self` to the current module; `super` to the
  parent module. External crates emit no edge.
- Determinism: edge list sorted by `(src_id, dst_id, site_line, kind)`
  before insert; unresolved list sorted by `(file_id, kind, dst_name,
  site_line)`.

### 7.3 Test classification

Pinned path/symbol rules in F8. Classification is query-time, not a stored
table. `dead` uses the path subset via `--exclude-tests`; `clones` and
`hotspots` are not changed by P8.

### 7.4 Metrics

Pinned formulas in F9. Metrics are computed in the parse walk and stored
with `meta.metrics_version`; a formula change invalidates the fast path.

### 7.5 Performance budgets

| Surface | Budget | Corpus |
|---|---|---|
| `implementers` | < 20 ms best-of-7 | a corpus with `inherits` edges (not the self index; E2 has 0) |
| `imports`, `tests` | < 20 ms best-of-7 | self index |
| `resolve` with 1000 names | < 100 ms best-of-7 | self index |
| `impact --depth 3` | < 50 ms best-of-7 | self index; < 150 ms at 1M edges |
| `tests --depth 10` | < 50 ms best-of-7 | self index; < 200 ms at 1M edges |
| `complexity` (COUNT + LIMIT, unscoped) | < 50 ms best-of-7 | 1M symbols, index-backed |
| `complexity --path` (COUNT + LIMIT) | < 100 ms best-of-7 | 1M symbols; scoped plan measured |
| `files`, `symbols <dir>` | < 50 ms best-of-7 | 2,025-file corpus |
| build overhead (imports + metrics) | <= 1.25x baseline wall | 2,025-file corpus, frozen baseline |
| Dart build overhead | <= 1.25x baseline wall | Flutter GT corpus |

Each gate that claims a latency budget must run a new
`bash tests/scripts/timing_assert.sh 7 <case> <budget-ms> <binary> ...`; the
script exits non-zero on budget overrun. `complexity` also runs `EXPLAIN
QUERY PLAN` and asserts the index is used for the unscoped ORDER BY.
`implementers` cannot use the self index as its latency corpus because it has
no inherits edges.

No new query materializes an unbounded result set. `COUNT(*)` and `LIMIT`
are separate statements, as in the existing bounded query path.

### 7.6 Gate harness deliverables

The implementation phase must commit these before any gate can pass:

- `tests/scripts/import_recall.py` and `docs/baselines/import-gt.jsonl`
  (40 hand-verified import pairs, independent oracle, freeze commit; report
  recall/precision with Wilson 95% intervals; local `--kg-root` and
  `--vihap-root` arguments, private-corpus gate);
- `tests/scripts/tests_recall.py` and `docs/baselines/tests-gt.jsonl`
  (20 test symbols across direct/indirect/negative cases and one real repo;
  report precision/recall with Wilson intervals);
- `tests/scripts/timing_assert.sh` with per-case budgets from §7.5;
- `docs/baselines/inheritance-gt.jsonl` (Rust trait, TS interface, Python
  multiple inheritance, negative controls);
- `docs/baselines/dart-gt.jsonl` for P9, with 20 call sites and 10
  inheritance pairs, precision >= 80% / recall >= 70%, Wilson intervals.

Every gate also runs MCP byte parity for its new tools and the
`code-index`/`kg-first` skill drift check. The drift check must parse fenced
code blocks, not only inline backticks, and must assert consumer-skill lines;
when `KG_BIN` is unset it runs a static command/flag manifest instead of
skipping the whole check.

## 8. Gate registry

| Gate | Phase | Bar | Pass command | Kill / withdrawal |
|---|---|---|---|---|
| G23 | P8a | `implementers` 100% exact bytes on the inheritance fixture (Rust trait, TS interface, Python multiple inheritance, negative controls); `dead` 0 false positives on inherited/imported symbols, positive dead fixture, no AMBIGUOUS-call regression; `impact` default byte-identity plus `--edges/--depth/--context` exact bytes and depth-monotonicity; `--path` boundary tests; `symbols <dir>` exact bytes; v6 migration/partial-index tests; MCP parity; query bytes equal across fresh/carried and serial/parallel builds | `cargo test p8a_` + `cargo test --test mcp` + `KG_WORKERS=1` equality + `bash tests/scripts/timing_assert.sh 7 implementers 20 <binary> ...` | Revert P8a and schema v6; remove `implementers`, v6 columns, flags, MCP tools, journal/payload rows, and skill lines |
| G24 | P8b | Import edges: >= 90% recall and >= 90% precision on 40 frozen pairs over kg + vihap, Wilson 95% reported; Python plain/dotted/relative/star, TS relative/side-effect/require, Rust use/mod fixtures exact; unresolved-import gap-healing; `require`/`import()` add no unresolved call; MCP parity; import-only build overhead <= 1.15x | `python3 tests/scripts/import_recall.py --pairs docs/baselines/import-gt.jsonl --kg-root <repo> --vihap-root <repo2> <binary>` + `cargo test import_` | Remove import extraction and `kg imports`; withdraw `--edges imports`, unresolved kind rows, MCP tool, journal/payload rows, and skill lines |
| G25 | P8b | 10 seeded tests plus 20 real-repo test symbols; >= 90% recall and 0 non-test false positives under the name-only candidate rule; helper-in-test-file negative control; truncated-depth footer; exact bytes; MCP parity; timing | `cargo test tests_impact_seed` + `python3 tests/scripts/tests_recall.py ...` + `timing_assert.sh` | Remove `kg tests`; withdraw MCP/journal/payload rows and skill lines |
| G26 | P8c | Per-feature sub-verdicts: (a) metrics exact on Python/TS/Rust fixtures incl. `elif`, ternary, lambda/arrow, comprehension, comment, nested-class cases; (b) `api` exact for `__all__`, `pub`/`pub(crate)`, local/default/re-export/type-only exports, module-symbol limit; (c) generated `--exclude-generated` 10/10 markers, 0/10 false positives incl. `gen/` and bare `DO NOT EDIT`; (d) unresolved `--names` exact, `site_line>=1`, kind, header scoping; (e) `complexity` ranking exact and `EXPLAIN QUERY PLAN` accepted; (f) v6 full-column carry-forward, deterministic order, no `metrics_version` carry; MCP parity | `cargo test p8c_` + `EXPLAIN` probe + `timing_assert.sh` | Remove the failed feature's command/flags before v6 release; withdraw every consumer flag/filter, MCP tool, journal/payload row, and skill line |
| G27 | P8b | `resolve --stdin` bytes and exit codes for found/ambiguous/missing; 1000-name/4096-byte/1MiB caps; empty input; no partial output on fatal error; MCP `isError` carrier; search filters exact; skill drift gate parses fenced blocks and consumer lines; static manifest passes without `KG_BIN` | `cargo test resolve_batch` + `KG_BIN=<binary> python3 ~/Workspace/personal/agent-skills/tests/skill_contract_drift.py` | Remove `resolve` and search filters; withdraw MCP/journal rows and skill lines |
| G28 | P9 | P9 addendum approved with pinned grammar crate/version, licenses, node kinds, pubspec containment; Dart fixture 20/20 call sites, 10/10 inheritance pairs, imports/parts exact; Flutter GT precision >= 80%, recall >= 70%, Wilson reported; build overhead <= 1.25x; no crash on the 2,025-file corpus; allowlist and write-path checks green | `cargo test dart_` + `python3 tests/scripts/dart_recall.py --pairs docs/baselines/dart-gt.jsonl ...` + `bash tests/scripts/timing_assert.sh ...` + `bash tests/scripts/allowlist_check.sh Cargo.lock` + `bash tests/scripts/write_path_check.sh` | Remove Dart grammar/extraction/walk extension, allowlist entry, generated suffix activation, MCP/journal rows, and skill lines |

Gate numbers are provisional until the owner merges Amendment 18. Each gate
is kill-able; no feature ships without its gate. If G26 is split, the
withdrawal list in §4.10 is the authority for what must be removed.

## 9. Test plan

1. **Fixtures.** One fixture per language per feature, exact expected stdout
   bytes, and at least one negative control (a call/import/inherit that must
   not resolve).
2. **Carry-forward.** A v6 build followed by an edit that changes one file
   preserves metrics, exported flags, generated flags, unresolved
   kind/line, import edges, and gap healing for a newly added module/base.
   One assertion per new column.
3. **Partial version.** A v6-stamped DB missing one new column rebuilds
   fresh with exit 0; query opens still exit 5 on a v5 DB.
4. **MCP.** Byte-parity fixtures for every new tool; malformed JSON and
   oversized frame tests remain green; `resolve` `isError` behavior pinned.
5. **Journal/payload.** `kg usage --verify` replays `implementers`,
   `imports`, `tests`, `api`, `complexity`, and `symbols <dir>` where
   applicable; report shapes produce no payload.
6. **Skill drift.** `tests/skill_contract_drift.py` extended to the new
   commands and flags; a behavior probe asserts the current `search` claim
   (identifiers + path + signature; no docstrings).
7. **Ground truth.** Frozen `import-gt.jsonl`, `inheritance-gt.jsonl`,
   `tests-gt.jsonl`, and `dart-gt.jsonl`, each with its construction
   protocol and freeze commit.
8. **Safety.** Seatbelt/network-namespace jobs unchanged; write-path gate
   covers any new write; no new child process.
9. **Performance.** Timing script extended per §7.5; build A/B on the
   2,025-file corpus; memory RSS recorded.

## 10. Skill contract changes

The kg repository ships `skills/kg-first/SKILL.md`; the agent-skills
repository ships `skills/code-index/SKILL.md`. This amendment requires both,
plus one-line pointers in the consumer skills.

### 10.1 `code-index` (agent-skills, 3000-byte budget)

Add the new commands only after they land. The table is at budget, so prune
older prose; one line per command:

```bash
kg context X               # definition + direct callers + direct callees
kg implementers X          # who inherits/implements X?
kg impact file.py --edges calls,imports,inherits --depth 3
kg imports file.py         # what does this file import (outbound)?
kg tests X                 # which tests reach X? (candidates)
kg complexity --min-cyc 10 # functions over the ceiling
kg api src/                # public surface of a package
kg files --path src        # indexed files in a directory
kg symbols src/ --outline  # directory structure with signatures
kg unresolved --names      # unresolved callees with lines
kg resolve --stdin         # batch name resolution
```

Correct the current `search` caveat: `kg search` matches symbol names,
path tokens, and signature tokens (Amendment 14); it does not index
docstrings, bodies, comments, config, or strings. Amendment 16 was reverted.

### 10.2 `kg-first` (kg repo)

Add routing rows for the same commands. Keep the existing staleness, exit-2,
cap, and grep-fallback rules. Add one calibration sentence: `implementers`,
`imports`, and `tests` are static and inherit the 10-48% resolved-call
coverage limit; a negative is not proof.

### 10.3 Consumer skills (agent-skills)

Add one line each, pointing to `code-index`; do not duplicate the command
table:

| Skill | Line to add |
|---|---|
| `code-review` | For callers, dependency consumers, duplicates, and per-function complexity, query `code-index` first; grep remains mandatory for config, docs, strings, and sibling repos. |
| `code-simplification` | Get callers and file metrics from `code-index` before changing a symbol; `dead` is a candidate list, not proof. |
| `code-minimalism` | Use `code-index callers` before declaring a helper local; use `imports`/`impact --edges imports` for cross-package consumers. |
| `deprecation-migration` | Use `code-index callers`/`impact --edges` for internal consumers; `__all__`, sibling repos, and external consumers still require grep/manual audit. |
| `codebase-contracts` | Use `kg symbols <dir> --outline`, `kg files`, and `kg api` for directory inventory; resolve claims in batch with `kg resolve`. |
| `python-patterns` | Use `kg imports` for module boundaries and `kg api` for public exports; call-graph traces use `callers`/`path`. |
| `python-testing` | Use `kg tests` as a candidate test set, then run the decisive targeted pytest command. |
| `doubt-driven-development` | Use `kg path` and `kg impact --depth` for blast-radius probes; state the static-resolution caveat. |

Flutter skills get no pointer until P9 lands; before that, the correct
instruction is "kg has no Dart index; use analyzer/grep".

### 10.4 Drift gate

Extend `tests/skill_contract_drift.py`:

- all new commands and flags exist in `kg --help`/subcommand help;
- the `code-index` byte budget still passes;
- a behavior probe asserts the search claim against the current binary
  (identifier/path/signature hit; docstring-only query does not match).

## 11. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Static resolution covers 10-48% of raw call sites; new queries imply more certainty than they have | Caveat line on every candidate surface; skill calibration; never label a negative as proof |
| Import edges add false positives through aliases, re-exports, and path mapping | Precision gate G24; unresolved imports are omitted, not guessed |
| Test impact creates false confidence | `tests` is a candidate list; G25 seeds direct and indirect cases; caveat names fixtures and indirect calls |
| Metric formulas drift across languages | Pin formulas in F9; `meta.metrics_version` invalidates the fast path; hand-computed fixtures |
| Schema v6 rebuild cost and carry-forward bugs | One additive bump; complete partial-version probe; per-column carry-forward assertions |
| Dart grammar adds a dependency and build cost | Owner approval; allowlist update; P9 gate; kill action removes the grammar |
| Skill budget overflow | Prune `code-index` prose; one line per command; consumer skills only point |
| Uncommitted `impact` grouping change collides with this proposal's IDs | F4 absorbs the grouping; owner resolves Amendment 18/19 at merge |
| Import/inherit gaps do not heal on the fast path | `unresolved_calls.kind` covers call/import/inherit; gap query extended; G24/G26 tests |

## 12. Open questions for the owner

- **OQ6**: Is a Dart grammar dependency approved for P9? If not, P9 is
  deferred and the Flutter skills stay on analyzer/grep.
- **OQ7**: Should cognitive complexity be in scope? Proposal: no; cyclomatic,
  branches, nesting, and effective LOC are pinned and sufficient for the
  skill thresholds. Cognitive complexity needs a specified formula and a
  separate gate.
- **OQ8**: Should `api --include-private` also respect `--exclude-generated`?
  Proposal: yes; the flags are independent.
- **OQ9**: Should `tests` include tests that only import the target but never
  call it? Proposal: no in v1; add `--include-imports` only if a measured
  workflow needs it.

The command-surface decisions in rev 1 (OQ6/OQ7) are resolved by this
revision: `implementers` is a new command; `impact` keeps `calls` as its
default edge set.

## 13. Implementation plan (post-approval)

1. **Canonical merge.** Add Amendment 18 and G23-G28 to
   `docs/designs/2026-10-31_kg-rust-code-index.md`. Do not implement before
   this merge; kg is spec-first. Resolve the in-flight `impact` grouping ID
   first.
2. **P8a (schema v6 + query + symbol extraction).** `schema.sql`,
   `src/db.rs` (v6 columns, index, partial probe, carry-forward),
   `src/extract.rs` (trait/interface symbols, Rust trait-impl inherits),
   `src/query.rs` (`implementers`, `dead` predicate,
   `impact --edges/--depth/--context/--explain`, `--path`,
   `files`, `symbols <dir>`), `src/main.rs` dispatch/journal args,
   `src/cli.rs`, `src/mcp.rs`, `src/journal.rs`; tests and skill edits.
   Commit slices: v6 schema, F1, F2+F4, F3.
3. **P8b (extraction + queries).** `src/extract.rs`: import edges and
   unresolved import rows; `src/query.rs`: `imports`, `tests`, `resolve`,
   search filters; `src/main.rs`, `src/cli.rs`, `src/mcp.rs`,
   `src/journal.rs`; fixtures; `import-gt.jsonl`, `tests-gt.jsonl`; skill
   edits. Commit slices: F7, F8, F6.
4. **P8c (v6 population + queries).** `src/extract.rs`: metrics, exported,
   generated, unresolved kind/line, `metrics_version`; `src/query.rs`:
   `complexity`, `api`, `unresolved --names`; carry-forward and
   partial-version tests; old-index exit 5. No new schema stamp.
5. **P9 (Dart, separate amendment).** A separate P9 addendum must pin the
   grammar dependency and node kinds before code. It is not part of this
   amendment's implementation approval.
6. **Docs.** README usage block, CHANGELOG entries, EVALUATION updates for
   every measured gate, `skills/kg-first/SKILL.md`.
7. **Gates.** Run each gate on the shipped binary with the command recorded.
   A failing gate blocks the phase; no partial release.

## 14. Rejected alternatives

| Alternative | Why rejected |
|---|---|
| LSP or language-server integration | Long-lived servers violate the no-child-process invariant; static pointers plus grep fallback are the existing honest boundary |
| Lexical reference index (`kg refs`) over source, docs, and config | Grep already wins on cost and completeness for lexical search; kg's value is resolved structure, not token search |
| Cross-repo index | Explicitly out of scope; per-repo qualnames and safety boundaries would change |
| Framework route mapping (`go_router`, React Router) | Framework-specific semantics, not a code-index concern; remains a skill/manual task |
| Runtime test execution or coverage | kg is static and read-only; `tests` is a candidate surface only |
| PageRank or semantic ranking for `complexity` | Unmeasured at this scale; deterministic threshold ranking is sufficient |
| Separate `kg dependents` command | `impact --edges` covers the same question and keeps one surface |
| JSON output now | LLM consumers use text; JSON expands the contract; defer until a machine gate needs it |
| Revision-aware index now | Source materialization and storage are a separate phase; grep at a revision covers the current review workflow |
| Cognitive complexity in v1 | No pinned formula; adds a metric-definition debate without a gate |

## 15. Definition of done

- Amendment 19 and G23-G28 are merged into the canonical spec; Amendment 18
  and G22 remain unchanged.
- Each P8a-P8c feature either passes its gate on the shipped binary or is
  explicitly killed with the full withdrawal checklist recorded. P9 is a
  separate addendum and is out of scope here.
- New commands have exact-byte fixtures, MCP parity, journal tests, payload
  tests only where §4.7 says yes, and skill drift coverage.
- `dead` reports zero inherited/imported false positives on the fixture,
  has a positive dead fixture, and preserves the AMBIGUOUS-call suppression.
- `impact --edges` returns import-only and inheritance-only consumers;
  default impact bytes are unchanged; depth > 1 is monotone with depth 1.
- `kg tests`, `complexity`, and `api` meet their gate criteria (exact bytes,
  candidate precision/recall, metric formulas), not a vague "answer the
  skill question".
- Schema v6 is additive, carries forward every new column, recomputes the new
  columns on fresh and fast-path builds, rebuilds a partial v6 index, and
  exits 5 on v5 query opens.
- README, CHANGELOG, EVALUATION, `skills/kg-first/SKILL.md`, the agent-skills
  `code-index` skill, and the consumer lines in §10.3 are updated.
- The gate harness files in §7.6 exist and run; `timing_assert.sh` exits
  non-zero on budget overrun.
- No safety invariant is weakened; no new dependency except the approved Dart
  grammar in the separate P9 addendum.
