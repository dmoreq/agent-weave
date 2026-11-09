# Long Method and parameter-clump debt (accepted)

- Date: 2026-11-06
- Revision: `32f9e96` plus the uncommitted review fixes (line numbers are
  working-tree line numbers)
- Follow-up: `90e76b8` plus the current review fixes
- Scope: `src/**/*.rs`, non-test functions
- Origin: third code review of `agent-weave` at `32f9e96`
- Status: accepted debt

## Decision

The `AGENTS.md` §3 complexity caps remain binding for new and changed code.
The existing breaches listed below are recorded as debt rather than split in
the review-fix change. Seven high-value functions were split in the same
change; the rest are flat dispatch tables, data tables, phase orchestrators,
or resolver functions whose split needs behavior characterization first.

This is a recorded decision, not a claim that the §3 acceptance is met.

## Measurement

- Effective lines: non-comment physical lines, excluding `#[cfg(test)]`
  modules. A source line counts when it is non-blank and does not start with
  `//`, `/*`, or `*`; multi-line SQL string content therefore counts.
- Branch proxy: occurrences of `if`, `else if`, `for`, `while`, `match`,
  `loop`, `&&`, `||`.
- Nest proxy: maximum brace depth.
- Counts before this change: Long Method 69, branch proxy 26.
- Counts after this change: Long Method 63, branch proxy 24.

Functions split in this change:

| Function | Before | After |
| --- | ---: | ---: |
| `query/callgraph.rs::path_bfs` | 97 | 46 |
| `query/analysis.rs::dead` | 93 | 53 |
| `query/analysis.rs::unresolved` | 96 | 45 |
| `query/analysis.rs::complexity` | 87 | 46 |
| `build.rs::extract_units` | 90 | 62 |
| `build.rs::run_build` | 97 | 69 |
| `extract/resolve.rs::resolve_call_sites` | 90 | 80 |

A follow-up dedup pass (shared report cap footer, scope prologue, module
symbol row, and Python prefix edges) reduced the current Long Method count
from 65 to 63; the branch-proxy count stayed at 24.

A second follow-up dedup pass (current review) removed the duplicated
cap/staleness contract, import-edge literals, impact row mappers, and the
single-caller `scope_sql` helper. Net effect: 16 files, +413/-463 lines;
`jscpd src --min-tokens 50` fell from 13 exact clones / 99 lines / 1.08% to
5 clones / 41 lines / 0.51%. The Long Method count is 58 by the same proxy;
no new Long Method row was introduced.

A third follow-up dedup pass removed the duplicated path-scope prologue and
prefix predicate, the per-language `Assignment`/`ParamType`/TS module-edge
literals, the duplicated build skip notice, the MCP notification guards, the
parallel `exit_code`/`class` matches, and the duplicated SQL join blocks.
Net effect relative to the second pass: 19 src files, +491/-770 lines
(net -279); `jscpd src --min-tokens 50` fell to 4 exact clones / 31 lines /
0.35%. No accepted row was removed: `files`, `unresolved`, `impact_deep`, and
`record_import_ts` shrank but stay above the cap where already recorded.

## Why this is accepted

1. Flat dispatch tables (`visit_node`, `run_graph_query`,
   `run_analysis_query`, `from_cli_graph`, `journal_args_*`) are one line per
   case. Splitting them adds indirection without reducing review risk.
2. Data tables (`write_meta_counts`, `journal_args_*`) are ordered
   `set_meta`/tuple maps; the length is the data.
3. Phase orchestrators (`run_build`, `run_mem_write`) already call named phase
   helpers. The remaining length is argument passing, not hidden policy.
4. Deep resolver functions (`resolve_class_of_chain`, `resolve_name_callee`,
   `resolve_attr_callee`) need a behavior-characterization pass before any
   split, because branch order is load-bearing.
5. The three long parameter lists are cohesive input bags, not accidental
   clumps; replacing them with structs adds a type without removing a concept.

## Overlapping debt

- The six-parameter `(conn, root, name, limit, depth, explain)` clump on
  `callers`/`callees` is the same surface as the accepted S2 dispatch debt in
  `docs/archive/reports/2026-11-06_s2-dispatch-edit-sites.md`.
- The five-parameter `(node, src, fx, ctx, work)` clump has an existing
  `Walker` struct (`src/extract/walk/mod.rs`). A future walk refactor should
  extend that struct instead of adding another context type.

## Re-open conditions

Revisit this record when any of these occurs:

- A listed function must change behavior; split it before the change.
- A listed function grows past twice the hard cap with no written justification.
- A third command surface or a new walk state makes the parameter clump grow.
- A new command or language adds another one-line dispatch arm to the same
  table.

## Appendix A: Long Method (>50 effective lines)

