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
