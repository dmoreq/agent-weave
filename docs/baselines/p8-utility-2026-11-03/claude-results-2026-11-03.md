# P8 Claude Code + DeepSeek pilot results — 2026-11-03

Status: complete. Descriptive pilot, not part of the frozen four-host decision
rule. See `protocol-claude-extension.md`.

## Setup

| Item | Value |
|---|---|
| Host | Claude Code 2.1.317 |
| Endpoint | `https://api.deepseek.com/anthropic` |
| Model | `deepseek-flash` |
| Binary | kg `6dbe57a934cc9589af3e5c54aba4d13cd44c74e0` (`kg 0.2.1`) |
| Cells | 20 (`5 tasks x 2 arms x 2 runs`) |
| Credential | `DEEPSEEK_API_KEY` injected per subprocess; not persisted |

Command:

```sh
python3 -I run_p8.py --hosts claude --suffix=-claude \
    --kg /tmp/kg-p8-baseline-6dbe57a/target/release/kg
```

The gate cell `claude-kgp8-r1-I1` ran first and passed before the other 19
cells started. Every corpus `.kg` directory was moved aside before the run, so
the baseline binary built fresh indexes.

## Outcome

- Answers: 20 of 20 present.
- Correctness: 20 of 20 exact set matches (`score_p8.py`, 0 failures).
- Violations: 0.
- Control arms: 0 target-feature calls in all 10 cells.
- Treatment arms: 2 target-feature calls in all 10 cells.
- Direct SQLite/index reads: 0.
- Maximum host tool calls: 8 (limit 25).
- Maximum cell duration: 18.8 s (limit 600 s).
- Total durations: 205.3 s.
- Tokens: 284,556 input, 21,249 output, 1,120,768 cache reads.

## Per-task aggregates

| host | task | arm | n | correct | tools | kg calls | feature calls | input tok | cache tok | output tok | seconds |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| claude | I1 | kgc | 2 | 2/2 | 3.0 | 0.0 | 0.0 | 12424 | 36672 | 1064 | 9 |
| claude | I1 | kgp8 | 2 | 2/2 | 4.0 | 2.0 | 2.0 | 12368 | 46528 | 714 | 9 |
| claude | T1 | kgc | 2 | 2/2 | 3.5 | 0.0 | 0.0 | 15024 | 49856 | 670 | 8 |
| claude | T1 | kgp8 | 2 | 2/2 | 5.5 | 2.0 | 2.0 | 15192 | 78656 | 886 | 11 |
| claude | H1 | kgc | 2 | 2/2 | 6.5 | 0.0 | 0.0 | 15262 | 58880 | 1222 | 10 |
| claude | H1 | kgp8 | 2 | 2/2 | 4.5 | 2.0 | 2.0 | 14972 | 64256 | 918 | 10 |
| claude | D1 | kgc | 2 | 2/2 | 6.0 | 0.0 | 0.0 | 15260 | 58368 | 1380 | 12 |
| claude | D1 | kgp8 | 2 | 2/2 | 6.5 | 2.0 | 2.0 | 15699 | 89152 | 2352 | 19 |
| claude | A1 | kgc | 2 | 2/2 | 2.5 | 0.0 | 0.0 | 12510 | 29696 | 608 | 6 |
| claude | A1 | kgp8 | 2 | 2/2 | 4.0 | 2.0 | 2.0 | 13568 | 48320 | 809 | 9 |

## Arm deltas

| task | tool delta | output-token delta | duration delta |
|---|---:|---:|---:|
| I1 | +33% | -33% | -2% |
| T1 | +57% | +32% | +43% |
| H1 | -31% | -25% | +2% |
| D1 | +8% | +71% | +55% |
| A1 | +60% | +33% | +36% |

The treatment arm never reduced total tool calls by the 20% economy threshold.
Correctness was already 2/2 in every control cell, so the correctness-win
condition cannot fire. Under the frozen rule this host contributes no KEEP;
the frozen rule is not applied here because it requires three valid hosts.

## Per-cell evidence

