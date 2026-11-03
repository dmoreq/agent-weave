# L2 — Utility / economics lens (R14)

You are one independent Goldfish reviewer. Vote on F1-F11 feature value at
commit `5706dff`. Do not edit files.

Read:
- `docs/baselines/reviews/R14-p8/README.md`, `remediation.md`;
- raw battery: `docs/baselines/p8-utility-2026-11-03/protocol.md`,
  `metrics-p8.jsonl`, `prompts/`, `expected/`, `score_p8.py`;
- `docs/baselines/reviews/R14-p8/evidence/utility-limitations.md`;
- R13 factual feature sheets for the question/surface each task measures.

Forbidden: the utility report's interpretation/verdict sections; R13
`reports/`, `decision-matrix.md`, `R13-P8-keep-improve-remove.md`; another
reviewer's report. Do not treat README/skill claims as evidence.

Task:
1. Recompute the tool-call and token deltas from `metrics-p8.jsonl`; state
   per-cell n and exclusions. Do not trust the published table.
2. For each feature, decide whether any valid task shape shows >=25% median
   tool-call or token reduction at equal correctness with n>=5. If n=2, say
   the bar is not met and classify the evidence as directional only.
3. Distinguish "no value in this task shape" from "feature has no value".
4. Decide KEEP / IMPROVE / REMOVE / DEMOTE / DEFER per the R14 rule.

Write `docs/baselines/reviews/R14-p8/reports/L2.md` with:
- table: Feature | Decision | Confidence | Utility evidence | Delta | n | Missing evidence | Owner action;
- a corrected per-feature delta table from raw rows;
- findings Critical / Important / Suggestion with command or row references.

Return a short summary with the table and Critical findings.
