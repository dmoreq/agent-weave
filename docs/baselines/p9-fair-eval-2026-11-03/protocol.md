# P9 Fair Evaluation — frozen 2026-11-03

Status: prospective evaluation. P8 frozen data is not retro-edited.

Template: [P8 protocol](../p8-utility-2026-11-03/protocol.md) and
[P8 Claude pilot results](../p8-utility-2026-11-03/claude-results-2026-11-03.md).

Withdrawn amendment: [protocol-amendment-2026-11-03.md](protocol-amendment-2026-11-03.md)
is retained for audit only. The user authorized the source change; `aw callers
--depth` is implemented and T1 uses the single handoff command.

**Purpose:** rerun the agent-utility battery after removing five structural
confounds in P8, using current `aw` surfaces.

## Confounds and fixes

| # | P8 confound | P9 fix |
|---|---|---|
| C1 | Corpora lived in `/tmp/` without a git root, so every `aw` call failed without `--root` and cost a retry. | `setup_corpora.sh` copies each corpus into `/tmp/p9-corpus-2026-11-03/`, runs `git init` + commit, and every corpus is a git root. Prompts contain no `--root`. |
| C2 | A1 used `aw api`, removed in `cdd82bf`. | A1 is retired. A2 uses `aw symbols <file>` for indexed interfaces; exported type aliases come from a source read because the index does not extract them. |
| C3 | T1 asked for a direct test caller and the model verified with grep, so both arms used grep. | T1 now asks for test functions that reach a symbol through at least one intermediate call. The route is `aw callers <symbol> --depth 3`; rows carry `[depth=N]`, and definition lines come from `aw symbols`. Grep cannot enumerate the transitive set in one pass. |
| C4 | No pre-built index, so treatment paid `aw build` inside the cell budget. | Every corpus is indexed before any cell. The runner does not build, and every treatment prompt says the index is already built. |
| C5 | Low-baseline host compressed relative gains. | The new host uses `deepseek-v4-pro` instead of `deepseek-flash`. Economy verdicts additionally require the control arm to average at least 3.0 tools; otherwise the cell is labeled `compressed baseline` and cannot qualify by economy. Absolute counts are always reported. |

## Deviations from the handoff, with evidence

1. **T1 target.** The handoff's named target
   `vihap.cleaners.text_utils.exact_text_key` has one direct test and zero
   indirect tests, so it cannot supply 2+ transitive test callers. T1 targets
   `vihap.cleaners.vocab_corrector.load_lexicon`: `aw callers --depth 3`
   reaches one direct caller, then two test functions at distances two and
   three. The `--depth` command is now implemented; the target substitution is
   the only remaining T1 deviation.
2. **A2 alias gap.** `aw symbols` has no `--filter exported` flag, and the index
   does not extract TypeScript type aliases. For
   `packages/core-ledger/src/types.ts`, `aw symbols` reports 15 `interface`
   rows and one `module` row. A2 runs `aw symbols` for the interfaces, then
   reads the file only to collect the 8 exported type aliases. The Trust
   Contract allows a source read when the question asks for a field the output
   does not contain. The A2 oracle is 23 items, matching the handoff's
   interfaces-and-aliases requirement.
3. **Trust Contract scope.** The contract accepts `aw` navigation output as
   ground truth for the fields it returns. Source reads remain required for
   behavioral claims, edits, safety, and review. T1 uses `callers`, which the
   contract lists as ground truth; definition lines come from `symbols`, also
   ground truth.

## Hosts and cells

| host | command family | model |
|---|---|---|
| `grok` | Grok Build CLI | host default |
| `agy` | agy CLI | `gemini-3.8-flash-high` |
| `zcode` | zcode CLI | host default |
| `claude` | Claude Code 2.1.317 | `deepseek-v4-pro` through `https://api.deepseek.com/anthropic` |

OpenCode is excluded: in P8 it executed tools against the primary workspace
instead of the assigned corpus, invalidating its cells.

Claude cells require `DEEPSEEK_API_KEY` in the invoking environment. The key is
injected into the cell subprocess as `ANTHROPIC_AUTH_TOKEN`; it is not written
by the harness.

Valid cells: `5 tasks x 2 arms x 2 runs x 4 hosts = 80`.
`runid = <host>-<arm>-r<run>-<task>`, arms `kgc` (control) and `kgp9`
(treatment).

## Tasks