| runid | task | arm | correct | tools | kg | feature | in tok | cache tok | out tok | s |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| claude-kgc-r1-A1 | A1 | kgc | OK | 2 | 0 | 0 | 12991 | 24192 | 586 | 5.2 |
| claude-kgc-r2-A1 | A1 | kgc | OK | 3 | 0 | 0 | 12029 | 35200 | 631 | 7.7 |
| claude-kgp8-r1-A1 | A1 | kgp8 | OK | 4 | 2 | 2 | 13540 | 48256 | 705 | 8.1 |
| claude-kgp8-r2-A1 | A1 | kgp8 | OK | 4 | 2 | 2 | 13595 | 48384 | 913 | 9.5 |
| claude-kgc-r1-D1 | D1 | kgc | OK | 7 | 0 | 0 | 15180 | 58240 | 1390 | 12.0 |
| claude-kgc-r2-D1 | D1 | kgc | OK | 5 | 0 | 0 | 15341 | 58496 | 1369 | 12.0 |
| claude-kgp8-r1-D1 | D1 | kgp8 | OK | 8 | 2 | 2 | 15830 | 104448 | 2308 | 18.8 |
| claude-kgp8-r2-D1 | D1 | kgp8 | OK | 5 | 2 | 2 | 15568 | 73856 | 2397 | 18.5 |
| claude-kgc-r1-H1 | H1 | kgc | OK | 6 | 0 | 0 | 15235 | 58752 | 973 | 8.9 |
| claude-kgc-r2-H1 | H1 | kgc | OK | 7 | 0 | 0 | 15288 | 59008 | 1472 | 11.1 |
| claude-kgp8-r1-H1 | H1 | kgp8 | OK | 4 | 2 | 2 | 14806 | 56448 | 698 | 9.5 |
| claude-kgp8-r2-H1 | H1 | kgp8 | OK | 5 | 2 | 2 | 15139 | 72064 | 1139 | 11.0 |
| claude-kgc-r1-I1 | I1 | kgc | OK | 3 | 0 | 0 | 12245 | 36352 | 1012 | 9.2 |
| claude-kgc-r2-I1 | I1 | kgc | OK | 3 | 0 | 0 | 12602 | 36992 | 1115 | 9.3 |
| claude-kgp8-r1-I1 | I1 | kgp8 | OK | 4 | 2 | 2 | 12373 | 46720 | 779 | 9.4 |
| claude-kgp8-r2-I1 | I1 | kgp8 | OK | 4 | 2 | 2 | 12362 | 46336 | 648 | 8.7 |
| claude-kgc-r1-T1 | T1 | kgc | OK | 3 | 0 | 0 | 14822 | 42624 | 519 | 6.7 |
| claude-kgc-r2-T1 | T1 | kgc | OK | 4 | 0 | 0 | 15225 | 57088 | 822 | 8.3 |
| claude-kgp8-r1-T1 | T1 | kgp8 | OK | 7 | 2 | 2 | 15594 | 101120 | 1155 | 13.6 |
| claude-kgp8-r2-T1 | T1 | kgp8 | OK | 4 | 2 | 2 | 14791 | 56192 | 618 | 7.8 |

## Notes and limitations

- Every treatment cell called the pinned command twice. The first call omitted
  `--root` and failed in the non-git corpus with
  `kg: usage: no git root ancestor found — pass --root <path>`; the second call
  added `--root` and succeeded. The measured tool counts include this recovery
  call. Subtracting one tool per treatment cell changes the mean from 4.9 to
  4.0 tools per cell against 4.3 for control (-7% overall). H1 remains a clear
  win (-46%); T1 and A1 remain worse; I1 becomes equal; D1 becomes -8%. The
  recovery turn also inflates treatment duration.
- The same two-call pattern appears in the frozen four-host rows for most
  treatment cells, so it is not specific to Claude Code.
- Every cell wrote `[claude-code:unrecognized_model]` to stderr. The warning is
  non-fatal: all 20 calls completed and the transcript recorded
  `model = deepseek-flash`.
- Single host. The model and serving provider differ from the four frozen
  hosts, so token and latency values are not like-for-like.
- `kg_calls` uses the larger of the journal count and the transcript count.
  Journal counts can be lower because one journal file covers a cell.
- Results are descriptive. They do not alter the frozen four-host evidence or
  its decision rule.

## Evidence

- `metrics-p8-claude.jsonl` (20 rows)
- `status-claude.txt` (20 start/done pairs, 0 errors)
- `manifest-claude.txt` (20 rows)
- `logs-claude/` (20 stream logs, 20 stderr files, prompt files, usage logs)
- `answers-claude/` (20 answer copies)
- Frozen `metrics-p8.jsonl`, `status.txt`, and `manifest.txt` hashes matched
  the pre-run values.