| Location | Function | Effective | Branch | Nest |
| --- | --- | ---: | ---: | ---: |
| `build.rs:94` | `run_build` | 69 | 5 | 2 |
| `build.rs:407` | `plan_gap_files` | 74 | 10 | 4 |
| `build.rs:573` | `write_rows` | 82 | 7 | 4 |
| `build.rs:705` | `write_meta_counts` | 78 | 1 | 2 |
| `build.rs:875` | `extract_units` | 62 | 7 | 5 |
| `db.rs:100` | `ensure_aw_gitignore` | 61 | 9 | 6 |
| `db.rs:189` | `acquire` | 74 | 11 | 5 |
| `db.rs:358` | `probe_old` | 61 | 12 | 3 |
| `db.rs:659` | `load_carried` | 70 | 7 | 3 |
| `db.rs:1308` | `memory_write` | 75 | 8 | 5 |
| `extract.rs:533` | `extract_file_with_code_lines` | 63 | 2 | 3 |
| `extract/resolve.rs:65` | `register_symbol` | 52 | 6 | 3 |
| `extract/resolve.rs:127` | `emit_inherit_edge` | 59 | 5 | 3 |
| `extract/resolve.rs:230` | `merge_fresh` | 69 | 9 | 4 |
| `extract/resolve.rs:423` | `symbol` | 57 | 8 | 5 |
| `extract/resolve.rs:483` | `module` | 56 | 6 | 5 |
| `extract/resolve.rs:678` | `resolve_call_sites` | 80 | 9 | 9 |
| `extract/resolve.rs:804` | `resolve_name_callee` | 68 | 17 | 4 |
| `extract/resolve.rs:889` | `resolve_attr_callee` | 57 | 16 | 5 |
| `extract/resolve.rs:1019` | `resolve_rust_keyword_path` | 55 | 14 | 5 |
| `extract/resolve.rs:1082` | `rust_unknown_receiver_outcome` | 55 | 5 | 4 |
| `extract/resolve.rs:1424` | `resolve_class_of_chain` | 70 | 19 | 7 |
| `extract/walk/mod.rs:138` | `dotted_chain` | 56 | 5 | 4 |
| `extract/walk/mod.rs:313` | `visit_node` | 97 | 14 | 4 |
| `extract/walk/mod.rs:907` | `collect_param_types` | 58 | 10 | 5 |
| `extract/walk/mod.rs:1077` | `record_assignment` | 70 | 10 | 7 |
| `extract/walk/mod.rs:1310` | `define_function` | 73 | 11 | 3 |
| `extract/walk/rust.rs:326` | `walk_use_tree` | 63 | 6 | 6 |
| `extract/walk/rust.rs:392` | `emit_use_leaf` | 77 | 10 | 6 |
| `extract/walk/rust.rs:472` | `resolve_use_path` | 55 | 8 | 4 |
| `extract/walk/typescript.rs:6` | `define_class_ts` | 57 | 10 | 4 |
| `extract/walk/typescript.rs:65` | `collect_heritage` | 72 | 12 | 8 |
| `extract/walk/typescript.rs:321` | `record_import_ts` | 83 | 10 | 8 |
| `journal.rs:327` | `render_summary` | 57 | 4 | 4 |
| `main.rs:25` | `main` | 54 | 9 | 5 |
| `mcp.rs:104` | `read_frame` | 51 | 9 | 5 |
| `mcp.rs:160` | `handle_message` | 75 | 12 | 6 |
| `mcp.rs:240` | `handle_tools_call` | 61 | 10 | 4 |
| `query/analysis.rs:29` | `dead` | 53 | 2 | 3 |
| `query/analysis.rs:281` | `files` | 53 | 6 | 4 |
| `query/analysis.rs:337` | `unresolved_names` | 69 | 3 | 2 |
| `query/analysis.rs:524` | `symbols` | 68 | 6 | 3 |
| `query/analysis.rs:628` | `symbols_directory` | 79 | 7 | 3 |
| `query/callgraph.rs:48` | `callers` | 57 | 3 | 4 |
| `query/callgraph.rs:110` | `transitive_callers` | 70 | 9 | 6 |
| `query/callgraph.rs:325` | `implementers` | 66 | 2 | 3 |
| `query/callgraph.rs:395` | `callee_empty_reason` | 51 | 6 | 3 |
| `query/callgraph.rs:505` | `callees_transitive` | 62 | 1 | 3 |
| `query/callgraph.rs:647` | `render_path` | 60 | 4 | 3 |
| `query/callgraph.rs:913` | `render_context` | 85 | 8 | 4 |
| `query/impact.rs:107` | `impact` | 55 | 11 | 3 |
| `query/impact.rs:189` | `impact_direct_grouped` | 56 | 2 | 4 |
| `query/impact.rs:249` | `impact_direct_sites` | 57 | 2 | 4 |
| `query/impact.rs:309` | `impact_deep` | 72 | 12 | 3 |
| `service.rs:218` | `from_cli_graph` | 58 | 1 | 4 |
| `service.rs:459` | `journal_args_graph` | 64 | 1 | 3 |
| `service.rs:524` | `journal_args_analysis` | 53 | 1 | 3 |
| `service.rs:759` | `run_graph_query` | 78 | 1 | 4 |
| `service.rs:840` | `run_analysis_query` | 71 | 3 | 5 |
| `service.rs:941` | `run_mem_write` | 82 | 5 | 4 |
| `service.rs:1026` | `run_mem_query` | 75 | 11 | 4 |
| `service.rs:1251` | `mem_ranked` | 57 | 10 | 4 |
| `walk.rs:110` | `walk_git_visible` | 58 | 8 | 3 |

