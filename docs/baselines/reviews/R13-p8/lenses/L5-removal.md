# L5 — Removal cost / coupling / demand lens

You are one of six independent Goldfish reviewers for kg at baseline `0b51585`.
You vote on F1-F11 feature value, not code style.

Read:
- feature sheets F1-F11
- `docs/baselines/reviews/R13-p8/evidence/dependencies/graph.md`
- consumer inventory
- usage journal with the explicit warning that it is not representative
- source/tests/docs as needed

Forbidden: R8-R12 verdicts; prior reviewer output; utility report verdict.

Task:
1. For each feature, map what depends on it and what breaks if removed.
2. Judge demand evidence: cited consumer workflow, skill demand, test usage,
   journal signal. Label journal evidence as weak/non-representative.
3. Decide whether REMOVE is safe, or DEMOTE/IMPROVE is the better action.
4. For each feature output KEEP / IMPROVE / DEMOTE / REMOVE / DEFER.

Return:
- feature table: Feature | Decision | Confidence | Demand evidence |
  Dependencies | Removal risk | Safer alternative | Owner action
- removal candidates with a concrete withdrawal checklist
- findings with paths/commands.
Do not edit files.
