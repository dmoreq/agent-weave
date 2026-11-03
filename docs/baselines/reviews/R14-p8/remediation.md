# R14 remediation facts

Commit `5706dff` fixes or documents the R13 IMPROVE packets. This file states
what changed; it does not vote.

## Contract and correctness

- `resolve` honors `--no-journal`; the CLI updates journal arguments after
  parsing stdin and records the parsed name count.
- MCP `resolve` rejects a missing names array, non-string entries, and
  blank/comment-only input; it records results on success.
- `journal::set_args` replaces the armed argument object without re-arming.
- Tests: `resolve_honors_no_journal_and_records_parsed_count`,
  `resolve_empty_names_and_non_strings_are_tool_errors`.

## Inheritance and calls

- Nested Python function calls resolve lexically through enclosing function
  scopes, stopping before class/module scope. This removes false `dead`
  candidates such as `scripts/build_knowledge_base.py:366`.
- Test: `nested_function_calls_resolve_and_dead_keeps_inner_alive`.
- TS `tsconfig` `paths` aliases remain unresolved by design; a zero-result
  `implementers` query with matching unresolved inherit sites now reports the
  count in the detail line. Test: `inheritance_ts_path_alias_is_pinned_unresolved`.
- `dead` caveat names type references and attribute references as
  false-positive classes.
- GT: 17 inheritance rows (15 vihap, 2 finguard; 6 cross-file), 17/17 recall.
- Probe: 50 vihap dead candidates, 6/50 had zero external textual references
  (`docs/baselines/dead-precision-2026-11-03.md`).

## Impact and tests latency

- Removed full inbound `ORDER BY`; bounded heap selects output.
- Added `edges_dst_cover(dst_id, kind, src_id, site_line)`.
- Direct impact level groups by caller path when `--context`/`--explain` are
  absent.
- BFS stops when all reachable files (impact) or all callable symbols
  (`tests`) are covered. The impact universe is read from exact per-kind
  source-file meta counters (module-level callers included); `tests` returns
  no-results immediately when no test-shaped symbol exists.
- Measurements on the 1.37M-edge hub corpus (`/tmp/kg-r13-l3/edges1m-sub`):
  `impact --depth 3` 110.0-144.3 ms best-of-7 (budget 150 ms);
  `impact --edges all` 146.9 ms; `tests --depth 10` 9.3-9.9 ms via the
  no-test fast path (budget 200 ms).
- 3.1M stress: 269.4 ms / 391.8 ms (outside the 1M budget).
- 987k-symbol representative: 5.1 ms / 5.6 ms.
- Raw method: `docs/baselines/p8-latency-2026-11-03.md`.

## API and generated files

- Zero-result `api` on a file with import edges now says module declarations
  and re-exports are not listed.
- F10 output contract documents the appended `[signature]`.
- New export GT: Python `__all__`/underscore rules, Rust `pub`/`pub(crate)`,
  directory scope, crate-root omission detail.
- F11 negative controls: `gen/` and bare `DO NOT EDIT` are not generated;
  `@generated`, `Code generated`, and `_pb2.py` are; complexity/api/dead
  `--exclude-generated` filters are covered.
- Tests: `p8c_api_export_gt_python_rust_and_crate_root_caveat`,
  `p8c_generated_negative_controls_and_filters`.

## Verification

- `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets -- -D warnings`,
  `cargo test --locked`: 197 tests, 0 failures.
- GT at the current source: imports 40/40 recall, 183/183 precision; tests
  20/20 recall, 2679/2679 precision; inheritance 17/17.
