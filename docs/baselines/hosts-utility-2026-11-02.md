# Agent-Utility A/B — agy CLI and zcode CLI (2026-11-02)

- Hosts: `agy` 1.4.12 (`gemini-3.8-flash-high`) and `zcode`
  0.1.82 (`deepseek-ai/DeepSeek-V4.1-Flash-1017`, host default).
- Method: the frozen prompts and expected sets from
  `docs/baselines/tasks-utility.md`; same sanitized corpora; same two arms
  (control = no feature under test; treatment = route to the feature).
  Sequential cells, one live agent session at a time.
- Scope: path P1-P3 x 2 arms x 2 hosts x 2 runs (24 cells); search B1-B2,
  clones C1-C2, context F1-F3 x 2 arms x 2 hosts x 1 run (28 cells).
  Total 52 cells. Search/clones/context are directional (n=1 per cell);
  path keeps the original n=2.
- Command surface: `agy -p=<prompt> --output-format stream-json
  --dangerously-skip-permissions`; `zcode -p <prompt> --output-format
  stream-json --verbose --permission-mode yolo`.
- Runner: `run_hosts_2026-11-02.py`; aggregation:
  `summarize_hosts.py`; raw status/metrics/manifests and per-cell logs are in
  `utility-run-2026-11-02/`.

## Correctness: 52/52

Every valid cell matched its frozen oracle exactly after the one dated
ground-truth correction below. Control cells made zero calls to the feature
under test (checked from transcript commands and, where the host journals,
the kg usage log); every treatment cell called its feature at least once.

## Economics (mean per cell; host-recorded; never pool cross-host tokens)

| host | feature | arm | tools | kg calls | input tok | cache-read tok | out tok | seconds |
|---|---|---|---:|---:|---:|---:|---:|---:|
| agy | path | control | 32.2 | 21.3 | 104,003 | 875,165 | 12,210 | 215 |
| agy | path | **treatment** | **9.3 (−71%)** | **4.0 (−81%)** | **38,897 (−63%)** | **177,427 (−80%)** | **3,406 (−72%)** | **65 (−70%)** |
| agy | search | control | 6.5 | 1.0 | 26,964 | 135,462 | 3,360 | 60 |
| agy | search | **treatment** | **4.0 (−38%)** | 1.5 (+50%) | **14,338 (−47%)** | **103,350 (−24%)** | 3,208 (−5%) | 54 (−10%) |
| agy | clones | control | 11.5 | 0.5 | 46,837 | 267,634 | 7,866 | 99 |
| agy | clones | **treatment** | **9.0 (−22%)** | 1.0 (+100%) | 38,305 (−18%) | **155,716 (−42%)** | **4,060 (−48%)** | **68 (−31%)** |
| agy | context | control | 16.7 | 7.0 | 68,006 | 373,083 | 7,121 | 109 |
| agy | context | **treatment** | **12.0 (−28%)** | **3.0 (−57%)** | **43,809 (−36%)** | **256,057 (−31%)** | **5,033 (−29%)** | 99 (−9%) |
| zcode | path | control | 17.0 | 17.3 | 22,934 | 217,472 | 5,548 | 59 |
| zcode | path | **treatment** | **6.3 (−63%)** | **6.2 (−64%)** | **14,676 (−36%)** | **109,611 (−50%)** | **2,349 (−58%)** | **32 (−46%)** |
| zcode | search | control | 7.0 | 1.0 | 15,307 | 85,376 | 992 | 20 |
| zcode | search | **treatment** | **5.0 (−29%)** | 2.0 (+100%) | 16,843 (+10%) | 85,248 (−0%) | 1,180 (+19%) | **13 (−32%)** |
| zcode | clones | control | 8.0 | 0.0 | 19,455 | 108,544 | 4,322 | 35 |
| zcode | clones | **treatment** | 7.5 (−6%) | **4.5** | 19,062 (−2%) | 87,552 (−19%) | **2,592 (−40%)** | 27 (−22%) |
| zcode | context | control | 9.0 | 3.0 | 17,984 | 110,165 | 2,350 | 27 |
| zcode | context | **treatment** | **4.7 (−48%)** | **1.0 (−67%)** | **13,446 (−25%)** | **89,088 (−19%)** | **1,535 (−35%)** | 22 (−19%) |

