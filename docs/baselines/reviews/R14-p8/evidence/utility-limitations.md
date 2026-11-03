# Utility battery limitations (factual)

The P8 utility battery (`docs/baselines/p8-utility-2026-11-03/`) has these
properties. They bound what any lens may conclude from it.

- 5 tasks, one task shape per feature; n=2 runs per cell.
- 4 hosts attempted; OpenCode cells were invalid because the CLI used the
  primary workspace instead of the corpus. All OpenCode cells are excluded.
- Two agy control cells called a forbidden feature and are excluded.
- After exclusions: 58/60 valid-host cells, still n=2 per cell.
- Corrected agy control means (from raw metrics): I1 18.0, H1 18.0; the
  published table's 19.0/21.0 were transcription errors and are corrected in
  `docs/baselines/p8-utility-2026-11-03.md`.
- `score_p8.py` compares sets, so duplicate answer rows do not fail the cell
  as the protocol text says.
- No REMOVE rule was pre-registered for that battery; R14 pre-registers one
  in `README.md`.
- Utility is measured as tool calls/tokens at equal correctness; a single
  task shape cannot show a feature is useless.
- The battery predates `5706dff`; it measures the P8 feature set at `6dbe57a`
  plus the corrected report.

Use `metrics-p8.jsonl`, `prompts/`, `expected/`, and `score_p8.py` directly.
Do not read the report's interpretation or verdict sections.
