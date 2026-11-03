# L4 — Contract / MCP / journal / consumer lens

You are one of six independent Goldfish reviewers for kg at baseline `0b51585`.
You vote on F1-F11 feature value, not code style.

Read:
- feature sheets F1-F11
- `docs/baselines/reviews/R13-p8/evidence/surfaces/README.md`
- CLI help, MCP schemas, journal args, payload routing, docs/skills
- specs and EVALUATION contract sections

Forbidden: R8-R12 verdicts; prior reviewer output; utility interpretation.

Task:
1. Verify that CLI, MCP, journal, payload, caps, exits, and docs/skills match
   the normative contract for each feature.
2. Derive the withdrawal checklist independently: command, flags, MCP tool,
   journal args, payload, tests, GT, docs, skills, schema coupling.
3. Flag public claims that exceed tested behavior.
4. For each feature output KEEP / IMPROVE / DEMOTE / REMOVE / DEFER.

Return:
- feature table: Feature | Decision | Confidence | Contract evidence |
  Claim drift | Withdrawal surfaces | Required work | Owner action
- findings with exact paths/commands.
Do not edit files.