## Verdicts

| feature | agy | zcode | reading |
|---|---|---|---|
| `path` | **PASS**: tools −71%, kg −81%, input −63%, correctness 6/6 | **PASS**: tools −63%, kg −64%, input −36%, correctness 6/6 | same direction as Grok/OpenCode; the strongest cross-host result |
| `search` | directional: tools −38%, input −47%, but kg calls +50% | directional: tools −29%, but kg calls +100%, tokens mixed | call bar met at n=1; token/kg economics are host-dependent, not a clean win |
| `clones` | directional: tools −22%, cache −42%, input −18% | **not met**: tools −6%, input −2%, kg calls 4.5 | same host-dependence as Grok (pass) vs OpenCode (fail), now reversed onto agy/zcode |
| `context` | **PASS**: tools −28%, kg −57%, input −36%, correctness 3/3 | **PASS**: tools −48%, kg −67%, input −25%, correctness 3/3 | unlike Grok's −19%, both new hosts clear the 20% tool bar |

### Path per-task shape

The path gain is concentrated on positive routes. Negative calibration (P2)
still costs the treatment arm follow-up verification: agy treatment 14.5
tools / 10.0 kg calls vs control 29.5 / 25.0; zcode treatment 11.5 tools /
16.5 kg calls vs control 14.5 / 23.0, with one zcode run spending 23 kg calls
(no kg-call saving) before accepting the negative. Positive P1/P3:
agy treatment 6-7.5 tools and 1 kg call; zcode 2.5-5 tools and 1 kg call.

## Protocol notes and limitations

1. **Answer isolation was repaired before any valid cell.** The first smoke
   round let agy list `/tmp/utility-answers-2026-11-02` and read a previous
   host's answer. That directory was moved to
   `~/kg-eval-archive/answers-2026-11-02`; valid cells use
   `/tmp/utility-answers-2026-11-02-hosts`, and the prompt now forbids reading
   outside the repository except the answer path. The two contaminated smoke
   cells were discarded and are not in the 52.
2. **Accounting uses transcript commands.** Both CLIs run shell commands
   without inheriting `KG_ARM`/`KG_SESSION` (observed: empty `arm` in the kg
   journal). Counts come from parsing each cell's `stream-json` tool
   commands; the kg journal is a cross-check (zcode journals without arm,
   agy's sandbox does not persist the journal). Control feature calls were
   zero in every cell.
3. **agy exceeded the 25-tool budget in 5 of 6 control path cells**
   (27, 31, 28, 46, 39 tools). They are retained as host behavior, not
   excluded; treatment cells stayed at 7-15 tools. One agy control cell
   (P3-r1) stalled for ~40 minutes after a host/API hang, was killed and
   re-run; the re-run is the scored cell.
4. **F2 context oracle refreshed (dated correction).** The current v0.2.1
   resolves two additional real call sites,
   `vihap/validators/street_validator.py:629` and `:707`; both arms reported
   them, and they are verified source calls. The host battery scores against
   `docs/baselines/expected-hosts-2026-11-02/F2.json`; the shared frozen
   `expected/F2.json` is unchanged so the earlier Stage 5 record remains
   reproducible. This is a resolver improvement relative to the older oracle,
   recorded as a host-specific overlay, not a retroactive edit.
5. **Cross-host tokens are not comparable.** agy reports `cache_read_tokens`
   separately and its control cache reads are much larger; zcode input
   excludes cache reads. Deltas are within-host only.
6. n=1 for search/clones/context; treat those verdicts as directional. Path
   is n=2, matching the earlier protocol.

## Reproduction

```
python3 -I docs/baselines/utility-run-2026-11-02/run_hosts_2026-11-02.py
python3 -I docs/baselines/utility-run-2026-11-02/summarize_hosts.py \
  > docs/baselines/hosts-battery-2026-11-02.md
```
