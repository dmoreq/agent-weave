# Review disposition — 2026-11-03

Source register: [2026-11-03_agent-weave-review-findings.md](2026-11-03_agent-weave-review-findings.md).
Snapshot reviewed: `1439a9e`; fixes are uncommitted working-tree changes.

## Status

| Finding | Disposition | Evidence |
|---|---|---|
| S1 symlink confinement | FIXED | Symlink tests in `tests/cli.rs`; MCP build test in `tests/mcp.rs` |
| S2 `unresolved --path` parameters | FIXED | `unresolved_path_scope_binds_params` |
| S3 invalid UTF-8 kills MCP | FIXED | `mcp_framing_survives_invalid_utf8_and_oversized_frames` |
| S4 oversized frame eats next request | FIXED | Same MCP framing test (both boundary cases) |
| S5 lock identity during invalidate | FIXED | Root-directory lock; `invalidate_refuses_while_root_lock_is_held` |
| A2 removed inheritance carried | FIXED | Full normalized edge-set comparison `incremental_and_fresh_edge_sets_match_after_removed_inheritance` plus positive control |
| A3 added candidate keeps resolved edge | FIXED | `added_duplicate_candidate_invalidates_resolved_edges`; unrelated-addition negative control |
| A4 comment scan quadratic | FIXED | Cursor scan plus `comment_scan_work_is_near_linear` work-counter measurement |
| C1 function-local import bindings | FIXED | `local_imports_do_not_cross_bindings` |
| C2 parameter shadow | FIXED | `parameter_binding_shadows_module_function` |
| C3 unknown reassignment receiver | FIXED | `unknown_reassignment_clears_receiver_type` |
| C4 Rust associated factory | FIXED | `rust_factory_does_not_type_receiver` |
| C5 arrow body double walk | FIXED | `arrow_initializer_has_one_owner` |
| C6 nested-class method qualname | FIXED | `nested_class_methods_keep_class_qualname` |
| C7 receiver vs shadowed module | FIXED | `local_receiver_blocks_shadowed_module` |
| C8 class ambiguity | FIXED | `ambiguous_class_does_not_pick_one_method` |
| D1 release repository context | FIXED | `.github/workflows/release.yml` `--repo` + `GH_REPO` |
| D2 force help hides parse bypass | FIXED | `build_force_bypasses_parse_gate_and_help_says_so` |
| D3 invalidate help scope | FIXED | Same help test; `doctor.rs` |
| D4 changelog private corpus names | FIXED | Neutralized labels; public-doc scan clean |
| D5 context description vs ambiguous rows | FIXED | `CONTEXT_CAVEAT`, CLI/MCP descriptions, byte test |
| D6 docs deny dotted suffix | FIXED | `docs/cli-reference.md`, `skills/aw-first/SKILL.md` |
| D7 parse-abort exit code drift | FIXED | `AGENTS.md` now says exit 7 |
| E1 scorer accepts duplicates | FIXED | Scorer rejects normalized duplicates and missing runids |
| T1 shared fixture directories | FIXED | Atomic sequence in `temp_repo` |
| T2 exporter deletes destination early | FIXED | Export tests: nonempty and overlapping output |
| T3 vacuous process assertion | FIXED | `pgrep` checks + seeded-child detection |
| T4 scanner fail-open | FIXED | Scanner return-code check; injected-error test |
| T5 negative fixture indentation | FIXED | Raw string in `tests/fixtures.rs` |
| H1 historical search hops | FIXED | Missing/null fail, explicit zero passes |
| R1 long orchestration functions | FIXED | Phase helpers in `impact`, `test_impact`, `clones`; exact-output tests |
| R2 duplicated direct-edge loaders | FIXED | Private `direct_callers`/`direct_callees`; callees/context parity tests |
| R3–R6, M1 refactors | DEFERRED | Behavior-preserving maintainability work; no correctness defect |
| H2, H3 | CONTESTED | Register already marks them `CONTEST`; no code change |

## Verification

- `cargo fmt --all -- --check`: pass.
- `cargo clippy --locked --all-targets -- -D warnings`: pass.
- `cargo test --locked`: unit 27, CLI 136, fixtures 34, MCP 10, doc-tests 0 — all pass.
- `bash tests/scripts/allowlist_check.sh Cargo.lock`: pass.
- `bash tests/scripts/write_path_check.sh`: pass.
- `python3 tests/scripts/test_public_export.py`: 8 pass.
- P8 scorer re-run: same 4 pre-existing invalid OpenCode failures; no verdict change.
- P9 scorer re-run: 6/6 exact match.

## Deferred work

R3, R5 and M1 are shared-helper/refactor candidates; R4 and R6 are
suggestions. They are valid maintainability findings but not correctness or
security defects. They are intentionally left out of this change to keep the
fix diff reviewable; each requires its own behavior-preserving refactor pass.
