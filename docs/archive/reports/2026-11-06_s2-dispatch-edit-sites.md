# S2 dispatch edit-site debt (accepted)

- Date: 2026-11-06
- Scope: `src/service.rs`, `src/cli.rs`
- Origin: second code review of `agent-weave` at `f64e3bd`
- Status: accepted debt

## Decision

Adding or renaming one command still edits 11 sites. The `<= 7` acceptance from
the review is not reachable without either a code-generation macro or merging
the CLI and service enums. Both alternatives are architectural changes with a
larger regression surface than the 11-site cost they remove. This record
accepts the 11 sites as debt.

The owner did not answer the implementation/debt prompt (asked twice). This
record takes the debt option under the review's own three-way choice; it is a
recorded decision, not a claim that the review acceptance was met.

## The 11 sites

1. `src/cli.rs`: `Cmd` variant (clap surface).
2. `src/service.rs`: `cli_family` arm.
3. `src/service.rs`: `from_cli_*` conversion arm.
4. `src/service.rs`: `CommandRequest` variant.
5. `src/service.rs`: `CommandRequest::family` arm.
6. `src/service.rs`: `tool_name` arm.
7. `src/service.rs`: `journal_args_*` arm.
8. `src/service.rs`: `run_*` arm.
9. `src/service.rs`: `from_tool_args` tool-name list.
10. `src/service.rs`: `from_tool_args_*` arm.
11. `src/service.rs`: `TOOL_SPECS` entry.

## Why `<= 7` needs an architectural change

- Four surfaces are mandated by the repository sync gate (`AGENTS.md` §4):
  CLI, service, journal, and MCP/`TOOL_SPECS`.
- `Cmd` and `CommandRequest` are deliberately separate: `cli.rs` owns clap and
  the service stays surface-agnostic. Collapsing them couples the service to
  clap and changes the CLI/MCP parity contract.
- A macro/registry can generate the seven mappings from one declaration, but
  adds a DSL over 24 commands with per-field argument defaults and journal
  formatting. The review ranked that risk above the edit-site saving.

## Alternatives considered

1. Macro/registry: 1-2 edit sites, but a 200-300 line DSL and high regression
   risk.
2. Single clap-derived enum: about 7 edit sites, but service-clap coupling and
   a `resolve --stdin` / nested `MemAction` redesign.
3. Accepted debt (this record): 11 sites, no new abstraction.

## Re-open condition

Revisit if a third command surface appears, if the command count grows enough
that the 11-site edit becomes a frequent error source, or if `cli.rs` and
`service.rs` are merged for another reason.
