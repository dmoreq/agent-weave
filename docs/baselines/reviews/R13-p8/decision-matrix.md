# R13 — Decision matrix

Baseline: `0b51585`. Six independent lenses; L6 did not vote feature value.

**L6 gate: INSUFFICIENT for REMOVE.** No feature can be removed on this
evidence. DEMOTE actions also require owner sign-off.

| Feature | L1 correctness | L2 utility | L3 latency | L4 contract | L5 removal | Final | Owner action |
|---|---|---|---|---|---|---|---|
| F1 implementers/dead | IMPROVE | IMPROVE | KEEP | IMPROVE | KEEP | **IMPROVE** | Cross-file GT, TS aliases, dead utility |
| F2 impact --context | KEEP | DEFER | KEEP | KEEP | REMOVE | **KEEP** (no removal) | Owner decides whether to demote flag |
| F3 path/files/outline | KEEP | DEFER | KEEP | KEEP | KEEP | **KEEP** | Fix padded path/help; no utility claim |
| F4 impact edges/depth | KEEP | KEEP | IMPROVE | KEEP | KEEP | **IMPROVE** | 1M-edge hub latency Critical |
| F5 unresolved names | KEEP | DEFER | KEEP | KEEP | KEEP | **KEEP** | Add utility task; no removal |
| F6 resolve/search filters | KEEP | DEFER | KEEP | IMPROVE | DEMOTE | **IMPROVE** | Fix --no-journal/MCP/journal blocker |
| F7 imports | KEEP | KEEP | KEEP | KEEP | KEEP | **KEEP** | TS GT; no removal |
| F8 tests | KEEP | IMPROVE | IMPROVE | KEEP | KEEP | **IMPROVE** | Latency + utility evidence before any removal |
| F9 complexity/metrics | KEEP | DEFER | KEEP | KEEP | IMPROVE | **KEEP** | Add GT/utility; no utility claim |
| F10 api | IMPROVE | IMPROVE | KEEP | IMPROVE | KEEP | **IMPROVE** | Crate-root, export GT, claims |
| F11 generated | KEEP | DEFER | KEEP | IMPROVE | DEMOTE | **OWNER** | DEMOTE filters recommended; add negatives/GT |

## Why no REMOVE

- L6 = INSUFFICIENT: no pre-registered REMOVE rule, n=2, one task shape,
  post-hoc OpenCode exclusion directionally favors treatment, and the utility
  table had transcription errors.
- Only F2 received a REMOVE vote (1/5), below the 4/5 threshold.
- F6 and F11 received DEMOTE votes, not REMOVE; both need owner sign-off.

## Why F4 and F8 are IMPROVE despite positive/known behavior

L3 found a Critical 1M-edge latency failure on hub-shaped graphs:
`impact --depth 3` 1,132 ms vs 150 ms and `tests` 1,336 ms vs 200 ms.
F4 still has the only robust utility positive (D1), so it is IMPROVE, not remove.
F8 has no demonstrated utility and the same latency defect; fix before any
DEMOTE/REMOVE decision.

## L6 evidence remediation before any future REMOVE

1. Pre-register protocol/prompt/oracle hashes before the first cell.
2. Add a REMOVE rule with minimum effect, >=5 runs, >=2 task shapes/feature,
   confidence intervals.
3. Fix or per-cell audit OpenCode cwd; report exclusion direction.
4. Generate report tables from raw metrics; correct agy means to 18.0/18.0.
5. Use multiset scoring; enforce full control-tool whitelist; audit corpus
   hashes, build success, writes, network, and feature-first sequence.
6. Rerun GT at pinned revisions and record raw outputs.
