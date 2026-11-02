# R10 — P8 Four-Agent Evaluation

- Baseline: kg `c39e21f`, binary `target/release/kg` v0.2.1, schema v6.
- Date: 2026-11-02.
- Agents: A1 inheritance/impact, A2 imports/tests/unresolved, A3
  metrics/API/generated/schema, A4 latency/contract/utility/safety. Four
  independent stateless reviewers; two concurrent per wave; no reviewer saw
  R8/R9 or another agent's findings.
- Overall verdict: **BELOW CEILING**. F5, F8, F11, and schema v6 are at
  ceiling. F1, F4, F6, F7, F9, F10, latency, contract, utility, and safety
  remain below.

## 1. Verdict by feature

| Feature | Verdict | Reason |
|---|---|---|
| F1 inheritance | BELOW | Rust supertraits never emitted; tuple/pointer impls create false edges; scoped/grouped/aliased Rust trait paths and TS interface methods missing; unresolved `inherit` rows missing |
| F4 impact | BELOW | Depth-cap footer fires on exhausted BFS; `--context` tags module/class/lambda sites and omits calls-only caveat |
| F5 unresolved | AT CEILING | kind/site line, staleness, cap units, call-only scoped default verified; only unlocated-row stderr count missing (Suggestion) |
| F6 resolve/search | BELOW | `resolve` missing ambiguous cap line; MCP ignores 4096/1 MiB bounds; `search --exact` falls back to FTS |
| F7 imports | BELOW | Star suppression is file-wide; TS `export from`/`require`/dynamic import missing; Rust grouped/self/super/wildcard/single-segment use and `mod.rs` mapping missing; deletion/restore not healed |
| F8 tests | AT CEILING | Direct/indirect/depth/file/seed/method/`./` behavior correct; caveat misses one pinned limit (Suggestion) |
| F9 metrics | BELOW | `effective_loc` docstring heuristic swallows body lines; `generator_function` leaks; Rust attributes counted as code; complexity tie-break `file_id` flips fresh/carried |
| F10 api | BELOW | Rust `pub fn` in inline `mod` classified as method and never exported; Python `__all__` raw text scan; TS export substring/private members; impl-before-type export pass |
| F11 generated | AT CEILING | Suffix/marker set, 5-line boundary, negative controls, carry, six filters, `stats generated=` verified |
| Schema v6 | AT CEILING | All 8 columns carried byte-identical; partial column/index drop rebuilds; v5 open exit 5; `metrics_version` invalidation verified |
| Latency | BELOW | Fresh build best 2.94 s vs 2.91 s budget; `tests` and `impact --depth>1` materialize full sets |
| Contract | BELOW | `search --exact`, path rejection, impact caveat/footer, symbols no-results, resolve/MCP caps, complexity/api payload/staleness/order deviate |
| Utility | BELOW | GT/recall harness absent; EVALUATION/consumer skills not updated; README/CHANGELOG false commands; drift gate only inline backticks |
| Safety | BELOW | `resolve` reads unbounded stdin before 1 MiB check; `search`/`api` accept absolute/`..` paths; no write/network/child/hook/unsafe found |

## 2. Critical findings

