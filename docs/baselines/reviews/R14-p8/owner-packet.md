# R14 owner packet — DEMOTE / REMOVE / replication

Status: awaiting owner decision. No routing or claim edits are applied yet.

## A. DEMOTE packets (code stays)

### D1 — `kg api` (F10)

- Evidence: A1 tools +14.3% / +125.0% / +28.6% at equal correctness; 3/3
  hosts negative; 27 journal calls; no real-corpus export JSONL GT.
- Edits: remove `kg api` from `skills/kg-first/SKILL.md:26`; remove the
  `README.md:64` example; mark the command experimental in CLI help or spec.
- Keep: CLI, MCP tool, `symbols.exported`, tests, export GT.
- Risk: low; one routing line and one example.
- Reversal: restore the two lines.

### D2 — search filters (F6)

- Evidence: 1,198 journal search rows, zero with `--kind` / `--path` /
  `--exact`; no utility task.
- Edits: remove the filter list from `README.md:55`; mark filter combinations
  experimental in help/spec. Keep base search and `kg resolve`.
- Risk: low; no code change.
- Reversal: restore the README text.

### D3 — `--exclude-generated` filters (F11)

- Evidence: zero filter uses across 2,913 journal rows; no utility task or GT;
  tag/stats/files are used passively.
- Edits: mark the six filters experimental in help/spec; do not remove the
  flags. Keep `files.generated`, `is_generated`, `stats generated=N`.
- Risk: low; no schema change.
- Reversal: remove the experimental note.

### D4 — `dead` claims (F1)

- Evidence: 6/50 sampled vihap candidates have zero external textual
  references; type/attribute references are known false positives; no
  precision gate.
- Edits: remove the `kg dead` routing line from
  `skills/kg-first/SKILL.md:30`; keep `implementers` routed. Mark `dead`
  candidate-only in help/spec.
- Risk: medium; `dead` is a documented command and has existing tests.
- Reversal: restore the routing line.

## B. REMOVE candidates (blocked)

| Candidate | Votes | Blocker |
|---|---|---|
| F2 `impact --context` | 1/5 REMOVE | L6 INSUFFICIENT; no 4/5; low removal cost but no value evidence |
| F11 `--exclude-generated` filters | 1/5 REMOVE (filters only) | L6 INSUFFICIENT; no 4/5; tag stays regardless |

They are not approved. Removing either now would violate the pre-registered
R14 rule.

## C. F4/F7 replication (completed 2026-11-03)

- Tasks: D1 (F4, `impact --edges imports --depth 1`) and I1 (F7, `imports`).
- Host: ZCode/DeepSeek only; grok/agy not re-run.
- Design: 5 control + 5 treatment runs per task; n=5 per cell; git-rooted
  corpora with explicit `--root --no-journal`; exact set scoring.
- Results: D1 median tools 15 -> 7 (-53.3%), 5/5 correct. I1 median tools
  7 -> 3 (-57.1%), 5/5 correct.
- Outcome: F4/F7 meet KEEP rule (a) on this host. Raw record:
  `replication-2026-11-03.md`. Cross-host grok/agy remains unverified.

## D. Owner choices

1. Approve D1+D2+D3+D4, a subset, or none.
2. Authorize the D1/I1 replica at n>=5 per host, or leave F4/F7 at IMPROVE.
3. Decide whether ambiguous call sites in `impact`/`unresolved` are a code fix
   or a documented limit.
