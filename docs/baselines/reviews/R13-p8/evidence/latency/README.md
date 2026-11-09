# Latency / scale evidence

Budgets and prior measurements:
- `docs/archive/designs/2026-11-02_p8-structural-graph_amendment.md` §7.5 and Amendment 21.
- `docs/evaluation.md` §10.
- `tests/scripts/timing_assert.sh`.
- `tests/scripts/make_synth_corpus.py`.
- `/tmp/kg-r11-a3/synth1m` — 987k-symbol corpus when present.
- `/tmp/kg-eval` — 2,025-file synth corpus when present.

Required method:
- Rebuild the release binary at the frozen baseline.
- Re-run each timing command; do not cite a prior number without a fresh probe.
- Separate default vs path-scoped complexity; report the amended 250 ms path budget.
- Mark any host-load caveat explicitly.
