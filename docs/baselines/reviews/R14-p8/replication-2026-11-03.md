# R14 local utility replication (D1/F4, I1/F7)

R14 L2 measured n=2 per cell and rejected KEEP. This replication adds 5 runs
per arm on one host for the two tasks with a positive signal.

## Method

- Host/model: ZCode `deepseek-ai/DeepSeek-V4.1-Flash-1017` subagents, one
  independent session per cell.
- Tasks: frozen P8 prompts D1 (F4) and I1 (F7), unchanged except the treatment
  command uses explicit `--root` and `--no-journal`.
- Arms: control uses source reads/grep only; treatment must run the frozen
  `kg` command first.
- n=5 per arm per task, 20 cells total.
- Corpora: vihap at `d060ace` (D1), kg at `bc292ba` (I1); both are git
  repositories, so no failed first command.
- Scoring: exact set match against `expected/D1.json` and `expected/I1.json`.
- Tool calls: harness `tool_uses`, not the agent's self-report.

## Results

| Task | Control calls | Treatment calls | Median delta | Mean delta | Correct |
|---|---|---:|---:|---:|---:|
| D1/F4 | 15, 20, 16, 8, 12 (median 15) | 6, 6, 7, 8, 7 (median 7) | **-53.3%** | -52.1% | 5/5 vs 5/5 |
| I1/F7 | 7, 9, 8, 4, 7 (median 7) | 4, 5, 3, 3, 3 (median 3) | **-57.1%** | -48.6% | 5/5 vs 5/5 |

Both tasks exceed the R14 KEEP bar (>=25% median tool-call reduction at equal
correctness, n>=5) on this host. D1 treatment had one extra failed attempt in
run 4 (`--no-cache` is not a valid flag); its count is 8, so the median is
unchanged.

## Caveats

- One host/model only; the original battery's grok/agy results are not
  replicated. Host and model remain confounded.
- No token metric was captured; the claim is tool-call only.
- The treatment prompt requires the feature command first, so the comparison
  is feature-first by design.
- Agents self-reported counts that differ by +/-1 from `tool_uses`; the harness
  count is used consistently.
- These are benchmark agents, not the six Goldfish value lenses. They supply
  utility evidence, not a new independent review.

## Effect on R14 decisions

- F4: KEEP under rule (a): 5/5 correct, -53.3% median tool calls, n=5 on one
  host. The 1.37M-edge latency gate also passes (97.1-144.3 ms vs 150 ms).
- F7: KEEP under rule (a): 5/5 correct, -57.1% median tool calls, n=5 on one
  host. The previous host-dependent result is superseded on this host.
- F8/F10 and the unmeasured features are unchanged.
