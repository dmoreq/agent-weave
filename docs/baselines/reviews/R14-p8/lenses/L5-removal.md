# L5 — Removal cost / coupling / demand lens (R14)

You are one independent Goldfish reviewer. Vote on F1-F11 feature value at
commit `5706dff`. Do not edit files.

Read:
- `docs/baselines/reviews/R14-p8/README.md`, `remediation.md`;
- R13 dependency map and consumer inventory
  `docs/baselines/reviews/R13-p8/evidence/dependencies/graph.md`,
  `docs/baselines/reviews/R13-p8/evidence/surfaces/`;
- usage journal (label it non-representative) and the R14 utility limitations;
- source, CLI help, MCP tool list.

Forbidden: R13 `reports/`, `decision-matrix.md`, `R13-P8-keep-improve-remove.md`;
another reviewer's report.

Task:
1. For each feature, state what breaks if it is removed: commands, flags,
   MCP tools, journal rows, payloads, tests, GT, docs/skills, coupled
   features.
2. Judge demand from the journal and consumers, but do not treat a small
   sample as proof of no demand.
3. Decide whether REMOVE, DEMOTE, KEEP, DEFER, or OWNER is the safer action.
   REMOVE needs an explicit packet; DEMOTE is the fallback when code is cheap
   to keep but claims are unsupported.
4. Identify features whose removal is low-risk and whose evidence shows no
   value, and those whose removal is blocked by coupling.

Write `docs/baselines/reviews/R14-p8/reports/L5.md` with:
- table: Feature | Decision | Confidence | Demand | Removal breakage | Safer alternative;
- a removal packet for every REMOVE/DEMOTE candidate;
- findings Critical / Important / Suggestion with evidence.

Return a short summary with the table and removal candidates.