| task | corpus | target feature | treatment route | expected items |
|---|---|---|---|---|
| `I1` | kg | `aw imports` | `aw imports src/query.rs` | 3 crate-internal modules |
| `T1` | vihap | `aw callers` (transitive) | `aw callers vihap.cleaners.vocab_corrector.load_lexicon --depth 3` | 2 test functions |
| `H1` | vihap | `aw implementers` | `aw implementers vihap.exceptions.VihapError` | 4 direct subclasses |
| `D1` | vihap | `aw impact --edges imports` | `aw impact vihap/reform/temporal.py --edges imports --depth 1` | 3 importer files |
| `A2` | finguard | `aw symbols` | `aw symbols packages/core-ledger/src/types.ts` plus a source read for aliases | 23 items: 15 interfaces + 8 type aliases |

Control arms:
- I1, H1, D1: grep/read plus non-target `aw` commands (`callers`, `callees`,
  `symbols`, `search`). The target command is forbidden.
- T1, A2: grep/read only; no `aw` command.

## Answer contract

One JSON object at the absolute answer path:

```json
{
  "task": "I1",
  "items": ["..."],
  "answer": "<one or two sentences>"
}
```

Item formats: I1 `<line> <module>`, T1/H1 `<path>:<line> <qualname>`,
D1 repo-relative path, A2 `<line> <interface-or-alias-name>`. Order does not matter.
Duplicate rows fail; extra rows fail; missing rows fail. The `answer` field is
scored only for presence.

## Budget and isolation

- Max 25 host tool calls per cell.
- 300-second cell timeout.
- No file modification; no network beyond the host CLI's own model calls.
- No direct `sqlite3` or `.aw/index.db` reads.
- Control cells must make zero target-feature calls.
- Treatment cells must make at least one target-feature call.
- Answers live outside every corpus. The runner appends: do not read outside
  the repository except the exact answer path; do not read prior answers or
  `.aw/index.db`; use only the `aw` CLI and repository files.
- Journals are wiped before and moved after every cell.

## Metrics and decision rule

Per cell: correctness, host tool calls, `aw` calls, target-feature calls,
tokens, wall duration, `aw` latency p50/p95, and violations.

For each `(task, host)` pair:

- correctness parity: treatment correct count >= control correct count;
- economy win: mean total tool calls reduced by >=20%, and control mean >=3.0;
- correctness win: control correct <=1/2 and treatment correct >=3/4.

A task is **KEEP** if it meets its condition on >=3 of 4 hosts. **PARTIAL** if
the condition holds on 1-2 hosts or the direction is inconsistent. **NO VALUE**
otherwise. A control mean below 3.0 is reported as `compressed baseline` and
cannot qualify by economy.

## Oracle freeze

Expected answers were derived from `aw 0.3.0` on the initialized P9 corpora
before any agent cell:

- I1: 3 modules.
- T1: 2 tests; zero direct test callers, two within three caller hops.
- H1: 4 implementers.
- D1: 3 importer files.
- A2: 23 items: 15 interfaces + 8 type aliases.

Dated correction (2026-11-03, before any A2 cell): the A2 oracle was expanded
from 15 interfaces to 15 interfaces + 8 aliases after confirming `aw symbols`
does not index TypeScript type aliases. The original 15-interface commit
`1439a9e` remains in history. Correction commit
`f2d47336830297e8ee1f55e9a12c6bd6af870af6` records the expanded pre-cell
oracle; no A2 cell had run at that point.

Freeze commit: `1439a9e63f6483343ce69bfa2cda1369ed451be3`
Freeze timestamp: `2026-11-03T13:48:10Z`

## Smoke gate

2026-11-03: `python3 -I run_p9.py --smoke` ran two I1 cells on
`claude` + `deepseek-v4-pro` after the oracle commit. Both answers matched the
frozen I1 set. Treatment used 1 `aw` call and 3 tools; control used 0 `aw`
calls and 4 tools. These two rows are preflight validation rows, not the full
battery.

## Route validation (2026-11-03)

Two additional validation pairs ran after the T1 and A2 routes were finalized:

| pair | treatment | control | result |
|---|---|---|---|
| T1 | 3 tools, 1 `callers` + 1 `symbols`, 13.3 s | 7 tools, 0 `aw`, 63.2 s | both exact match |
| A2 | 3 tools, 1 `symbols` + 1 source read, 13.9 s | 2 tools, 0 `aw`, 11.6 s | both 23/23 |

All four cells had zero violations. Including the I1 smoke pair, the six
validation cells scored 6/6 exact match. They are validation evidence, not the
full 80-cell battery.

## Artifacts

- `metrics-p9.jsonl`
- `status-p9.txt`
- `manifest-p9.txt`
- `logs-p9/`
- `answers-p9/`
- `/tmp/p9-utility-answers-2026-11-03/`
- `pre-depth-2026-11-03/` (superseded validation rows; audit only)
