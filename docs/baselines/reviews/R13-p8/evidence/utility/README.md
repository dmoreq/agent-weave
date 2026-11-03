# P8 utility evidence

Primary files:
- `docs/baselines/p8-utility-2026-11-03/protocol.md`
- `docs/baselines/p8-utility-2026-11-03/metrics-p8.jsonl` — 80 per-cell rows.
- `docs/baselines/p8-utility-2026-11-03/score_p8.py`
- `docs/baselines/p8-utility-2026-11-03/summarize_p8.py`
- `docs/baselines/p8-utility-2026-11-03/expected/` — task oracles.
- `docs/baselines/p8-utility-2026-11-03/prompts/` — frozen prompts.
- `docs/baselines/p8-utility-2026-11-03/logs/`, `answers/` — raw cells.

Read only the raw protocol, metrics, prompts, scoring code, and per-cell rows.
Do not read the report's interpretation/verdict sections.

Disclosures:
- OpenCode cells are invalid: the CLI ignored corpus cwd and used the primary kg workspace.
- Two agy control cells called the forbidden feature and are excluded from decisions.
- T1/H1 scoring normalizes module-qualified names to short names after `path:line`.
- 58/60 valid-host cells remain; n=2 per cell; results are directional.
