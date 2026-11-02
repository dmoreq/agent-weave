# Search directions lab — evaluation and comparison (2026-11-02)

- Base commit: `c39e21f` (schema v6, includes P8). Three detached worktrees
  under `/tmp/kg-lab`; no main-tree source changes.
- Scope: test the three directions that survived the earlier root-cause work:
  A source-first ranking, B noisy-corpus search utility, C proactive symbol map.
- Hosts: agy 1.4.12 (`gemini-3.8-flash-high`) and zcode 0.1.82
  (`deepseek-ai/DeepSeek-V4.1-Flash-1017`), n=2 for every A/B cell.
- Raw cells, prompts, expected sets, runners, patches: this directory
  (`docs/baselines/search-directions-2026-11-02/`).

## Verdict

| direction | result | decision |
|---|---|---|
| A. Source-first ranking | G14 0.725 -> **0.800** (21/28 -> 23/28); held-out 5/8 **unchanged**; gap 6/6; latency neutral | **Reject as default.** Gate-specific; keep only as an opt-in experiment. |
| B. Search on noisy corpus | 32/32 correct across 4 hosts; tools **−58% to −62%**, input tokens −29% to −59% (Grok output +11%) | **Positive at this task class on 4/4 hosts.** Search utility is real when grep is noisy and the target vocabulary lives in signatures. |
| C. Proactive map | C1 7/8 correct (treatment 4/4); tools 0/+4%, kg calls −22%/−71%, tokens mixed; C2 invalid | **Defer.** No tool-call benefit; 6,980-char context cost; the task needs redesign. |

## Root cause, corrected

- Utility: search replaces one cheap grep call with one kg call. In the
  identifier-vocabulary batteries it cannot save calls; kg calls rose in every
  treatment arm. That is a task-fit problem, not a ranking bug.
- Ranking: field weighting is not the cause. A multi-column FTS5 + `bm25()`
  prototype reproduced the gate exactly and did not beat the flat index
  (best case tie; name boosts lost a symbol). The measurable loss is
  test-symbol crowding; demoting `tests/` recovers two symbols on HEAD's G14.
- The untested cell was noise/scale. Direction B tests it and finds utility.

## Method

- `base` worktree = pristine HEAD. `sourcefirst` and `map` are isolated
  branches with their own `CARGO_TARGET_DIR`.
- Corpora: `kg` (git archive HEAD), `vihap` (252-file copy), `noisy`
  (2,002 files, deterministic generator). Indexes rebuilt with schema v6.
- A gates: `search_recall.py` (G14), gap scorer, 8 held-out queries, journal
  latency on the 2,201-file synthetic corpus.
- B: 2 tasks x 2 arms x 2 hosts x 2 runs = 16 cells. Control = grep/read +
  `kg symbols`; treatment = `kg search` first.
- C: C1 8 cells (2 tasks attempted); C2 4 agy cells were invalid (below).
  Control = search/path/report/grep; treatment = `kg map --limit 40` first.
- Contamination: control feature calls = 0 and treatment >= 1 in every B/C
  cell; answers isolated; corpus `.kg/` usage logs cleared per cell.

## A. Source-first ranking

Implementation: opt-in `--prefer-source`; pool `min(500, max(64, limit*5))`
ordered by bm25, stable demotion of `tests/` paths unless the query asks for
tests; truncate to `limit`. Default output unchanged.

| arm | G14 mean | G14 pooled | gap | held-out |
|---|---|---|---|---|
| flag off | 0.725 | 21/28 | 6/6 | 5/8 |
| flag on | **0.800** | **23/28** | 6/6 | 5/8 |

Latency on 2,201 files: 0.95 ms off vs 0.79 ms on (p95 1.52 vs 1.18 ms).
The two gained symbols are `src.query.callers` and
`vihap.knowledge.registry.get_knowledge_base`; both were crowded out by test
symbols. Held-out queries (4 kg, 4 vihap, frozen before the flag) do not
improve. This is gate-specific and not enough for a default behavior change.
Patch: `a/source-first.patch`; checks: fmt/clippy/tests green in the worktree.

## B. Noisy-corpus search

Corpus: 2,002 files, two targets with signature-only vocabulary
(`mercer_street_tolerance`, `normalize_hex_ward`), 2,000 distractors whose
comments repeat the natural-language phrases. `grep -rl` on the phrases
matches 2,000 files.

| host | arm | tools | kg calls | input tok | cache tok | out tok | seconds |
|---|---|---:|---:|---:|---:|---:|---:|
| agy | control | 10.0 | 2.25 | 58,606 | 182,380 | 3,550 | 86.9 |
| agy | search | **4.25 (−58%)** | **1.25 (−44%)** | **41,642 (−29%)** | **57,190 (−69%)** | **2,274 (−36%)** | **51.1 (−41%)** |
| zcode | control | 11.0 | 3.00 | 20,654 | 137,792 | 1,884 | 33.4 |
| zcode | search | **4.25 (−61%)** | **1.50 (−50%)** | **8,563 (−59%)** | **66,624 (−52%)** | **885 (−53%)** | **12.7 (−62%)** |

