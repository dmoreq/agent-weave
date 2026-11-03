# GT evidence index

Available:
- `docs/baselines/real-corpus-gt-2026-11-02.md` — construction protocol and exclusions.
- `docs/baselines/import-gt.jsonl` — 40 source-derived import rows.
- `docs/baselines/tests-gt.jsonl` — 20 vihap test seeds.
- `docs/baselines/inheritance-gt.jsonl` — 11 vihap/finguard inheritance rows.
- `tests/scripts/import_recall.py`, `tests_recall.py`, `inheritance_recall.py`.
- `docs/baselines/path-gt.jsonl`, `search-gt.jsonl` — older G14/G17 assets, not P8 GT.

Rules:
- L1 must not open expected JSON until it has independently derived the expected set from source.
- Other lenses may inspect construction and scores, but must not treat them as utility evidence.
- Local-only gates are not CI-enforced; record that explicitly.
