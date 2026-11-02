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

## Stage 2 — `kg search` (F2, Amendment 12b)

Protocol as Stage 1 (frozen `tasks-utility.md` S1-S3; sanitized corpora;
same hosts/arms; 24 cells). GT derived 2026-11-02 and Goldfish-audited
before any cell: the audit rejected the first S3 pointer
(`_ascii_diacritic_trans:23`, a translate-table builder) and the set was
corrected to `normalize_ascii:47`; S2/S3 prompts gained the route-schema
parenthetical; expected sets gained `qualname` and the scorer checks it.

### Correctness: 24/24 both arms

Every cell produced the exact expected pointer + qualname in both arms and
both hosts.

### Economics (mean per run; host-recorded)

| host | arm | kg calls | tool calls | input tokens |
|------|-----|----------|------------|--------------|
| Grok Build | kgc control | 1.0 | 5.67 | 162,110 |
| Grok Build | kgs treatment | 1.67 (+67%) | 4.67 (−18%) | 162,486 (+0.2%) |
| OpenCode gpt-6-sol | kgc control | 1.0 | 5.0 | 17,722 |
| OpenCode gpt-6-sol | kgs treatment | 1.33 (+33%) | 5.17 (+3%) | 11,415 (−36%, cache-contaminated) |

### Verdict: FAIL (utility unproven) — honest negative

The decision rule requires correctness parity AND >= 20% calls/bytes
reduction on >= 2/3 tasks on BOTH hosts. Measured: −18% tool calls on
Grok, +3% on OpenCode; kg calls went UP in both arms; Grok tokens flat.
No bar met on both hosts.

Interpretation: for single-target symbol lookups whose vocabulary appears
in the identifier itself (split identifiers -> `split_ident`, replay
verify -> `verify`, diacritics -> `normalize_ascii`), `grep` already
matches, and one `kg search` call is an extra query rather than a
replacement. The G14 recall gate (0.850) remains valid as retrieval
quality; this A/B does not demonstrate an agent-economics gain at this
task class. `search` stays as-is: quality-gated, utility unproven.

Scope limits (not tested): large corpora where grep output is noisy;
queries whose vocabulary does NOT appear in identifiers (the case where
lexical search should shine). Any future claim needs a task class of that
shape, not a re-run of this battery.

Contamination guard: all 12 control cells feature calls = 0; all 12
treatment cells `search` calls >= 1. No cell excluded.

## Stage 3 — `kg clones` (F4, Amendment 12d)

Protocol as Stages 1-2; 16 cells = C1 (list every duplicated block pair)
+ C2 (near-duplicate rejection) × 2 arms × 2 hosts × 2 runs. Fixture
generated deterministically (`make_clone_fixture.py`): 4 seeded pairs with
an identical 9-line block at exactly 3-11 in both files, one near-duplicate
(all identifiers renamed), 3 unique decoys. Goldfish audit round 1 found
1 Critical (C2 prompt pre-filled the answer), 3 Important (span ambiguity
2-11 vs 3-11, C2 definition underspecified, "answer field" vs `pairs`),
1 Suggestion (index.db readable by the control arm). All fixed before any
cell: unique per-file header lines make raw and normalized maxima coincide
at 3-11; prompts neutral (`"exists": <true or false>`); C2 defines duplicate
as identical normalized text (5+ lines, blank lines ignored); the scorer
reads `pairs`; audit flags zero-tool cells and direct sqlite queries.

### Correctness: 16/16 both arms

Every cell reported the 4 pairs with exact spans (C1) and rejected
near.py (C2).

### Economics (mean per run; host-recorded)

| host | arm | kg calls | tool calls | input tokens |
|------|-----|----------|------------|--------------|
| Grok Build | kgc control | 0.0 | 12.25 | 248,806 |
| Grok Build | kgcl treatment | 1.5 | **5.0 (−59%)** | **151,911 (−39%)** |
| OpenCode gpt-6-sol | kgc control | 0.0 | 5.75 | 10,968 |
| OpenCode gpt-6-sol | kgcl treatment | 1.0 | 4.75 (−17%) | 14,436 (+32%, cache-contaminated) |

### Verdict: PARTIAL (host-dependent)

On Grok the routed arm cuts tool calls 59% and input tokens 39% with
correctness parity — the decision-rule bar is met on both tasks. On
OpenCode the call reduction is 17% (below the 20% bar) and the token
numbers are small and cache-contaminated, so no claim is made. This is
the same host-dependence pattern as the P4 navigation battery
(grok-a-b-2026-11-01.md finding 3): utility transfers only where the host
agent actually follows the routing and avoids file-by-file verification.

Scope limits: 12-file fixture with exact-duplicate seeding; the control
arm had no kg call at all (pure file reading), so the comparison is
tool-assisted audit vs manual audit, not `clones` vs another kg tool.

## Overall

| stage | feature | correctness | economics | verdict |
|-------|---------|-------------|-----------|---------|
| 1 | `path` | 24/24 both arms | kg calls −91%/−71%; tokens −57%/−36% | **PASS** (both hosts) |
| 2 | `search` | 24/24 both arms | tools −18%/+3%; kg calls up | **FAIL** (utility unproven) |
| 3 | `clones` | 16/16 both arms | tools −59%/−17%; tokens −39%/+32% | **PARTIAL** (Grok only) |

All three features keep their quality gates (G14/G16/G17). The utility
A/B adds a second axis: `path` is utility-proven on both tested hosts;
`search` shows no economics gain at the tested task class; `clones`
transfers on Grok, not on OpenCode at n=2.

## Stage 2b — `kg search` after the Amendment 14 upgrade (retest)

Protocol: 16 cells = B1-B2 × 2 arms × 2 hosts × 2 runs on vihap, after the
P6 upgrade (path+signature FTS tokens; signature rendered in hits). The
first Stage-2b GT design was rejected by the Goldfish audit (B2 had 35
equally valid `(str|None)->str` answers; B1 had two exact
`(Any)->list[Any]` matches). Rewritten around a delegation constraint +
signature: B1 = the `list[Any]` coercion helper called by
`_raw_items_from_sklearn_input` (unique, search rank 1); B2 = the
`list[str]` helper called by `_fit_string_sequence` (unique, search rank 1).

### Correctness: 16/16 both arms

### Economics (mean per run; host-recorded)

| host | arm | kg calls | tool calls | input tokens |
|------|-----|----------|------------|--------------|
| Grok Build | kgc control | 1.0 | 7.25 | 242,431 |
| Grok Build | kgs treatment | 2.0 | **4.5 (−38%)** | **174,928 (−28%)** |
| OpenCode gpt-6-sol | kgc control | 1.0 | 6.0 | 13,248 |
| OpenCode gpt-6-sol | kgs treatment | 1.25 | **5.0 (−17%)** | 9,053 (−32%, small numbers) |

### G18(iii) verdict: FAIL by the stated bar (>= 20% on BOTH hosts)

Grok clears the bar (−38% calls, −28% tokens). OpenCode lands at −17%
calls — one call short; its token delta (−32% on 13K→9K) is cache-noisy
and does not rescue the call bar. Per Amendment 14, a (iii) failure keeps
the automatic revert on the table, but the owner may re-decide to keep the
recall improvement (G18 i+ii passed: frozen recall unchanged 0.850, gap
set 6/6 improved) — recorded here, not silently taken.

Scope: 2 tasks, n=2/arm/host; the upgrade demonstrably changes what
search can retrieve, but the agent-economics gain remains host-dependent
(the same pattern as Stages 2 and 3).