| # | Feature | Finding | Evidence | Fix |
|---|---|---|---|---|
| 1 | F1 | Rust supertraits are not extracted: `trait_item.bounds` passes through `collect_heritage`, which matches `"bounds"` not grammar `trait_bounds`; `trait Derived: Root` emits no edge | `implementers Root` exit 1; `dead` reports Root | Add `trait_bounds`; add supertrait positive fixture |
| 2 | F1 | `impl_type_name` accepts any nested `type_identifier`, so `(U,U)`, `*const V`, `[U;N]` create false inherits edges and suppress real dead symbols | `implementers Base2` lists `U`/`V`; `impl OnlyTuple for (Z,Z)` makes `Z` an implementer | Accept only nominal `type_identifier`/`generic_type`/`reference_type`; reject tuple/array/pointer; unresolved row |
| 3 | F7 | TS `export ... from`, `export * from`, dynamic `import()`, and `require()` are not imports; `require` pollutes unresolved calls | four fixtures `kg imports` exit 1; `unresolved` has `call require` | Dispatch `export_statement` source; detect literal `import()`/`require()`; one edge, no call row |
| 4 | F7 | Rust use-tree forms are not extracted: grouped, aliased, wildcard, `self`/`super`, single-segment; `mod.rs`/`lib.rs` mapping is wrong | `use crate::a::{...}`, `use super::...`, `mod inner;` all unresolved; `src/inner/mod.rs` is `src.inner.mod` | Traverse `use_list`/`scoped_use_list`/`use_as_clause`/`use_wildcard`; map module roots; collapse `mod.rs`/`lib.rs` |
| 5 | F9 | `effective_loc` docstring heuristic is wrong: a closing `"""` is treated as an opening delimiter and swallows the rest of the body; `r"""..."""` counted as code; non-doc multi-line string excluded | `outer` 11 lines with 3 doc lines + 1 comment => spec 7, actual 2 | Use the parsed first body `expression_statement` string + comment node byte ranges |
| 6 | F10 | Rust `pub fn` inside inline `mod` is extracted as `method` under the module symbol; `parent_exported` is false, so `api` returns zero | `mod inner { pub fn inner_exported() {} }` -> `kg api` exit 1 | Separate module scope from type scope; `pub` function exported regardless of module nesting |
| 7 | F10 | Python `__all__` is a raw text scan: tuple scans to EOF, comments suppress exports, non-literal restricts instead of fallback | tuple/comprehension/binop/comment fixtures | Parse only a module-level literal list/tuple of strings; otherwise fallback |
| 8 | F6 | `search --exact` runs FTS first and returns fuzzy hits when no exact identity exists | `search impac --exact` returns 10 `impact*`; `search Shap --exact` returns Shape/Circle/Square | Exact mode skips FTS entirely; exact rows or exit 1 |
| 9 | Gate | §7.6 GT/recall harness absent: `import_recall.py`, `tests_recall.py`, `inheritance-gt.jsonl`, `import-gt.jsonl`, `tests-gt.jsonl`; `p8a_`/`p8c_` select zero tests | `ls tests/scripts`, `grep fn p8a_` | Commit harness/GT or withdraw gate claims |
| 10 | Safety | `resolve --stdin` reads entire stdin before the 1 MiB check; 32 MB input -> 38.5 MB RSS | `read_to_string` then length check | Bounded read `take(1 MiB + 1)` or streaming line read |

## 3. Important findings (selected)

| Area | Finding | Evidence |
|---|---|---|
| F4 | Depth-cap footer fires when BFS exhausted; `frontier` from `break` used as truncation flag | `impact a.py --depth 2` prints footer with only direct dependents |
| F4 | `--context` tags module/class/lambda sites as `[in module]`; calls-only `--context` omits caveat | `impact c.py --context` / `--sites --context` probes |
| F1/F7 | One `from x import y` can still show duplicate `--sites` rows (module + symbol edge) | `b.py:1` twice, `(2 sites)` |
| F6 | `resolve` ambiguous candidates capped at 50 without `showing 50 of 60 candidates` | 60 `dup` fixture |
| F6 | MCP `resolve` ignores 1000/4096/1 MiB bounds | 1001 names, 5000-byte name accepted |
| F7 | Star suppression is file-wide, not per statement; drops legitimate `import pkg` edge | `import pkg` + `from pkg import *` |
| F7 | AMBIGUOUS star edge can self-reference in `pkg/__init__.py` | `src_id=dst_id` |
| F7 | `from pkg import mod` emits parent `pkg` edge plus `pkg.mod`; F7 pins only `pkg.mod` for that form | fixture |
| F7 | Default TS import binds `module.default` but emits local-name candidate, causing false unresolved | `unresolved ts.default_src.A` |
| F7 | Deletion/restore does not heal: carried edge dropped, no unresolved row, re-add not matched | `pkg/later.py` delete/restore probe |
| F9 | TS `generator_function` leaks decisions into enclosing function | `outerGen cyc=2` vs pinned 1 |
| F9 | TS const-arrow metrics use arrow range but declarator line_start/end | multiline arrow `loc=3` vs 4 |
| F9 | `complexity` tie-break `file_id` flips fresh/carried; order must be `path` | `pkg.py` vs `pkg/__init__.py` |
| F9 | `complexity --min-cyc 0` lists zero-metric trait signatures | `lib.Shape.area` |
| F10 | TS export detection is substring on declaration line; private members exported | `"export x"` string, `private helper` |
| F10 | Rust impl-before-type export pass misses `pub` methods | `impl Widget` before `pub struct Widget` |
| F10 | `api` does not validate `..`/absolute paths | `api ..` exit 1, not 2 |
| Latency | Build best 2.94 s vs 2.91 s (1.25x frozen 2.33 s) | eight `build --force` runs |
| Latency | `tests` collects all reachable tests, no SQL LIMIT+COUNT | code inspection |
| Latency | `impact --depth>1` materializes/sorts all dependents | code inspection |
| Contract | `symbols <dir>` no-result string is `path not indexed`, not `0 symbols in '<dir>'` | `symbols nosuchdir` |
| Contract | `complexity`/`api` no payload; no staleness for complexity; `context` max 50 but clamps 10000 | code/probes |
| Utility | README/CHANGELOG claim `resolve < names` (needs `--stdin`), 2.33 s build (actual 2.94), payload for all pointer queries (complexity/api no), search path validation (absent) | docs probes |
| Utility | EVALUATION not updated for P8/schema v6 | `git diff` empty |
| Utility | `timing_assert.sh` adds ~30 ms Python startup per run; `set -e` aborts on exit 1 with no measurement | `imports` measured 40 ms vs direct 9.2 ms |
| Utility | drift gate parses inline backticks only, skips without `KG_BIN`, does not check consumer lines | source + run |
| Utility | Skills missing `resolve` row and eight §10.3 consumer pointers | grep |

