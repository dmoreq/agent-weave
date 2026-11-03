# L3 — Latency / scale / build-cost lens (R14)

You are one independent Goldfish reviewer. Vote on F1-F11 feature value at
commit `5706dff`. Do not edit files.

Read:
- `docs/baselines/reviews/R14-p8/README.md`, `remediation.md`;
- `docs/baselines/p8-latency-2026-11-03.md`;
- `docs/baselines/reviews/R13-p8/evidence/latency/README.md`;
- P8 amendment §7.5 budgets;
- `tests/scripts/timing_assert.sh`, `tests/scripts/make_synth_corpus.py`.

Forbidden: R13 `reports/`, `decision-matrix.md`, `R13-P8-keep-improve-remove.md`;
another reviewer's report.

Task:
1. Rebuild `target/release/kg` if needed and run fresh best-of-N timings.
   At minimum: `impact --depth 3` and `tests --depth 10` on
   `/tmp/kg-r13-l3/edges1m-sub` (1.37M edges), and the self index.
2. Verify the 150 ms / 200 ms 1M-edge budgets and state the corpus shape.
3. Measure build wall time on `/tmp/kg-r11-a3/synth1m` or the 2,025-file
   corpus; compare with the frozen 1.25x overhead gate if the baseline is
   available. If not available, say so.
4. Check for unbounded scans or new regressions in the changed BFS paths.
5. Decide KEEP / IMPROVE / REMOVE / DEMOTE / DEFER per the R14 rule.

Write `docs/baselines/reviews/R14-p8/reports/L3.md` with:
- table: Feature | Decision | Confidence | Cost summary | Budget | Result;
- raw commands and best-of-N numbers;
- findings Critical / Important / Suggestion with command output;
- host-load caveats.

Return a short summary with the table and Critical findings.
