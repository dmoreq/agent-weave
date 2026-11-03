# R14 cross-host utility replication (D1/F4, I1/F7)

Extends `replication-2026-11-03.md` (zcode n=5) to grok and agy. Uses the
frozen P8 prompts and the existing `run_p8.py` runner; corpora are real git
repositories, so no failed first command.

## Method

- Hosts/models: grok `grok-4.7-build`, agy `gemini-3.8-flash-high`, zcode
  `deepseek-v4.1-flash` (local subagent run).
- Tasks: D1 (F4) on vihap `d060ace`; I1 (F7) on kg `bc292ba`.
- n=5 per arm per host. Partial extra agy I1 runs are shown because one
  control was wrong and one used `kg` (excluded).
- Scoring: exact set match against `expected/D1.json` and `expected/I1.json`.
- Tool counts: harness `tools`; no token claim.
- Validity: treatment must run the feature command; control must not run any
  `kg`. Journal counts were contaminated by concurrent I1 cells on the shared
  kg repo, so transcript text was the validity source.

## Results

| Host | Task | Control median | Treatment median | Delta | Correct (c/t) | n (c/t) |
|---|---|---|---:|---:|---:|---:|
| grok | D1/F4 | 6 | 5 | **-16.7%** | 5/5 | 5/5 |
| grok | I1/F7 | 6 | 3 | **-50.0%** | 5/5 | 5/5 |
| agy | D1/F4 | 18 | 6 | **-66.7%** | 5/5 | 5/5 |
| agy | I1/F7 | 21 | 5 | **-76.2%** | 6/7 | 5/5 |
| zcode | D1/F4 | 15 | 7 | **-53.3%** | 5/5 | 5/5 |
| zcode | I1/F7 | 7 | 3 | **-57.1%** | 5/5 | 5/5 |

Agy I1 control detail: one run answered `10 src.db` only (wrong) and one run
called `kg imports` (control violation). Excluding the invalid run, the six
correct control runs are 21, 24, 20, 31, 21, 9 (median 21); the delta is
unchanged at -76.2%. Treatment is 5/5.

Grok D1 is the only cell below the 25% bar. The original n=2 battery had grok
D1 at -29.4%; this n=5 run is -16.7%. F4 KEEP is therefore not uniform across
hosts: zcode and agy pass, grok does not.

## Decision impact

- **F7 KEEP** is uniform: -50.0% / -76.2% / -57.1% on grok / agy / zcode,
  equal correctness on all valid cells.
- **F4 KEEP** is confirmed by the owner but is host-dependent: -66.7% (agy) and
  -53.3% (zcode), -16.7% (grok).
- No removal decision changes. F2 and F11 filters remain blocked by L6.

## Caveats

- Tool calls, not tokens.
- Grok's `kg` call parser missed some treatment commands; transcript text
  confirms `kg impact`/`kg imports` ran. Tool counts are unaffected.
- I1 control journal counts can include a concurrent cell's kg call; validity
  used the transcript.
- Agy I1 control has one reasoning error; the conclusion is stated with the
  correct-only n=6 median.
