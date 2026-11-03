# R12 — P8 Four-Agent Re-evaluation

- Baseline: kg `69ad161` (schema v7 + Amendment 21), binary
  `target/release/kg` v0.2.1 rebuilt at review start.
- Date: 2026-11-03.
- Agents: A1 inheritance/impact/resolve, A2 imports/tests, A3
  metrics/API/generated/schema, A4 contract/latency/safety/utility. Four
  independent stateless reviewers; two concurrent per wave; none read R10/R11.
- Overall verdict before the fix cycle: **BELOW CEILING**. One Critical and
  six Important findings were open. The fix cycle below closes the Critical
  and all six Important findings; remaining items are owner decisions.

## 1. Critical findings and disposition

| Agent | Finding | Disposition |
|---|---|---|
| A1 | Carried `trait`/`interface` symbols were filtered from resolution, so an incremental Rust `impl Trait for T` stayed unresolved forever | `load_carried` now maps both kinds; `incremental_carries_trait_and_qualified_impl_targets` pins the fast-path edge |

## 2. Important findings and disposition

| Agent | Finding | Disposition |
|---|---|---|
| A1 | `impl crate::Shape for T` produced no edge and no unresolved row | `scoped_type_identifier` is now a dotted chain, and `crate::` impl targets map through the crate root; covered by the same test |
| A2 | Repo-root TS `index.*` could not be imported and emitted an empty-name unresolved row | `resolve_ts_module` returns the root module when the collapsed path is empty; covered by `root_ts_index_and_python_root_parent_imports_resolve` |
| A2 | Bare `from .. import X` from a first-level package did not resolve | Empty parent no longer emits a bogus edge; root children bind through a root module alias; covered by the same test |
| A2 | Rust inline `mod` members were kind `method` and excluded from bare-name/import resolution | Direct children of a module are registered in `by_mod_name` regardless of the method kind; inline-mod import/call test |
| A3 | Non-`pub` inherent methods were exported when the type had any trait impl | Trait-impl method identities are tracked per impl block; `rust_trait_impl_export_does_not_leak_inherent_methods` |
| A3 | `pub fn` in an inline `mod` was never exported | Public methods whose parent is a module are exported; covered by the inline-mod test |
| A4 | A pre-manifest v7 index plus a no-op build made `complexity` falsely empty | `probe_old` requires `callable_count`; the stale index rebuilds and the manifest is rewritten; `pre_manifest_index_rebuilds_instead_of_empty_complexity` |

## 3. Suggestions and owner decisions

Not fixed; recorded for owner action:

- Impact `[ambiguous]` tagging is unreachable because impact joins resolved
  destinations and AMBIGUOUS edges have `dst_id=NULL`. No wrong output was
  observed; choose whether to remove the dead branch or extend impact to
  ambiguous rows.
- `resolve_names` re-resolves duplicate inputs. Cost measured at 26 ms for
  1,000 names; memoization is optional.
- `dead` reports a never-implemented trait while `implementers` exits 1; both
  are spec-conformant, but a docs line could reconcile the signals.
- Qualified std trait impls (`impl std::fmt::Display for T`) will now store
  unresolved rows; decide whether known-external roots should be filtered.
- `files.unresolved` remains call-only per F5; inherit/import unresolved noise
  is not reflected in the build summary.
- `tests_recall.py` precision is name-shape based, not source-verified; the
  import harness is source-verified.
- Non-UTF-8 `resolve --stdin` is internal exit 6 rather than usage exit 2, and
  MCP non-string `names` items are silently dropped instead of `-32602`.
- §7.5's 50 ms complexity row should be worded as the default invocation; the
  manifest-backed default is 6.7 ms, while non-default COUNTs are slower.

## 4. Measured state after fixes

| check | result | command |
|---|---|---|
| full suite | 191 tests pass (25 lib + 25 bin + 109 CLI + 26 fixture + 6 MCP) | `cargo test --locked` |
| fmt/clippy | clean | `cargo fmt --all -- --check`; `cargo clippy --all-targets --locked -- -D warnings` |
| import GT | recall 40/40, precision 183/183 | `tests/scripts/import_recall.py` |
| tests GT | recall 20/20, precision 2679/2679 | `tests/scripts/tests_recall.py` |
| inheritance GT | 11/11 | `tests/scripts/inheritance_recall.py` |
| 1M complexity default | 6.7 ms best-of-3 (budget 50 ms) | `timing_assert.sh` on 987k-symbol corpus |
| 1M complexity path | 216.2 ms best-of-3 (amended budget 250 ms) | same |
| build budget | not conclusive during review: host load average was 18.4/8 cores and the 2,025-file build measured 3.24-4.5 s; an earlier quiet-host best-of-7 was 2.70-2.79 s vs 2.91 s | `kg build --force` |

## 5. Disposition

The R12 Critical is closed, and every R12 Important finding has a regression
test. The remaining items above are suggestions or owner decisions, not open
defects. R13 is not required unless the owner rejects one of these decisions
or requests another full four-lens pass.
