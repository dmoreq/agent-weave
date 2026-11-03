# R14 owner decision (2026-11-03)

Owner instruction: confirm KEEP F4/F7 and decide D1-D4 (DEMOTE) or keep
routing.

## Decisions

| Item | Decision | Applied change |
|---|---|---|
| KEEP F4 | **CONFIRMED** | Existing `kg impact` routing kept; latency gate 97-144 ms vs 150 ms; n=5 replication -53.3% tools, 5/5 correct |
| KEEP F7 | **CONFIRMED** | Existing `kg imports` routing kept; n=5 replication -57.1% tools, 5/5 correct |
| D1 F10 `api` | **REMOVE** | `kg api` CLI, MCP tool, and query removed; `symbols.exported` kept for schema compatibility |
| D2 F6 search filters | **REMOVE** | `search --kind/--path/--exact` removed from CLI/MCP/query; base search and `kg resolve` stay |
| D3 F11 filters | **REMOVE** | Six `--exclude-generated` flags removed; tag, `files`, and `stats generated=N` stay |
| D4 F1 `dead` | **DEMOTE** | `kg dead` removed from `kg-first` routing; command stays candidate-only; `implementers` stays routed |
| F2 `--context` | **NOT REMOVED** | L6 INSUFFICIENT and 1/5 REMOVE; no change |
| F11 filters | **REMOVED (D3)** | Covered by D3; tag/stats/files remain |

## Consequences

- D1-D3 removed their CLI/MCP/query surfaces; schema columns and generated tags remain.
- `kg api` and the filter flags are no longer available; `dead` remains an
  explicit command outside routing.
- F4/F7 are KEEP on one host. Cross-host grok/agy replication remains open.
- A future REMOVE decision still requires the L6 checklist.

## Evidence

- `decision-matrix.md`, `owner-packet.md`, `replication-2026-11-03.md`
- `R14-P8-value-re-evaluation.md`
- Commits before this decision: `d4c34ac` (review), code through `cd00e5d`.
