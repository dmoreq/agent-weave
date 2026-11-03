# P8 Agent-Utility Battery — frozen 2026-11-03

Frozen before any valid cell. Prompts and the decision rule must not be
retro-edited; corrections get a dated note and invalidate affected cells.

**Purpose:** measure the marginal agent value of five P8 surfaces against the
same kg binary with P8 routing disabled.

**Baseline:** kg `6dbe57a`, schema v7, `target/release/kg` rebuilt before runs.

## Arms

- `kgc` — control. The target P8 command is forbidden. Allowed: grep/read and
  legacy kg (`callers`, `callees`, `symbols`, `search`).
- `kgp8` — treatment. The pinned P8 command is run first. Fallback: legacy kg,
  grep/read.

The binary is identical in both arms. Isolation is prompt-level plus journal
contamination checks.

## Hosts and sample size

| host | command family | runs per cell |
|---|---|---|
| `grok` | Grok Build CLI | 2 |
| `oc` | OpenCode CLI | 2 |
| `agy` | agy CLI | 2 |
| `zcode` | zcode CLI | 2 |

Valid cells: `5 tasks x 2 arms x 4 hosts x 2 runs = 80`.

`runid = <hosttag>-<arm>-r<run>-<task>`, e.g. `zcode-kgp8-r1-I1`.

## Tasks

| task | corpus | target feature | treatment route | expected items |
|---|---|---|---|---|
| `I1` | kg | `kg imports` | `kg imports src/query.rs` | three crate-internal module imports of `src/query.rs` |
| `T1` | vihap | `kg tests` | `kg tests vihap.cleaners.text_utils.exact_text_key` | one direct static test candidate |
| `H1` | vihap | `kg implementers` | `kg implementers vihap.exceptions.VihapError` | four direct implementers |
| `D1` | vihap | `kg impact --edges imports` | `kg impact vihap/reform/temporal.py --edges imports --depth 1` | three direct importer files |
| `A1` | finguard | `kg api` | `kg api packages/core-ledger/src/types.ts` | fifteen exported interfaces |

Task-specific prompts live in `prompts/<arm>-<task>.txt`. `{ANSWER_PATH}` is
substituted by the runner.

## Answer contract

One JSON object at the absolute answer path:

```json
{
  "task": "I1",
  "items": ["..."],
  "answer": "<one or two sentences>"
}
```

`items` are exact strings:

- `I1`: `"<line> <module>"`, e.g. `10 src.db`.
- `T1`: `"<path>:<line> <qualname>"`.
- `H1`: `"<path>:<line> <qualname>"`.
- `D1`: repo-relative file paths.
- `A1`: `"<line> <interface-name>"`.

Order does not matter. Duplicate rows fail. Extra rows fail. Missing rows fail.
The `answer` field is not scored except that it must be present.

## Budget and violations

- Max 25 host tool calls per cell.
- No file modification.
- No network.
- No direct `sqlite3` or `.kg/index.db` reads.
- Control cells must have zero target-feature calls.
- Treatment cells must have at least one target-feature call.
- Host-side violations are recorded, not silently dropped.

## Metrics

Per cell: correctness, host tool calls, kg calls, target-feature calls,
tokens (input/output/cache), wall duration, kg-side latency p50/p95, and
violations. Tokens are never pooled across hosts.

## Scoring and decision rule

`score_p8.py` performs exact set equality after whitespace normalization.

For each `(task, host)` pair:

- correctness parity: treatment correct count >= control correct count;
- economy win: mean total tool calls reduced by >=20%;
- correctness win: control correct <=1/2 and treatment correct >=3/4.

A task is **KEEP** if it meets its condition on >=3 of 4 hosts:

- correctness parity + economy win, or
- correctness parity + correctness win.

**PARTIAL** if the condition holds on 1-2 hosts or the direction is
inconsistent. **NO VALUE** otherwise. FAIL/PARTIAL is reported as such.

## Contamination and leakage controls

- Sanitized corpora under `/tmp/p8-utility-corpus-2026-11-03/`.
- Answers outside every corpus.
- The runner appends: do not read outside the repository except the exact
  answer path; do not read prior answers or `.kg/index.db`; use only kg CLI
  and repository files.
- Journals are wiped before and moved after every cell.
- Smoke cells are discarded if any leakage, parser failure, or prompt issue is
  found.

## Dated notes

- 2026-11-03: protocol frozen. No valid cells had run at freeze time.
- 2026-11-03: Claude Code + DeepSeek pilot added as a fifth, non-frozen host.
  See `protocol-claude-extension.md`. Frozen cells and the four-host decision
  rule are unchanged.

## Dated corrections

- 2026-11-03, before final scoring: T1/H1 item matching normalizes the
  symbol to its short name after `path:line`; the file:line is the identity and
  bare vs module-qualified names are equivalent. Prompts were unchanged.
- 2026-11-03, after the first full pass: OpenCode CLI ignored the per-cell
  corpus working directory and executed tools against the primary
  `/Users/quy.doan/Workspace/kg` workspace. Its answers referenced files absent
  from the assigned corpora; all 20 OpenCode cells are therefore invalid
  evidence and excluded from the task verdicts. Raw OpenCode rows remain in
  the metrics file for audit. Valid hosts are `grok`, `agy`, and `zcode`;
  the pre-registered 3-of-4 condition is applied as 3-of-3 valid hosts, which
  is stricter.
- 2026-11-03, review correction: `score_p8.py` now rejects normalized
  duplicate items and reports requested runids that are missing. Re-scoring the
  frozen answers kept the same 4 pre-existing invalid OpenCode failures; no
  task verdict changed.
