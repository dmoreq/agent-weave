# Documentation

aw provides local, static code navigation. Start with the task you need to complete, then inspect the returned source locations.

## Start using aw

| You want to… | Read |
|---|---|
| Install from a clone and query aw's own repository | [Getting started](getting-started.md) |
| Inspect callers, dependencies, routes, and cleanup candidates | [Usage guide](usage.md) |
| Look up commands, options, defaults, output, or exit codes | [CLI reference](cli-reference.md) |
| Understand symbols, edges, pointers, and static limits | [Concepts](concepts.md) |
| Connect a coding agent through a skill or MCP | [Agent integration](agent-integration.md) |
| Diagnose an installation or query problem | [Support](../SUPPORT.md) |

All navigation results come from an index, not a running program. Empty results do not prove absence; rebuild after source changes and keep source search available.

## Evaluate the tool

[Evaluation](EVALUATION.md) explains reported measurements, comparison groups, and limitations. Distinguish **no aw** comparisons from **feature-disabled** comparisons that still permit other aw commands.

Neither fewer tool calls nor a correct set of sampled pointers proves whole-program coverage. Check the task, host, sample, and measured metric before applying a result elsewhere.

The [public pointer benchmark](../bench/README.md) is a small regression corpus you can run locally and in CI. It checks navigation output on known source; it does not reproduce the private agent-utility experiments.

## Integrate or contribute

| Document | Purpose |
|---|---|
| [Agent skills](../skills/README.md) | Repository-provided skill and installation scope |
| [Contributing](../CONTRIBUTING.md) | Development setup, contribution rules, and review requirements |
| [Technical specification](designs/2026-10-31_agent-weave-rust-code-index.md) | Technical contract and dated amendments; use amendments when earlier descriptions conflict |
| [Graft-learning decisions](designs/2026-11-05_graft-learning-decisions.md) | Proposed annotation, hybrid-search, and packaging options; not approved |
| [Changelog](../CHANGELOG.md) | Version history and user-visible changes |
| [Security](../SECURITY.md) | Safety boundaries and vulnerability reporting |
| [Code of conduct](../CODE_OF_CONDUCT.md) | Participation and review standards |

The user guides describe the current command interface; design documents also contain historical proposals and decisions. For exact option spelling, consult the [CLI reference](cli-reference.md) or `aw <COMMAND> --help`.
