# L4 — Contract / MCP / journal / consumer lens (R14)

You are one independent Goldfish reviewer. Vote on F1-F11 feature value at
commit `5706dff`. Do not edit files.

Read:
- `docs/baselines/reviews/R14-p8/README.md`, `remediation.md`;
- R13 factual surface inventory `docs/baselines/reviews/R13-p8/evidence/surfaces/`;
- CLI help, MCP schemas, journal code, README/skills only to audit claims;
- P8 amendment F1/F4/F6/F8/F10/F11 contract sections.

Forbidden: R13 `reports/`, `decision-matrix.md`, `R13-P8-keep-improve-remove.md`;
another reviewer's report.

Task:
1. Re-derive the withdrawal checklist for each feature: CLI enum/flags, MCP
   tool schema, journal args, payload, tests/GT, docs/skills.
2. Verify the R13 blockers are fixed: `resolve --no-journal` writes no
   journal; MCP blank/non-string names error; resolve journal count reflects
   parsed names.
3. Verify public claims match the binary for F1, F4, F6, F8, F10, F11. Report
   any doc/behavior mismatch with a path and command.
4. Check MCP/CLI parity for success and error shapes; state where
   "byte-identical" is false.
5. Decide KEEP / IMPROVE / REMOVE / DEMOTE / DEFER per the R14 rule.

Write `docs/baselines/reviews/R14-p8/reports/L4.md` with:
- table: Feature | Decision | Confidence | Contract summary | Mismatches | Owner action;
- findings Critical / Important / Suggestion with path/symbol or command output;
- the withdrawal checklist.

Return a short summary with the table and Critical findings.
