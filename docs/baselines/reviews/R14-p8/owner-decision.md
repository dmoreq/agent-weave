# R14 owner decision (2026-11-03)

Owner instruction: confirm KEEP F4/F7 and decide D1-D4 (DEMOTE) or keep
routing.

## Decisions

| Item | Decision | Applied change |
|---|---|---|
| KEEP F4 | **CONFIRMED** | Existing `kg impact` routing kept; latency gate 97-144 ms vs 150 ms; n=5 replication -53.3% tools, 5/5 correct |
| KEEP F7 | **CONFIRMED** | Existing `kg imports` routing kept; n=5 replication -57.1% tools, 5/5 correct |
| D1 F10 `api` | **DEMOTE** | Removed `kg api` from `skills/kg-first/SKILL.md` and the README command list; spec marks it experimental |
| D2 F6 search filters | **DEMOTE** | Removed `--kind/--path/--exact` filter claim from README; spec marks the combinations experimental; base search and `kg resolve` stay routed |
| D3 F11 filters | **DEMOTE** | Spec marks the six `--exclude-generated` flags experimental; tag, `files`, and `stats generated=N` stay supported |
| D4 F1 `dead` | **DEMOTE** | Removed `kg dead` from the `kg-first` routing row; spec marks `dead` candidate-only; `implementers` stays routed |
| F2 `--context` | **NOT REMOVED** | L6 INSUFFICIENT and 1/5 REMOVE; no change |
| F11 filters | **NOT REMOVED** | L6 INSUFFICIENT and 1/5 REMOVE; flags kept, marked experimental |

## Consequences

- No schema, CLI, MCP, payload, or journal surface was removed.
- The demoted commands remain available for explicit use; only routing and
  claim surfaces changed.
- F4/F7 are KEEP on one host. Cross-host grok/agy replication remains open.
- A future REMOVE decision still requires the L6 checklist.

## Evidence

- `decision-matrix.md`, `owner-packet.md`, `replication-2026-11-03.md`
- `R14-P8-value-re-evaluation.md`
- Commits before this decision: `d4c34ac` (review), code through `cd00e5d`.
