# R13 — P8 Keep / Improve / Remove audit

- Baseline: kg `0b51585` (schema v7). Current HEAD `434ce57` differs only in
  build/perf and schema-index ordering; contract/query files are unchanged.
- Six independent Goldfish lenses: L1 correctness, L2 utility, L3 latency,
  L4 contract, L5 removal cost, L6 evidence integrity. L1-L5 voted; L6 did not.
- L6 verdict: **INSUFFICIENT for REMOVE**.

## TL;DR

- **No feature is removed.** L6 blocks REMOVE and no feature reached the 4/5
  REMOVE threshold.
- **KEEP:** F2 (no removal; owner may demote flag), F3, F5, F7, F9.
- **IMPROVE:** F1, F4, F6, F8, F10.
- **OWNER:** F11 — DEMOTE filters recommended, not executed.
- F4 is the only robust utility winner (direct import impact, 29-45% fewer tool
  calls on 3 valid hosts) but has a **Critical 1M-edge hub latency** defect, so
  it is IMPROVE rather than KEEP.
- F6 has the only release-blocking contract defect: `--no-journal` is bypassed
  by `resolve`.
- F9 and F5 stay, but their utility claims are unproven; do not market them as
  measured wins.

## Decision matrix

| Feature | L1 | L2 | L3 | L4 | L5 | Final | Action |
|---|---|---|---|---|---|---|---|
| F1 implementers/dead | IMPROVE | IMPROVE | KEEP | IMPROVE | KEEP | **IMPROVE** | Cross-file GT, TS aliases, dead task |
| F2 impact --context | KEEP | DEFER | KEEP | KEEP | REMOVE | **KEEP** | No removal; owner decides flag demotion |
| F3 path/files/outline | KEEP | DEFER | KEEP | KEEP | KEEP | **KEEP** | Padded path/help; no utility claim |
| F4 impact edges/depth | KEEP | KEEP | IMPROVE | KEEP | KEEP | **IMPROVE** | Fix 1M-edge hub latency |
| F5 unresolved names | KEEP | DEFER | KEEP | KEEP | KEEP | **KEEP** | Add utility task; no removal |
| F6 resolve/search filters | KEEP | DEFER | KEEP | IMPROVE | DEMOTE | **IMPROVE** | Fix no-journal/MCP/journal blocker |
| F7 imports | KEEP | KEEP | KEEP | KEEP | KEEP | **KEEP** | TS GT; no removal |
| F8 tests | KEEP | IMPROVE | IMPROVE | KEEP | KEEP | **IMPROVE** | Latency fix + broader utility evidence |
| F9 complexity/metrics | KEEP | DEFER | KEEP | KEEP | IMPROVE | **KEEP** | Add GT/utility; no claim |
| F10 api | IMPROVE | IMPROVE | KEEP | IMPROVE | KEEP | **IMPROVE** | Crate-root/export GT/claims |
| F11 generated | KEEP | DEFER | KEEP | IMPROVE | DEMOTE | **OWNER** | Demote filters recommended |

## L6 integrity findings

L6 judged the battery insufficient for REMOVE because:

- No REMOVE rule was pre-registered; each feature has one narrow task shape,
  n=2, no confidence intervals.
- Protocol freeze timing is not evidenced; agy exclusion is absent from the
  protocol dated corrections.
- OpenCode exclusion is overbroad and directionally favors treatment: controls
  were 9/9 correct; treatment had 4 wrong + 1 missing.
- Utility table had transcription errors: agy I1/H1 control means should be
  18.0/18.0, not 19.0/21.0.
- `score_p8.py` uses sets, so it does not enforce the protocol's duplicate-row
  failure.
- Corpus/build/write/network integrity and feature-FIRST sequence are unaudited.
- R8-R12 closures are consolidated implementer summaries, not raw independent
  evidence.

These are remediation items for the next battery, not reasons to remove a
feature.

## Critical findings

1. **F4/F8 1M-edge hub latency.** L3 measured `impact --depth 3` 1,132.5 ms
   vs 150 ms and `tests` 1,335.6 ms vs 200 ms on a 1.37M-edge hub subset.
   `--limit 5` does not reduce impact cost. Pin the corpus/budget and fix the
   inbound ORDER BY/BFS.
2. **F6 `--no-journal` bypass.** `resolve` writes `.kg/usage.log` despite
   opt-out. This is a contract violation and release blocker.

## Important findings

- F1 GT is same-file only; TS package aliases miss cross-file extension;
  `dead` has no utility/GT task.
- F4 `--context --depth>1` silently groups; ambiguous tag unreachable.
- F5 default `unresolved` does not filter/count `site_line=0` like `--names`.
- F6 MCP blank names diverge from CLI; resolve journal metadata inconsistent.
- F8 reported precision is duplicate-inflated and name-shape only; no
  negative controls.
- F10 cannot answer crate-root API; no real export GT; signature/staleness
  docs disagree.
- F11 filters lack exclusion tests and negative controls; zero observed use.
- Build overhead is exactly at 1.25x paired (1.251-1.259x); frozen absolute
  budget passes but the gate definition must be chosen.

## Packets

- `improve/F1.md`, `F4.md`, `F6.md`, `F8.md`, `F10.md`
- `removal/F2.md` (blocked by L6; not approved)
- `removal/F11-demote.md` (owner decision)

## Next steps

1. Fix F6 blocker and add no-journal/MCP parity regressions.
2. Fix F4/F8 1M-edge latency; pin the corpus and p95 budget.
3. Correct utility report numbers and L6 evidence defects before any REMOVE
   decision.
4. Run the improvement packets under separate fix cycles (max 2 per canonical
   bound).
5. Re-run a REMOVE-only audit only after the next battery meets L6 requirements.
6. Do not update README/skills/EVALUATION until owner signs the matrix.
