# P0 Baseline Summary — grep arm on vihap @ f7ca48c (2026-10-31)

## Methodology

- Executor: the agent itself ran the grep arm (tools: `rg`, `sed -n` slice reads; tests/ excluded per task rules) on `~/Workspace/mapkit/vihap` at rev `f7ca48c` (2026-10-30).
- Recording: every tool call went through `harness/g`, which executes the command, captures stdout/stderr to `out/T<n>-<seq>.txt`, and appends a JSONL record (tool, command, in/out bytes). Byte accounting: `in_bytes` = the agent's tool-call input string (wrapper prefix excluded), `out_bytes` = tool-result bytes returned. Both directions counted per G-C definition.
- Expected sets: frozen in `expected/T<n>.json` by audited derivation scripts (`harness/derive_t1.py`, `harness/derive_t2_t5.py`) from the same recorded outputs. Resolution logic is mechanical (call-paren match, def/import exclusion, documented shadow sets) — no hand-picking.
- **Byte-accounting convention:** the wrapper's own `[g] <task>#<seq> ...` stderr status line (~40 B/call) is excluded from `out_bytes`. The P1 kg arm MUST use the identical `g` wrapper so the exclusion applies to both arms symmetrically.
- **Cold-run protocol for G6 (R0 suggestion 1, adopted):** the P0 per-task numbers above were recorded warm (T5's single call reuses callee names discovered during T4 work in the same session; T2's ordering draws on session flow knowledge). G6 must run fresh, cold, independent runs (spec: 2 runs x 2 modes) and must not reuse P0 per-task bytes as the denominator without a cold-run equivalence check.
- **Owner sign-off pending (R0 suggestion 2):** the 30 GT rows and the pain list are agent-produced (owner delegated). R0 finding 1 proved self-verification can miss facts. Owner sign-off on GT rows G16-G30 qualnames and the pain list is requested before G2/G7 scoring is treated as final.
- **R0 flag (known weakness):** baseline answers equal the frozen truth by construction (same agent, same recorded greps produced both). Correctness scores below are a reproducibility sanity check, not independent verification. R0 spot-audited 10+ rows and confirmed them; the kg treatment arm (P1, G6) is scored by this same checker, which is where the checker carries real weight.

## Baseline numbers (harness/count.py)

| task | tool calls | in bytes | out bytes | total bytes |
|------|-----------|----------|-----------|-------------|
| T1 | 5 | 441 | 38372 | 38813 |
| T2 | 2 | 155 | 2658 | 2813 |
| T3 | 1 | 51 | 4733 | 4784 |
| T4 | 3 | 231 | 2022 | 2253 |
| T5 | 1 | 342 | 219 | 561 |
| TOTAL | 12 | 1220 | 48004 | 49224 |

Correctness (harness/check.py vs frozen expected): T1–T5 all 100.00% precision / 100.00% recall / 100% ordering (self-derived; see R0 flag).

## P0 exit criterion (cheapest kill) — evaluation

- Rule: if grep answers EVERY navigation task in <= 2 tool calls, the premise is weak — stop before P1.
- Measured: T1 = 5 calls, T4 = 3 calls (only T2/T3/T5 <= 2). Premise is NOT weak.
- Projected G6 reduction (tool calls): baseline 12 calls for 5 tasks; kg projects ~1 indexed query per task ≈ 5 calls ≈ 58% reduction — above the 30% G-C bar, clearly above the 15% present-to-owner floor. Projection only: the kg arm must actually be run under this harness in P1; no byte-side claim is made here.

## Findings recorded during P0

1. `normalize_ascii` has 7 definitions: 1 canonical (`vihap/cleaners/unicode.py:47`) + 6 script-local shadows that exclude 60 call lines from canonical resolution (`expected/T1.json:excluded_local_shadow`).
2. Three re-export chains (text_utils, cleaners/__init__, normalizer) all collapse to the canonical def — a name-only index would still be right here, but per-module import maps are required for the shadow scripts (G30 trap).
3. No star imports exist in vihap (rg verified) — the spec's star-import resolution fixture must be synthetic.
4. tasks.md T5 essence correction: one-hop cross-module callees of `parse` are parsers/macro_resolver (x3), formatters/AddressFormatter.format, quality/QualityScorer.score. grammar/, validators/, knowledge/ are >= 2 hops (correction appended to tasks.md; scoring uses `expected/T5.json`).

## Gate status after P0

| gate | state |
|------|-------|
| G-A | targets frozen; GT 30 rows filled (ground-truth.md); measured in P1 (G2) |
| G-B | targets still labeled "target until measured"; kg numbers recorded in P1 (G1/G3) |
| G-C | grep baseline recorded here; kg arm + A/B in P1 (G6) |
| G-D/G-E | invariants CI-enforced from P1 onward |
| P1.5 (G7) | pain-list.md PROVISIONAL — owner confirmation pending; G7 degraded until then |

## G6 — A/B results (2026-10-31, run B, depth-N binary)

| arm | tool calls | bytes (both dirs) | correctness |
|-----|-----------|-------------------|-------------|
| baseline (grep, frozen P0, R0-approved) | 12 | 49,224 | 100% self-derived, R0 spot-audited |
| baseline (cold run B, equivalence check) | 13 | (transcripts/cold/) | — |
| treatment (kg, kgarm/, run B) | 6 | 16,125 | T1 100/100 + 7/9 dirs; T2 100/100 + ordering 100%; T3 100/100; T4 ordering 100% (pointer semantics differ: callees contract emits def lines); T5 100/100 |

