# Agent-Utility A/B — Stage 1: `kg path` (Amendment 13)

- Date: 2026-11-02. Frozen protocol: `docs/baselines/tasks-utility.md`
  (frozen before any valid cell). Expected sets: `docs/baselines/expected/
  P1-P3.json`, oracle = the frozen `docs/baselines/path-gt.jsonl` rows
  (hop edges verified EXTRACTED at G17 freeze).
- Corpora: sanitized source-only copies under
  `/tmp/utility-corpus-2026-11-02/{kg,vihap}` (eval artifacts under
  `docs/`, `skills/`, `tests/scripts/` stripped; fresh `kg build` per cell).
  Answers written to `/tmp/utility-answers-2026-11-02/`, copied into
  `utility-run-2026-11-02/answers/` after each cell.
- Hosts: Grok Build 1.0.70 (`--always-approve --max-turns 40`, model
  grok-4.7-build) and OpenCode CLI (`opencode run --auto
  -m openai/gpt-6-sol --format json`).
- Arms: `kgc` control (no `kg path`; compose callers/callees or grep) vs
  `kgp` treatment (route reachability to `kg path` first). Same binary,
  same corpus, routing-level isolation (G13 precedent).
- Runs: 24 cells = 3 tasks × 2 arms × 2 hosts × 2 runs, sequential,
  ABBA arm order per task. Runner: `utility-run-2026-11-02/run_stage1.sh`;
  accounting: `audit.py`; scoring: `path_check.py`.

## Result: PASS on both hosts

Correctness: **24/24 cells** match the frozen oracle (exact `exists` +
exact `hops` + well-formed route) — both arms, both hosts. No
correctness loss from routing to `path`.

### Economics (mean per run; host-recorded; cross-host tokens NOT comparable)

| host | arm | kg calls | tool calls | input tokens |
|------|-----|----------|------------|--------------|
| Grok Build | kgc control | 44.8 | 10.8 | 360,032 |
| Grok Build | **kgp treatment** | **4.0 (−91%)** | **4.7 (−57%)** | **155,956 (−57%)** |
| OpenCode gpt-6-sol | kgc control | 5.2 | 22.3 | 27,003 |
| OpenCode gpt-6-sol | **kgp treatment** | **1.5 (−71%)** | **3.2 (−86%)** | **17,411 (−36%)** |

Per-task kg-call pattern (r1/r2): Grok control P1 `0/20`, P2 `34/21`,
P3 `143/51`; treatment `1/1`, `10/10`, `1/1`. OpenCode control P1 `0/0`,
P2 `3/0`, P3 `16/12`; treatment `1/1`, `4/1`, `1/1`. The treatment arm
answers each route question with **one** `path` call on 8 of 12 runs
(two calls when it re-queries), then stops.

### Contamination guard

All 12 control cells: feature calls = 0 in the kg journal. All 12
treatment cells: `path` calls >= 1, journaled under `KG_ARM=<runid>`.
No cell excluded.

## Findings (honest)

1. **Utility axis flipped vs the P0/G6 navigation battery.** There the
   free-running agent used kg as a supplement and total tool calls went
   UP (grok-a-b-2026-11-01.md finding 1). For reachability questions the
   routed feature is not a supplement: one `path` call replaces a
   multi-query composition. Calls and tokens dropped on BOTH hosts, on
   ALL three tasks.
2. **The control arm can still be correct — at high cost.** Grok control
   P1-r1 answered correctly with zero kg calls (grep/read only, 376K
   tokens); P3-r1 thrashed at 143 kg calls. Correctness is parity; the
   utility is cost.
3. **Negative calibration holds (P2).** The treatment answered "no route"
   correctly in every run; grok control spent 21-34 kg calls to reach
   the same negative, treatment 10 (path + confirmation reads). On
   OpenCode the control sometimes never invoked kg at all (0-3 calls)
   and still answered correctly — a host-behavior difference, not a
   feature difference.
4. **Variance is real at n=2.** Grok control P1 split 0/20 kg calls
   across runs; P3 split 143/51. Means above are descriptive, not
   per-task guarantees (same caveat as the P4 battery).
5. **OpenCode token accounting excludes cache-read**, so its token
   column is not comparable to Grok's; the cross-host comparable metrics
   are tool calls and kg calls (reported per host, never pooled).
6. **Pre-run hardening caught a validity defect.** The first smoke round
   ran against the live kg repo where `docs/baselines/expected/P2.json`
   and `path-gt.jsonl` were readable; the agent answered from the GT with
   0 kg calls. Both smoke rounds were discarded, corpora sanitized and
   answer paths moved outside the repo before any valid cell. This is the
   R0 leakage lesson repeating; the frozen protocol now forbids eval
   artifacts inside the queried corpus.
7. **Process note.** The first background pass was killed mid-cell by an
   external session interrupt; 5 OpenCode cells were re-run. The runner's
   original skip pattern (`^<runid> `) never matched `[start]` lines, so
   two smoke cells were re-executed in the background pass; artifacts
   were overwritten consistently (last run wins, manifest/answer/log/
   journal aligned). Status counts reflect the duplicates; unique cells
   = 24.

## Verdict

**H-P1 (no correctness loss): PASS. H-P2 (fewer calls/bytes): PASS on
both hosts. H-P3 (no-path calibration): PASS.** `kg path` is utility-
positive for route/reachability questions. Scope: 3 tasks × 2 hosts ×
2 arms × 2 runs; static resolved edges only; host/model pair fixed.

## Reproduction

```
# corpora (sanitized, source-only)
git -C <repo> archive HEAD | tar -x -C /tmp/utility-corpus-2026-11-02/<corpus>
rm -rf /tmp/utility-corpus-2026-11-02/kg/{docs,skills,tests/scripts}
python3 docs/baselines/utility-run-2026-11-02/run_stage1.sh      # 24 cells
python3 docs/baselines/utility-run-2026-11-02/audit.py           # audit.json
python3 docs/baselines/utility-run-2026-11-02/path_check.py $(cat runids)  # 24/24
```

## Next stages (not run yet)

- Stage 2 `search`: expected sets must be derived and Goldfish-audited,
  then owner spot-check, per the agreed GT protocol. Runner/prompts are
  frozen in `tasks-utility.md`; prompts S1-S3 still to be generated.
- Stage 3 `clones`: seeded fixture + manifest, same protocol.
