# Goldfish Review — Prevent-Reinvention v2 Spec

- Target: `docs/designs/2026-11-12_reuse-prevention.md`
- Ranges: `FeatureRange` L1-1334; `DetailedRange` L1335-end
- Date: 2026-11-12
- Lenses run: 5 (Zoom-Out, Comprehension, Design Critic, Implementation Readiness, Codebase Details)
- Verdict: **FAIL — not approvable as written**

## Critical findings

1. **The spec is a delta overlay, not a self-contained spec.** Revision 5 is a 60-line English overlay on a 1,300-line Vietnamese predecessor stored in `.tmp/`, which is gitignored. Contradictions remain in the normative body: composition reject-vs-Low, `distinctive` df-cap-vs-no-cap, oracle 160-vs-140, hash 16-vs-64 MiB, output cap 28/30/32 KiB. A fresh implementer cannot tell which text is binding.
2. **Phase 1 confidence is undefined.** `provisional` is not in the `--min-confidence` enum, sort order, summary buckets, or output grammar. Phase 1 tables still say `Medium`; D3/D4 still have High cells; `High`/`unvalidated-high` render rules contradict “High absent”.
3. **Oracle arithmetic and power do not close.** 160 vs 140 labels, holdout separate vs subset, 60 vs 80 positives, budget 210 called 180-200, and a 0.15 margin at n=30 against MDD ≈0.24. The gate is underpowered by the spec’s own note.
4. **Path/exit/truncation is contradictory.** A typo path exits 1 in Revision 5/CUJ 1, exit 0 `matched 0` in §2.5; the status table is malformed; `path_matches` and `showing N of M` are absent from §9.4.
5. **Composition has two dispositions.** Gate 1 Decision 1 says resolved call edges reject; §4.1 item 4 says both rules reject only at `d=0` and demote at `d>=1`.
6. **`distinctive` has two definitions.** Revision 5 requires `df_all_excl <= 2`; §3.4 says “Distinctive KHÔNG có df-cap”.
7. **Judge phase and trust boundary conflict.** The normative body assigns hang fix/span/whitelist/containment to Phase 1; the detailed plan assigns them to Phase 5. The current judge resolves PATH-first, inherits the environment, passes source via argv, lacks containment, and `child.kill()` does not kill the process group/reader thread.
8. **Core API and data shapes are missing.** `ReuseOpts`, `analyze_corpus_with`, the new `run_reuse`, `LineQuery { mode, truncated }`, `ReuseCandidate`, `Confidence`, `ReasonCode`, `ReuseCounts`, `Pair`, and the `set_*` signatures are named but never declared.
9. **The implementation plan is not executable.** The file ledger is shorthand and omits required files (`tests/cli/main.rs`, `src/service.rs`, `src/query.rs`, `src/journal/render.rs`, `src/dashboard/data/summary.rs`, `src/query/judge.rs`, `src/db.rs`, docs/skill/CHANGELOG). Seven new `tests/cli/*.rs` are not declared in `tests/cli/main.rs`, so `cargo test --test cli reuse_scan` runs zero tests and exits 0.
10. **Module cap is violated.** `src/journal.rs` is 490 LOC and the plan adds 120-220; `src/journal/render.rs` is 470; `src/dashboard/data/summary.rs` is 450; `src/query/reuse/detectors.rs` is 429 with a positive delta. `tests/scripts/hygiene_check.sh` hard-fails any `.rs` over 500.
11. **Memo-gated phases have no gate mechanism.** `proposal.rs`, `allowlist.rs`, `coverage.rs` are created default-off; with `clippy -D warnings`, unused modules/fields fail. No Cargo feature, `cfg`, runtime flag, or cut procedure is defined.
12. **Pipeline order contradicts itself.** `guards -> finalize -> judge` vs `judge -> finalize`; `allowlist -> sort -> truncate` vs `sort -> allowlist -> truncate`. Judge unlocks High, so finalize cannot precede it.
13. **The full design is over-scoped and the alternatives are delivery slices.** Build-hint-first is treated as a baseline, not a first-class alternative; Phase 2-5 are fully specified but memo-gated/default cut. The only justified phase is 1a + R29.

## Important findings

- Caller counts have two definitions: `CALLER_COUNT_SQL` in search vs the new distinct-caller rule in reuse.
- `open_for_query` does not call `reject_tracked_eg_dir`; judge source lacks `safe_index_path`.
- `file_changed` returns `false` for a missing `files` row, so a missing index row looks fresh.
- MCP success path drops `LineQuery.stderr` except for `build`, so `eg reuse --judge` is not byte-identical.
- `tests/cli/main.rs` omits the new test modules; targeted test filters can pass vacuously.
- `src/journal.rs` and dashboard files exceed the 500-LOC cap; decomposition is not in the ledger.
- `df_all_excl` is described as a SQL aggregate and as a bulk corpus map; Phase 1 production-only load makes them equal.
- Hash budget 16 vs 64 MiB; output cap 28 vs 30 vs 32 KiB; `partial` is a fourth freshness state.
- Path status table is malformed; bare-name zero-match has no row; absolute/`..` security behavior is undefined.
- `--propose`/`--callees`/`--coverage`/`--min-confidence` are not in the shipped CLI table; CUJ 4 describes a different feature.
- R29 CUJ says `mem query` clears the stale tag; §7.1 says it intentionally does not.
- Allowlist key includes `@<line>`, so a line shift expires the note; MCP `mem_write` can create allowlist notes without the CLI confirm gate.
- `--limit` help says max 100, query clamps to 10,000, MCP max 100.
- Journal candidate IDs cap at 25, so uptake telemetry loses candidates 26+.
- `C-P0-05` says perf gates use `reuse_perf.py`; the detailed plan uses `cargo test --test reuse_bench`.
- The oracle script command is `--snapshot ... --labels ... --sap ...` with ellipses and no formats/exit codes.
- “Calibrated” remains on seven surfaces; the codebook defines duplicate as behavioral equivalence while Revision 5 says structural substitutable.
- Phase 2-5 exits are weaker than the phase gates; cut order does not model phase dependencies.

