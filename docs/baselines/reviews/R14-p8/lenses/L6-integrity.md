# L6 — Evidence integrity lens (R14)

You are the evidence-integrity auditor. Do not vote feature value. State
whether the R14 evidence is TRUSTWORTHY or INSUFFICIENT for a REMOVE decision.

Read:
- `docs/baselines/reviews/R14-p8/README.md`, `remediation.md`,
  `evidence/utility-limitations.md`;
- R13 reports, decision matrix, consolidated audit, and raw evidence;
- `docs/baselines/p8-utility-2026-11-03/protocol.md`, `metrics-p8.jsonl`,
  `score_p8.py`, raw logs;
- R13/R14 fix commits and test outputs;
- GT JSONL and construction docs.

Task:
1. Audit independence: did value lenses see forbidden prior verdicts? State
   what the prompt allowed, not what you cannot verify.
2. Audit the R14 pre-registered rule, the utility n/exclusions, and the
   corrected agy means.
3. Audit the latency evidence: corpus hash/shape, timing method, budget,
   host load, and whether the 1.37M corpus is fairly described as adversarial.
4. Audit the GT: derivation protocol, stale rows, cross-file additions, TS
   alias pin, dead probe oracle.
5. Check that test/GT claims in `remediation.md` match raw output.
6. Verdict: TRUSTWORTHY / INSUFFICIENT for REMOVE. List every evidence defect
   that matters.

Write `docs/baselines/reviews/R14-p8/reports/L6.md` with:
- verdict and scope;
- Critical / Important / Suggestion findings with paths or commands;
- a checklist of what a future REMOVE-grade battery must add.

Return the verdict and Critical findings.