## 4. At-ceiling summary

- F5 unresolved names: correct kind/site/staleness/call-only scoped default; only unlocated-row stderr count missing.
- F8 tests: correct kind/seed/depth/file/`./` behavior; only caveat wording missing.
- F11 generated: exact detection, negative controls, carry, filters, stats.
- Schema v6: exact carry, partial rebuild, version gate, metrics version.

## 5. Fix order

1. F1 supertraits + impl target nominal-type guard.
2. F9 docstring/effective_loc + generator leak.
3. F10 Rust inline-mod exports + Python `__all__` AST + TS export AST.
4. F7 TS export/require/dynamic + Rust use-tree/module mapping + star per-site + deletion healing.
5. F6 exact-only search + resolve/MCP caps + ambiguous cap line.
6. F4 footer/context caveat.
7. Latency: bounded tests/impact result sets; build overhead profile.
8. Contract/utility: path validation, symbols no-result, payload/staleness/order, docs/skills/drift gate.
9. Gate harness/GT sets.

## 6. Disposition

No fix was applied during this evaluation. All findings above are OPEN.
R10 is the baseline for the next fix cycle; re-run the four agents after the
fixes, with the same independence rules.

## 7. Fix cycle 1 (2026-11-02)

Fixed from R10:

| Finding | Fix | Probe |
|---|---|---|
| Rust supertraits | `trait_bounds` added to heritage containers | `trait Derived: Root`; `kg implementers Root` -> `Derived` |
| Tuple/pointer impl false edges | `impl_type_name` accepts only nominal type/generic name/reference | `impl Root for (u8,u8)`; `implementers u8` -> not in index |
| TS export-from/require/dynamic | `export_statement` source -> import edge; literal `require()`/`import()` -> one edge, no call row | `imports re.ts` -> 4 `util` rows; unresolved empty |
| TS default import false unresolved | default binding no longer emits local-name symbol candidate | default import probe |
| Python `__all__` | AST literal list/tuple only; non-literal/comment falls back | tuple `__all__` -> only alpha; comment fallback -> gamma |
| Rust inline-mod exports | `pub` function in module scope exported regardless of module parent | `kg api lib.rs` includes `lib.inner.inner_exported` |
| TS export/private | AST export flag; private members excluded | `"export"` string and `private helper` controls |
| Python docstring LOC | AST first body string range replaces text heuristic | multi-line docstring `loc=5` pinned |
| Generator leak | `generator_function` added to nested callables | generator branch no longer counted |
| `search --exact` | exact mode skips FTS; exact rows or exit 1; path/line order | exact-miss probe |
| Depth footer | set only when final level discovers nodes | exhausted BFS probe |
| `--context` caveat | context forces extended caveat | calls-only context probe |
| Resolve caps | bounded stdin read; MCP 1000/4096/1 MiB; candidate cap line | 60-duplicate and 5000-byte MCP probes |
| Path validation | `search --path` and `api` reject `..`/absolute | probes |

Remaining open from R10:

- Rust `use_list`/`scoped_use_list`/`use_as_clause`/`use_wildcard`/`self`/`super`
  and `mod.rs`/`lib.rs` module mapping.
- Star suppression is file-wide, not per import statement.
- TS relative resolution still uses module strings, not pinned extension order.
- Deleting a destination still drops carried edges without an unresolved row.
- `finalize_exports` is single-pass: `impl Type` before `pub struct Type`
  can miss exported methods.
- `complexity` ties on `file_id` (index-supported) rather than `path`.
- Build best 2.94 s vs 2.91 s budget.
- Gate harness/GT sets; docs/skills drift.

Verification after cycle 1: `cargo fmt --check` clean, `cargo clippy --all-targets -D warnings` clean, `cargo test --locked` 23 unit + 75 CLI + 26 fixture + 6 MCP pass.