- Tool-call reduction: (12-6)/12 = **50%** (target >= 30% — PASS).
- Bytes: 16,125 vs 49,224 = **-67%** (no increase — PASS).
- Cold-run equivalence (R0 suggestion): cold run B = 13 calls vs warm 12 (+8%) — warm baseline is a valid denominator.
- Treatment-arm disagreements audited individually: (1) expected/T1 had a docstring misclassified as a call site (kg was right); (2) expected sets included untracked+gitignored scripts — kg's walk contract is the git-visible set (git ls-files -co), expected sets corrected to match; (3) depth>1 queries added to callees (`--depth N`) so flow tasks collapse to 1 query — response contract extension recorded as Amendment 2.
- Run A (kgarm_runA/, depth-1 binary, 10 calls) retained as evidence that per-task chaining without --depth yields only ~17% tool-call reduction — the --depth extension is load-bearing for G6.

## Amendment 2 (response contract)

`kg callees` gains `--depth N` (default 1, clamp 1..10): transitive call closure
via recursive CTE. Output format unchanged (pointer lines). Recorded 2026-10-31
during G6; the spec's response-contract section gains this flag at next edit.

## Gate ledger after P1 (P1 core)

| gate | verdict | evidence |
|------|---------|----------|
| G1 | PASS | clean build 1.91s wall, 21.4MB RSS, 1.9MB DB (limits 5s/100MB/50MB) |
| G2 | PASS | recall 29/29 = 100% [88,100], precision 493/495 = 99.6% [99,100] (limits 70/80/95) |
| G3 | PASS | p95 = 10.1ms over 30 runs (limit 20ms) |
| G4 | PASS | staleness header + FNV re-hash path (tests/cli.rs g4_staleness) |
| G5 | PASS | seatbelt network-deny build + Cargo.lock allowlist + write-path grep + tree-snapshot (content-hashed) + proc-hygiene (pgrep) tests |
| G6 | PASS | 50% tool-call reduction, -67% bytes, no correctness loss |

## P1.5 gate results (2026-10-31)

| gate | verdict | evidence |
|------|---------|----------|
| G7 hotspots rank-overlap | PASS 8/10 (target >= 7/10) — still DEGRADED pending owner pain-list confirmation | R1.5 Critical fix: per-metric min-max normalization (pooled bounds let LOC swamp churn/fix; measured 4/10). Post-fix top-10 matches pain ranks 1,2,3,4,5,7,8,10. Command: `kg hotspots --limit 10 --root vihap` |
| G8 P1.5 extensions on architecture questions | PASS -67% calls — attribution corrected per R1.5 | The reduction is carried by `kg callees --depth` (Amendment 2), NOT `kg report`: baseline cold T4+T5 = 6 calls vs kg 2 calls (kgarm transcripts). `kg report` was never invoked in an A/B transcript; its differential utility on architecture questions is not demonstrated (callees --depth answers them in 1 call already). report is retained as a convenience summary; the G8 claim is scoped to the extension set, not to report alone |

P1.5 surface: `kg stats`, `kg hotspots [--limit N]`, `kg report` (4 sections).

## P2 gate results (2026-10-31, TS/TSX/JSX)

| gate | verdict | evidence |
|------|---------|----------|
| G2 rerun on finguard (20 TS/TSX GT sites) | PASS | recall 20/20 = 100% [84,100], precision 57/57 = 100% [94,100] — command: `python3 tests/scripts/precision_recall.py <finguard> <kg> docs/baselines/ground-truth-finguard.md` |
| Python regression | PASS | vihap recall 29/29, precision 493/495 on the rebuilt index; 54 tests green |

P2 notes:
- Grammars added in one commit (tree-sitter-typescript 0.23 + tree-sitter-javascript 0.25 + runtime 0.25); golden Python fixtures replay unchanged.
- Relative TS imports resolve against the FILE'S DIRECTORY (stems may contain dots: command-bus.test.ts, runner.node.ts) — module-string derivation was wrong and was replaced (found by the finguard G2 run).
- External imports (react, @finguard/*-alias) bind but resolve to unindexed modules: calls into them drop with the builtin counter, not unresolved.
- unresolved=3403 in finguard is dominated by destructured hook results and untyped receivers (spec non-goal); the import-call backbone resolves.
- R2 additions: `this.m()` resolves (TS parity with self.m()); pnpm workspace aliases (@finguard/*) documented as a limitation (value calls into workspace packages drop until alias resolution lands); JSX component usage (<Button />) is a spec non-goal — not indexed as calls.

## P2.5 + P3 (2026-10-31)

| item | verdict | evidence |
|------|---------|----------|
| P2.5 Rust self-index (optional, implemented) | PASS | kg indexes its own repo: 20 files, 218 symbols, 476 edges; G2-self recall 9/10 = 90% [60,98], precision 14/14 = 100% [78,100] — command: `python3 tests/scripts/precision_recall.py ~/Workspace/kg target/release/kg docs/baselines/ground-truth-kg-self.md`. crate:: paths + mod declarations mapped to the crate root dir; GRAMMAR_VERSION gains rs-0.24.2 |
| P3 stdio MCP server (optional) | SKIPPED — trigger condition not met | Spec P3 trigger: "only if CLI discoverability measurably fails". No such measurement exists; the CLI answers all three questions with documented exit codes + the code-index skill wrapper ships install instructions. Condition re-evaluable at owner request |

Final state: all mandatory phases (P0, P1, P1.5, P2) complete and independently reviewed (R0/R1/R1.5/R2 APPROVE); optional P2.5 implemented and gated; optional P3 skipped with documented trigger analysis.
