# Surface inventory

Sources:
- `src/cli.rs` — command/flags.
- `src/mcp.rs` — MCP tools, required args, caps.
- `src/main.rs` — journal args, exits, payload routing, resolve caps.
- `src/journal.rs` — usage.log/usage-results.jsonl contracts.
- `README.md`, `skills/kg-first/SKILL.md`, `/Users/quy.doan/Workspace/personal/agent-skills/skills/code-index/SKILL.md`.
- `docs/evaluation.md` §10-11.

For every feature/candidate, derive:
- CLI command and flags;
- MCP tool and schema;
- journal args and payload;
- exit/cap/no-result contract;
- tests and GT;
- docs/skills lines;
- removal/demotion surfaces.
