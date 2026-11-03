# P8 Claude Code + DeepSeek pilot — 2026-11-03

Status: pilot extension. It does not alter the frozen four-host cells or the
frozen decision rule.

**Purpose:** add Claude Code as a fifth host so the same P8 tasks can be
observed under a DeepSeek-served Anthropic-compatible endpoint.

## Host and provider

| Item | Value |
|---|---|
| Host CLI | Claude Code 2.1.317 (native install) |
| Endpoint | `https://api.deepseek.com/anthropic` |
| Model | `deepseek-flash` (recorded per cell from the `system/init` event) |
| Credential | `DEEPSEEK_API_KEY` from the invoking environment |
| Credential handling | Injected into the cell subprocess as `ANTHROPIC_AUTH_TOKEN`; not persisted by the harness |

Inherited `ANTHROPIC_API_KEY` and `CLAUDE_CODE_OAUTH_TOKEN` are removed for
Claude cells so the DeepSeek gateway cannot be shadowed by ambient auth.

## Binary and corpora

- Binary: kg `6dbe57a934cc9589af3e5c54aba4d13cd44c74e0`
  (`fix(p8): close R12 review findings`, `kg 0.2.1`), built in a detached
  worktree at `/tmp/kg-p8-baseline-6dbe57a`.
- Rebuild:
  ```sh
  git worktree add --detach /tmp/kg-p8-baseline-6dbe57a 6dbe57a
  (cd /tmp/kg-p8-baseline-6dbe57a && cargo build --release --locked)
  ```
- Corpora: `/tmp/p8-utility-corpus-2026-11-03/{kg,vihap,finguard}`. Each
  corpus `.kg` directory is moved aside before the first cell, so the baseline
  binary builds a fresh index.
- The baseline binary is required for task `A1`: current `main` removed
  `kg api` in `cdd82bf`.

## Cells

`5 tasks x 2 arms x 2 runs = 20`, with `runid = claude-<arm>-r<run>-<task>`.
Prompts are the unchanged `prompts/<arm>-<task>.txt` files plus the same
isolation appendix as the frozen run.

Invocation per cell:

```sh
claude -p <prompt> \
  --output-format stream-json --verbose \
  --dangerously-skip-permissions \
  --model deepseek-flash \
  --add-dir /tmp/p8-utility-answers-2026-11-03-claude
```

`--add-dir` lets Claude Code write the answer file outside the corpus working
directory. Isolation remains prompt-level, as for the other hosts.

## Artifacts

- `metrics-p8-claude.jsonl`
- `status-claude.txt`
- `manifest-claude.txt`
- `logs-claude/`
- `answers-claude/`
- `/tmp/p8-utility-answers-2026-11-03-claude/` (working answer directory)

The frozen `metrics-p8.jsonl`, `status.txt`, `manifest.txt`, `answers/`, and
`logs/` are not written by this pilot.

## Limitations

- Single host; results are descriptive and do not produce KEEP/PARTIAL
  verdicts under the frozen 3-of-4-host decision rule.
- The model and serving provider differ from the four frozen hosts, so token
  and latency comparisons are not like-for-like.
- `claude` cells use the same 25-tool-call and 600-second cell limits as the
  frozen run.
