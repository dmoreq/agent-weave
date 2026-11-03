# Blinding rules

Value lenses L1-L5 must not read:
- R8-R12 verdicts, finding lists, fix sections, O-items.
- `docs/baselines/p8-utility-2026-11-03.md` §4 Interpretation and §6 Verdict.
- `docs/EVALUATION.md` §11 verdict column.
- README/CHANGELOG/skill claims as evidence.
- The old plan file or any removal shortlist.
- Another reviewer's report.

Value lenses may read:
- Feature sheets, raw protocol, per-cell metrics, prompts, scoring code.
- Source, specs, tests, CLI help, MCP schemas.
- GT construction protocol; L1 must withhold expected JSON until derivation.

L6 may read prior reviews and fix commits, but must not vote feature value.
L6 must state whether the evidence trail is TRUSTWORTHY or INSUFFICIENT.
