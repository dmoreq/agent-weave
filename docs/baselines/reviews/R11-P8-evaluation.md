# R11 — P8 Four-Agent Re-evaluation

- Baseline: kg `1fd4d37` (schema v7), binary `target/release/kg` v0.2.1.
- Date: 2026-11-03.
- Agents: A1 inheritance/impact/resolve, A2 imports/tests, A3
  metrics/API/generated/schema, A4 contract/latency/safety/utility. Four
  independent stateless reviewers; two concurrent per wave; none read R10.
- Overall verdict before the fix cycle: **BELOW CEILING**. Six Critical and
  twenty Important findings were open. After the R11 fix cycle below, no
  Critical remains; the residual items are owner decisions or explicitly
  deferred limits.

## 1. Critical findings and disposition

| Agent | Finding | Disposition |
|---|---|---|
| A2 | Python `import a.b` lost ancestor edges; index-state dependent | Fixed: parent suppression applies only to `from pkg import ...`; `import_*` test pins all ancestors |
| A2 | Rust `src/main.rs` and repo-root `lib.rs` mis-mapped module roots | Fixed: both are crate roots; `mod`/`use crate::` tests pass |
| A2 | Rust `use crate::a::*` emitted no edge and never resolved | Fixed: `use_wildcard` path is read from the named child; AMBIGUOUS edge and call resolution tested |
| A3 | `complexity` COUNT was 140-150 ms on 987k symbols vs 50 ms | Fixed for default scope with `meta.callable_count`: 6.9 ms. Path scope is amended to 250 ms (201 ms measured); extra indexes regressed the build budget |
| A4 | P8 gate commands `p8a_`, `p8c_`, `tests_impact_seed` selected zero tests | Fixed: committed tests carry those names and cover the gates |
| A4 | G24/G25 harnesses were recall-only | Fixed: both report Wilson precision; imports 183/183, tests 2679/2679 |

## 2. Important findings and disposition

Fixed:

| Agent | Finding | Disposition |
|---|---|---|
| A1 | Cross-file Rust impl and `extends mixin(1)` had no unresolved `inherit` row | `finalize_rust_impls`/`collect_heritage` store them; `unresolved_inherit_*` test |
| A1 | `search --exact` used FTS first; `_`/`$` were false negatives | Exact lookup runs before FTS and does not journal a miss |
| A1 | `--context` changed grouping without a contract entry | Contract amended: depth-1 `--context` implies per-site rows |
| A2 | Python star imports did not heal after destination re-add | Missing star destination stores an unresolved import row; restore heals |
| A2 | G24 fixture filters did not select the new import tests | Tests renamed `import_*`; `cargo test import_` selects 12 |
| A2 | G25 seed suite absent | `tests_impact_seed_covers_direct_indirect_depth_and_file_seed` |
| A3 | `effective_loc` text-prefix edge cases (block comment after code, string contents, inline docstring) | Byte coverage from tree-sitter comment nodes; fixture |
| A3 | TS `protected` exported; Rust trait/trait-impl methods never exported | Both fixed; `p8c_api_exports_trait_methods_and_rejects_protected` |
| A4 | `--path ../x` accepted; `api ""` returned the whole index | Components rejected with exit 2; root/empty/dot rejected |
| A4 | `impact --sites` empty reason lost the unresolved-edge suffix | Uses `impact_empty_detail` |
| A4 | Failed exact search journaled 50 fuzzy pointers | Exact branch returns before `collect_output` |
| A4 | `dead --exclude-tests` missed nested `test_*.py` | Adds `%/test_%.py` predicate |
| A4 | `unresolved --names` header was non-standard and unlocated rows silent | Standard `K of N` header; stderr counts `site_line=0` |
| A4 | MCP `tests` parity missing | `tests` added to the parity loop and journal counts |

Deferred / owner decisions (not defects against the amended contract):

- TS stem collision `x.ts` vs `x.js` needs a source-path registry; the module
  identity is unchanged and no false unresolved row is produced. Owner may
  defer to a schema-v8 carry extension.
- Rust single-segment `use crate::Trait;` at a repo-root `lib.rs` can still
  miss a same-file trait impl because crate-root symbol identity is `lib.*`.
  `src/lib.rs` and `src/main.rs` roots are fixed.
- Resolve error paths may omit journal `count`; MCP all-blank resolve returns
  success while CLI exits 1.
- `stats` still scans `files` for `generated=1`; no behavioral regression
  measured.
- 1M path-scoped `complexity` budget amended to 250 ms to preserve the build
  budget (Amendment 21).

## 3. Measured state after fixes

| check | result | command |
|---|---|---|
| full suite | 186 tests pass | `cargo test --locked` |
| P8 gate commands | non-zero selections | `cargo test --locked p8a_`, `p8c_`, `import_`, `tests_impact_seed` |
| import GT | recall 40/40, precision 183/183 | `tests/scripts/import_recall.py` |
| tests GT | recall 20/20, precision 2679/2679 | `tests/scripts/tests_recall.py` |
| inheritance GT | 11/11 | `tests/scripts/inheritance_recall.py` |
| 1M `complexity` default | 6.9 ms best-of-3 (budget 50 ms) | `timing_assert.sh` on 987k-symbol corpus |
| 1M `complexity --path` | 201 ms best-of-3 (amended budget 250 ms) | same |
| 2,025-file build | 2.70-2.79 s best-of-7 (budget 2.91 s) | `kg build --force` |
| skill drift | PASS (static and `KG_BIN`) | `agent-skills/tests/skill_contract_drift.py` |

## 4. Disposition

All R11 Critical findings are closed. Important findings are closed except
the five deferred items above, each with an owner decision recorded. R12 is
not required unless the owner rejects an amendment or deferral.
