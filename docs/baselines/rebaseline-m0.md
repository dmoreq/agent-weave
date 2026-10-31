# M0 Re-baseline — post review-fix cycle (2026-10-31)

- Trigger: commit `e891693` (fix(review)) changed observable behavior:
  schema v2, ns-mtime fast-path key, dirty closure, MRO AMBIGUOUS edges,
  hotspots multi-language. All recorded gate numbers predate it.
- Binary: `target/release/kg` at HEAD `e891693`, release build.
- vihap at GT rev `f7ca48c` (unchanged); finguard at `dddfe07` with uncommitted
  working-tree edits (see G2-finguard note below); kg self-index at HEAD.

## Gate results

| gate | recorded (pre-fix) | re-measured | target | verdict |
|------|--------------------|-------------|--------|---------|
| G1 build | 1.91s / 21.4MB / 1.9MB DB | 0.46-0.58s / 22.8-23.1MB / 1.98MB DB (3 runs, `--force`, warm FS cache) | < 5s / < 100MB / < 50MB | PASS |
| G2 vihap | recall 29/29, precision 493/495 | recall 29/29, precision 493/495 (same 2 known call-site-verify failures) | >= 70% / >= 80% | PASS |
| G2 finguard | 20/20, 57/57 | 20/20, 57/57 | — | PASS |
| G2 self | 9/10, 14/14 | 9/10, 14/14 | — | PASS |
| G3 query p95 | 10.1ms | 9.8ms (30 runs, spawn-inclusive, `callers normalize_ascii`) | < 20ms | PASS |
| G7 hotspots | 8/10 rank-overlap | 8/10 (provisional pain list; degraded mode) | >= 7/10 | PASS |
| G6 utility A/B | -50% calls, -67% bytes | NOT re-run — agent task sessions with the `g` logger required | >= 30% calls | PENDING |
| G8 extensions A/B | -67% calls | NOT re-run — same harness requirement | >= 30% | PENDING |

## Notes

- G2-finguard G02: the finguard working tree moved after GT fill (parser.ts
  uncommitted edits; def 110 -> 117, call site 107 -> 114). GT row refreshed
  after direct code read; the resolution itself was correct throughout (the
  ambiguous-candidate list carried the true qualname). Not a kg regression.
- G2-self G01 (`crate::query::file_changed` at doctor.rs:76) is the known
  pre-existing recall gap: `crate::` 3-segment paths through mod trees do not
  resolve when the root dir segment is not itself an indexed module. This is
  the concrete target for phase M2 (recall gaps).
- G1 wall-time improvement (1.91s -> 0.47s) is attributed to warm FS cache and
  machine state, not code changes; no per-phase timing regression observed.
- G6/G8 pending rationale: they measure agent tool-call/byte behavior across
  task sessions, not index properties. The fix cycle can only have improved
  the index side (more carried edges, AMBIGUOUS first-class), but per the
  honesty rule the old numbers stay marked as P1/P1.5-era measurements until
  re-run. Re-run belongs to M3 preparation (the MCP surface will need the
  same A/B anyway).