## Reconciliation

| Finding | Decision | Evidence |
|---|---|---|
| Delta overlay/contradictions | PROPOSE FIX | Rewrite as one self-contained normative spec; remove the predecessor from the normative body. |
| Confidence contract | PROPOSE FIX | One enum `low < provisional < medium < high`; Phase 1 accepts only `low/provisional`; reject `medium/high`; one grammar. |
| Oracle arithmetic/power | PROPOSE FIX | One frozen table: tune 50, confirmation 60 pos (30 eligible + 15 D4 + 15 D3) + 30 neg, holdout 20 separate; SAP before margin. |
| Path/exit/truncation | PROPOSE FIX | One status table and one grammar; typo path exit 1; valid empty scope exit 0; `showing N of M`. |
| Composition/distinctive | PROPOSE FIX | One disposition and one `distinctive` predicate with `df_all_excl <= 2`. |
| Judge trust boundary | ESCALATE | Cut judge from Phase 1 or close the boundary in Phase 1; cannot close `killpg` without `libc` under zero-unsafe. Owner decision. |
| Missing APIs/data shapes | PROPOSE FIX | Declare exact Rust types and visibility in the spec. |
| File ledger/test false green | PROPOSE FIX | Exact file ledger; add `tests/cli/main.rs`; test matrix with names/commands. |
| Module cap | PROPOSE FIX | Decompose journal/dashboard/detectors into new files in the ledger. |
| Memo-gating | PROPOSE FIX | Define a Cargo feature or runtime flag; state cut procedure. |
| Pipeline order | PROPOSE FIX | One numbered pipeline: analyze → judge → finalize → filter → sort → allowlist → truncate → render → journal. |
| Full-scope YAGNI | ESCALATE | Owner choice: Phase 1a/1b spec + roadmap, or full-design second revision. |

## Gate 2 result

Gate 2 fails. The spec cannot move to `Approved` in its current form. The next step is an owner scope decision, then a rewrite and one re-audit (the review bound allows one re-audit per stage).

## Final re-audit (Phase 1 rewrite)

- Target: `docs/designs/2026-11-12_reuse-prevention.md` (Phase 1 rewrite)
- Lenses run: 2 (Comprehension, Implementation Readiness)
- Verdict: **FAIL — 10 Critical findings remain**
- Review bound: this is the third review cycle; no further full re-audit is allowed without owner acceptance.

Critical findings:

1. Cap/count model is unresolved drafting: `omitted` is not a field; `capped` ownership and `truncated` composition are undefined.
2. `FreshnessChecker::check` returns `Freshness` but cannot return the `unknown_reason` needed by the header; multiple unknown reasons have no precedence.
3. Freshness scope/aggregation/empty-result warning is undefined: “result files” cannot produce a warning when the result is empty; repo-wide new-file walk vs per-pair downgrade is unresolved.
4. `callers` / `inbound_callers` / test-scope / dedupe / self-edge semantics are undefined; `CALLER_COUNT_SQL` and reuse counts cannot be made to agree.
5. `C_res` hop depth, `C_in` parameter, minimum footprint, and raw-client membership are undefined; `substantial` raw-client branch is unreachable in `shared`.
6. Guard precedence is undefined: a D1 pair with no name token can be both provisional and low.
7. Stream assignment conflicts: ambiguous stdout/stderr, freshness warning, cap footer, and MCP success/error join are not byte-derivable.
8. Raw-client/wrapper/canonical definitions and D3 exclusions are missing; `{raw}` has no field.
9. Unsafe-path disposition conflicts: user input exit 2, index row `Stale`, tracked `.engram` exit 6 in the spec but `EgError::Security` exit 2 in code.
10. Self-match is not excluded: D1 can match a symbol to itself (containment 1.0, bal 1.0).

Important findings include: `LineQuery` has nine literal constructors (spec lists four); `src/service/explain.rs` is omitted from the ledger (if present in the working tree); `analyze_corpus` wrapper/target_names/freshness order undefined; `GUARD-KIND` is unreachable because `load_symbols` already filters; `detector_censored` has no observable channel; `df_all_excl` issues one SQL query per shared callee; `MEM_ROW_SQL` probe error path/column order undefined; MCP exit 6 mapping and default limit undefined; CLI `--limit` 100 change is breaking and undocumented; journal `shown` fallback needs `Option`; old `calibrated`/`<3ms` claims remain in docs; judge remnants (`verdict`, `set_judged`, dashboard badge) remain.

### Final decision

Decision: **Approved**. The owner explicitly approved the Phase 1 spec on 2026-11-12 after all 10 Critical findings were closed and a targeted closure check returned PASS on each. The spec status is `Approved`. Important findings remain implementation tasks; no implementation has started.

### Closure update (2026-11-12)

The 10 Critical findings were **closed in the Phase 1 spec** at `docs/designs/2026-11-12_reuse-prevention.md` (count/cap partition, freshness `CheckResult`, freshness scope/aggregation, caller semantics, `C_res`/`C_in`, guard precedence, streams/MCP, raw-client/canonical/self-match, unsafe-path disposition, self-match). A targeted closure check — not a full re-audit — returned PASS on all 10 with line evidence. The owner then explicitly approved the spec. Important findings remain implementation tasks. No implementation has started.
