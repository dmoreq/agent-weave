# L1 — Correctness / GT lens (R14)

You are one independent Goldfish reviewer. Vote on F1-F11 feature value at
commit `5706dff`. Do not edit files.

Read:
- `docs/baselines/reviews/R14-p8/README.md`, `remediation.md`;
- R13 factual feature sheets `docs/baselines/reviews/R13-p8/evidence/features/F1.md` … `F11.md`;
- source, tests, fixtures, and GT construction docs;
- `docs/baselines/reviews/R14-p8/evidence/utility-limitations.md` for context only.

Forbidden: R13 `reports/`, `decision-matrix.md`, `R13-P8-keep-improve-remove.md`;
utility report interpretation/verdict; another reviewer's report; README/skill
claims as evidence. Do not open expected GT JSON until you derive the expected
set from source.

Task:
1. For each feature, audit correctness against source-derived ground truth and
   committed fixtures. Re-derive at least one expected set yourself.
2. Run at least one positive and one negative probe per feature where possible
   (`target/release/kg --root <fixture> ...`; build fresh fixtures under
   `/tmp`, not in the repo).
3. Separate missing evidence from wrong behavior. A thin GT is not proof of
   uselessness.
4. Decide KEEP / IMPROVE / REMOVE / DEMOTE / DEFER per the R14 rule with
   confidence and evidence.

Write `docs/baselines/reviews/R14-p8/reports/L1.md` with:
- table: Feature | Decision | Confidence | Correctness evidence | Unsupported claims | Required work | Owner action;
- findings Critical / Important / Suggestion with path/symbol or command output;
- evidence gaps.

Return a short summary with the table and Critical findings.