## Appendix B: Branch proxy (>10)

| Location | Function | Effective | Branch | Nest |
| --- | --- | ---: | ---: | ---: |
| `db.rs:189` | `acquire` | 74 | 11 | 5 |
| `db.rs:358` | `probe_old` | 61 | 12 | 3 |
| `extract.rs:461` | `module_path_for` | 42 | 15 | 4 |
| `extract/resolve.rs:804` | `resolve_name_callee` | 68 | 17 | 4 |
| `extract/resolve.rs:889` | `resolve_attr_callee` | 57 | 16 | 5 |
| `extract/resolve.rs:971` | `resolve_receiver_method` | 42 | 14 | 5 |
| `extract/resolve.rs:1019` | `resolve_rust_keyword_path` | 55 | 14 | 5 |
| `extract/resolve.rs:1216` | `is_local_binding` | 31 | 11 | 4 |
| `extract/resolve.rs:1424` | `resolve_class_of_chain` | 70 | 19 | 7 |
| `extract/resolve.rs:1593` | `chain_from_module` | 42 | 13 | 5 |
| `extract/walk/mod.rs:52` | `signature_of` | 46 | 12 | 5 |
| `extract/walk/mod.rs:313` | `visit_node` | 97 | 14 | 4 |
| `extract/walk/mod.rs:1207` | `record_call` | 47 | 13 | 8 |
| `extract/walk/mod.rs:1310` | `define_function` | 73 | 11 | 3 |
| `extract/walk/rust.rs:185` | `scan_macro_call_names` | 46 | 17 | 6 |
| `extract/walk/typescript.rs:65` | `collect_heritage` | 72 | 12 | 8 |
| `journal.rs:142` | `append_line` | 39 | 12 | 6 |
| `mcp.rs:160` | `handle_message` | 75 | 12 | 6 |
| `memory.rs:85` | `split_run` | 31 | 14 | 4 |
| `parse.rs:185` | `code_lines_from_ranges_work` | 37 | 12 | 5 |
| `query/callgraph.rs:731` | `path_bfs` | 46 | 13 | 5 |
| `query/impact.rs:107` | `impact` | 55 | 11 | 3 |
| `query/impact.rs:309` | `impact_deep` | 72 | 12 | 3 |
| `service.rs:1026` | `run_mem_query` | 75 | 11 | 4 |

## Appendix C: Data Clumps

- `(node, src, fx, ctx, work)` on 15 `visit_*`/definition helpers in
  `src/extract/walk/mod.rs` and `src/extract/walk/typescript.rs`.
- `(conn, root, name, limit, depth, explain)` on `callers` and `callees`
  (`src/query/callgraph.rs`). This is the S2 dispatch-debt surface.

## Appendix D: Long Parameter List

The three rows below are the non-cohesive parameter bags considered for a
parameter object and deliberately left as-is. They are not the full
catalog-threshold set: the current measurement found 19 functions at 6+
parameters (the Fowler catalog threshold is 5+). The remaining rows are
Walker/phase context bags, covered by Appendix C.

- `normalize_note` — 7 parameters (`src/memory.rs`).
- `ImpactTop::insert` — 7 parameters (`src/query/impact.rs`).
- `collect_output` — 6 parameters (`src/query.rs`).

Measured at 6+ parameters (2026-11-06):

| Function | Params | File |
| --- | ---: | --- |
| `normalize_note` | 7 | `src/memory.rs` |
| `symbol_line` | 7 | `src/query/analysis.rs` |
| `ImpactTop::insert` | 7 | `src/query/impact.rs` |
| `parse_gate` | 6 | `src/build.rs` |
| `write_rows` | 6 | `src/build.rs` |
| `finish` | 6 | `src/build.rs` |
| `emit_inherit_edge` | 6 | `src/extract/resolve.rs` |
| `follow_binding` | 6 | `src/extract/resolve.rs` |
| `push_scope_symbol` | 6 | `src/extract/walk/mod.rs` |
| `define_callable` | 6 | `src/extract/walk/mod.rs` |
| `define_function` | 6 | `src/extract/walk/mod.rs` |
| `push_from_binding` | 6 | `src/extract/walk/python.rs` |
| `symbols` | 6 | `src/query/analysis.rs` |
| `symbols_directory` | 6 | `src/query/analysis.rs` |
| `callers` | 6 | `src/query/callgraph.rs` |
| `callees` | 6 | `src/query/callgraph.rs` |
| `callees_transitive` | 6 | `src/query/callgraph.rs` |
| `level_impact` | 6 | `src/query/impact.rs` |
| `collect_output` | 6 | `src/query.rs` |

## Verification

- `cargo fmt --all -- --check` — pass.
- `cargo clippy --all-targets --locked -- -D warnings` — pass.
- `cargo test --locked` — 327 passed, 0 failed.
- `bash tests/scripts/allowlist_check.sh Cargo.lock` — pass.
- `bash tests/scripts/write_path_check.sh` — pass.
