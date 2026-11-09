# R14 — P8 value re-evaluation after remediation

Baseline: `e74deab` (R13 remediation plus R14 fix cycle). Six independent Goldfish lenses,
one per role, two concurrent per wave. L1-L5 vote feature value; L6 audits
evidence integrity and does not vote.

## Scope

F1-F11. F12/P9 stays deferred. Schema v7/carry/probe are enablers, not
user-facing features.

## Pre-registered decision rule (frozen before wave 1)

| Decision | Condition |
|---|---|
| KEEP | Correctness passes the applicable GT/fixtures; no open Critical; and either (a) utility evidence shows >=25% median tool-call or token reduction at equal correctness on at least one valid task with n>=5, or (b) the feature is a structural dependency required by a KEEP feature and has no independent removal cost saving |
| IMPROVE | The question is real and has a consumer; a fixable defect or evidence gap remains; utility is plausible but not yet demonstrated |
| DEMOTE | No utility evidence, no KEEP dependency, and the claim exceeds the evidence; keep the code, remove it from routing/skill claims or mark it experimental |
| REMOVE | >=4/5 value lenses vote REMOVE, L6 is TRUSTWORTHY for the evidence used, and a removal packet lists surfaces, dependencies, and residual risk |
| DEFER | Evidence is missing or the task shape cannot be measured with the available data |
| OWNER | Lenses split or the decision is a trade-off the owner must make |

REMOVE cannot rest on a single narrow battery. A lens that cannot distinguish
"feature has no value" from "this task shape did not show value" must vote
DEFER or IMPROVE, not REMOVE.

## Evidence

- R13 factual feature sheets: `docs/baselines/reviews/R13-p8/evidence/`
  (read feature sheets, GT, latency, surfaces, dependencies; do not read
  `reports/`, `decision-matrix.md`, or the consolidated R13 audit).
- Remediation summary: `docs/baselines/reviews/R14-p8/remediation.md`.
- Utility limitations: `docs/baselines/reviews/R14-p8/evidence/utility-limitations.md`.
- Latency baseline: `docs/baselines/p8-latency-2026-11-03.md`.
- Dead precision probe: `docs/baselines/dead-precision-2026-11-03.md`.
- GT: `docs/baselines/{import,inheritance,tests}-gt.jsonl` and
  `docs/baselines/real-corpus-gt-2026-11-02.md`.

L1 must derive expected sets before opening GT JSON. L2-L5 may use the raw
metrics but must state the n and exclusion caveats. L6 may read the R13
reports and fix commits.

## Blinding

Value lenses L1-L5 must not read:
- R13 `reports/`, `decision-matrix.md`, or `R13-P8-keep-improve-remove.md`;
- `docs/baselines/p8-utility-2026-11-03.md` interpretation/verdict sections;
- another reviewer's report.

They may read source, specs, tests, CLI help, raw protocol/metrics, and the
factual evidence paths above. L6 may read prior reviews but must not vote
feature value.

## Output

Each lens writes `docs/baselines/reviews/R14-p8/reports/L{1..6}.md` with:
- feature table: Feature | Decision | Confidence | Evidence | Required work |
  Owner action;
- findings Critical / Important / Suggestion with a path/symbol or command;
- evidence gaps that block a defensible decision.

No code edits in this audit.

## Provenance (recorded 2026-11-03)

Final code commit: `bc292ba` (implementation through `cd00e5d`). Owner routing
decisions: `7579646`; evaluation corrections: `b89cea1`.

| artifact | baseline |
|---|---|
| L1 correctness report | `5706dff` |
| L2 utility report | `5706dff` |
| L3 latency report | `d714f29` (re-checked; original `5706dff`) |
| L4 contract report | `5706dff` |
| L4 verification | `62b781d` / `bc292ba` |
| L5 removal report | `e74deab` |
| L6 integrity report | `e74deab` |

Rule and lens hashes at freeze:

```
2b29054d17bdbd291e3d6f2898bbbd63cc9330508514d990ba48f6fba292616a  README.md
eca4461929e74600ceec4e24768897847b9a145088252d92d78a40b68e773082  lenses/L1-correctness.md
117b43a5034f5b0060e2dc47a4d798e912912e3c492445138b7ad229b854f1b8  lenses/L2-utility.md
7ac625afa231f41ad6c3f99f56722a460db1817d2baebdf8bffbcc59d0335f79  lenses/L3-latency.md
f93fbe4e869e00e92bbc947d48b709a9f164808330b477f5219eb488df1d5414  lenses/L4-contract.md
ea7c54044712bd5224d81ed568258f5f3beb4f2c8d8e05dd7e043b86c802cbb6  lenses/L5-removal.md
1c7ba88ba744f04c10b7fee622ab5fa4e7b8f2faa83ffedf6a62a0011ba97ef0  lenses/L6-integrity.md
```

Known deviations recorded by L6: the rule file was extended for the L5/L6 wave
after L1-L4 ran; L4 and L5 disclose a grep that returned prior-verdict lines in
`docs/evaluation.md`; the reports were untracked during the audit and are
committed with this record. These deviations do not change the L6 verdict:
INSUFFICIENT for REMOVE.
