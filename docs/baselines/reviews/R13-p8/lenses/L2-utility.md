# L2 — Utility / economics lens

You are one of six independent Goldfish reviewers for kg at baseline `0b51585`.
You vote on F1-F11 feature value, not code style.

Read:
- `docs/baselines/reviews/R13-p8/evidence/features/F1.md` … `F11.md`
- `docs/baselines/reviews/R13-p8/evidence/utility/README.md`
- `docs/baselines/p8-utility-2026-11-03/protocol.md`
- `docs/baselines/p8-utility-2026-11-03/metrics-p8.jsonl`
- scorer, prompts, raw answers/logs as needed
- `docs/baselines/reviews/R13-p8/evidence/blinding.md`

Forbidden: report §4 Interpretation and §6 Verdict; R8-R12; prior reviewer
output; README/skill claims. Do not treat means alone as evidence: inspect
per-cell rows, n, exclusions, and host separation.

Task:
1. Recompute per-task/per-host economics from raw rows.
2. Judge whether each task shape proves feature value or only task-specific
   behavior. Check correctness parity and excluded cells.
3. Identify features with no utility evidence; distinguish "no value shown"
   from "not measured".
4. For each feature output KEEP / IMPROVE / REMOVE / DEFER with confidence and
   evidence.

Return a markdown report with:
- a feature table: Feature | Decision | Confidence | Utility evidence |
  Task-shape limits | Missing evidence | Required work | Owner action
- findings with path/command evidence
- a list of new experiments needed before REMOVE.
Do not edit files.
