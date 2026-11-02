# Search directions lab — artifacts (2026-11-02)

- Base: `c39e21f` (schema v6; detached worktree `/tmp/kg-lab/base`).
- Worktrees: `lab/search-source-first`, `lab/proactive-map` (patches in `a/`, `c/`).
- Corpora: `/tmp/kg-lab/corpora/{kg,vihap,noisy}` (ephemeral); generator in this directory.
- A evidence: `a/` (G14 off/on, gap, held-out, latency, patch).
- B evidence: `b/` (prompts, expected, 16 cells, runner, checker, logs).
- C evidence: `c/` (prompts, expected, 8 valid C1 cells + 4 invalid C2 cells, runner, checker, map40, patch).
- Summary numbers: `summary.json`.
- Report: `../search-directions-2026-11-02.md`.

Reproduction highlights:
```
# baseline
/tmp/kg-lab/target-base/release/kg build --root /tmp/kg-lab/corpora/kg --force
python3 -I tests/scripts/search_recall.py --queries docs/baselines/search-gt.jsonl \
  /tmp/kg-lab/target-base/release/kg --kg-root /tmp/kg-lab/corpora/kg \
  --vihap-root /tmp/kg-lab/corpora/vihap
# B
python3 -I search-directions-2026-11-02/b/run_b.py
python3 -I search-directions-2026-11-02/b/noisy_check.py <runids...>
# C
python3 -I search-directions-2026-11-02/c/run_c.py
python3 -I search-directions-2026-11-02/c/c_check.py <runids...>
```