Grok Build and OpenCode ran the same corpus, prompts, and GT (16 more cells):

| host | arm | tools | kg calls | input tok | out tok | seconds |
|---|---|---:|---:|---:|---:|---:|
| grok | control | 9.25 | 2.00 | 217,609 | 2,325 | 46.2 |
| grok | search | **3.50 (−62%)** | 1.75 (−12%) | **117,318 (−46%)** | 2,591 (+11%) | 44.1 (−5%) |
| opencode | control | 7.25 | 0.75 | 14,275 | 453 | 51.0 |
| opencode | search | **3.00 (−59%)** | 1.00 (+33%) | **8,793 (−38%)** | **235 (−48%)** | **21.0 (−59%)** |

Correctness: 32/32 across all four hosts. Every host clears the 20%
tool-call bar with parity. Input tokens fall on all four; Grok output tokens
rise 11% (noise). kg calls fall on agy/zcode, are flat on Grok (control
already used `kg symbols`), and rise on OpenCode because the control used
almost no kg at all — the win there is grep/read replacement.

Limits: synthetic corpus and two tasks; the vocabulary is signature-only by
construction. Control can still guess the underscore spelling
(`grep -rl mercer_street` = 1 file), so the result measures a realistic but
favourable noise class, not all real repositories.

## C. Proactive symbol map

Prototype: `kg map --limit N [--path P] [--exclude-generated]`; output
`path:line kind qualname signature fan_in=N`, ordered by distinct call
in-degree. CLI-only; no MCP wiring. Output for 40 symbols: **6,980 chars**
(~1.7k tokens).

C1 (valid orientation task: entry + four first-read files; entry + all four
required):

| host | arm | tools | kg calls | input tok | cache tok | out tok | seconds |
|---|---|---:|---:|---:|---:|---:|---:|
| agy | control | 22.5 | 9.0 | 98,299 | 651,190 | 8,780 | 166.3 |
| agy | map | 23.5 (+4%) | **7.0 (−22%)** | 77,028 (−22%) | 757,994 (+16%) | 8,243 (−6%) | 149.5 |
| zcode | control | 14.0 | 3.5 | 26,756 | 211,584 | 3,186 | 33.2 |
| zcode | map | 14.0 (0%) | **1.0 (−71%)** | 28,133 (+5%) | 221,568 (+5%) | 3,831 (+20%) | 42.7 |

C1 correctness: 7/8. Treatment 4/4 correct; one zcode control listed seven
files (scorer limit: expected + 2) and failed. The map's top-40 contains both
entries and 3/4 expected files per task, but not all four, so the map alone
cannot pass.

C2 (street-name correction) is **invalid**: all four agy cells used 44-48
tools (one timed out at 67), and no cell passed; zcode C2 was stopped. The
task is too broad for the 25-call budget, so it yields no comparison.

Verdict: map reduces kg calls but not tool calls and adds ~7 KB of context;
C1 is parity at best. Defer; if pursued, use a smaller token budget and a
narrower task, and compare against `kg report`'s existing fan-in section.

## Comparison

| dimension | A source-first | B noisy search | C proactive map |
|---|---|---|---|
| problem addressed | top-10 crowding | task-fit / grep noise | orientation without a query |
| measured accuracy | G14 +2 symbols; held-out 0 | 16/16 parity | C1 7/8; treatment 4/4 |
| tool-call effect | not measured in an A/B | **−58% to −62% (4/4 hosts)** | 0% / +4% |
| kg-call effect | n/a | −44% / −50% | −22% / −71% |
| token effect | latency only (neutral) | **input −29% to −59%** | mixed |
| context cost | none | none (call output replaces reads) | +6,980 chars |
| decision | reject default; opt-in experiment | adopt for noisy/signature class | defer; redesign task |

## Threats to validity

- Base moved to P8 (`c39e21f`); baseline G14 on this base is 0.725 (21/28),
  not the historical 0.775. All A comparisons are same-base; do not mix with
  older records.
- B corpus is synthetic, 2,002 files, two tasks; GT audited in two cycles; the final
  round's Critical and Important findings were fixed. Control can guess the underscore
  spelling, so contrast is favourable but not absolute.
- C GT was audited in two cycles; C2 was declared invalid after budget/time
  breaches. C1 control retains `kg report`/`kg hotspots`, so the map's
  incremental value is compared against existing centrality surfaces.
- n=2 per cell; four hosts/models for B, two for A/C; cross-host tokens are not comparable.
- agy exceeded the 25-tool budget in some C cells; B had no violations.

## Reproduction

Artifacts and commands are in
`docs/baselines/search-directions-2026-11-02/README.md`. Key commands:

```bash
# A
python3 -I tests/scripts/search_recall.py --queries docs/baselines/search-gt.jsonl \
  /tmp/kg-lab/target-base/release/kg --kg-root /tmp/kg-lab/corpora/kg \
  --vihap-root /tmp/kg-lab/corpora/vihap
# B
python3 -I docs/baselines/search-directions-2026-11-02/b/run_b.py
# C
python3 -I docs/baselines/search-directions-2026-11-02/c/run_c.py
```
