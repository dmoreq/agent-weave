# Documentation Index

Welcome to the `agent-weave` (`aw`) technical documentation.

`aw` is a consultative, offline code knowledge index, real-time anti-reinvention engine, and zero-token persistent memory store for AI coding agents.

---

## User & Agent Guides

| Guide | Description |
|---|---|
| **[Getting Started](getting-started.md)** | Installation, first 5 minutes with the 3 USPs, language coverage, and index reset. |
| **[Usage Guide](usage.md)** | Task-oriented engineering workflows: refactoring, anti-reinvention, multi-agent handoffs, and developer preferences. |
| **[CLI Reference](cli-reference.md)** | Complete syntax, flags, exit codes, and output contracts for the 10 core commands and memory operations. |
| **[Agent Integration](agent-integration.md)** | MCP setup for Claude Desktop, Cursor, Antigravity, and `aw-first` agent routing policy. |
| **[Concepts](concepts.md)** | Architectural concepts: symbols, call edges, confidence tiers, staleness detection, and static analysis limits. |

---

## Evaluation & Architecture

| Document | Description |
|---|---|
| **[Evaluation](EVALUATION.md)** | Empirical measurements: recall benchmarks (98.6% vs 81.9%), token savings (-27.3%), and test methodology. |
| **[Pointer Benchmark](../bench/README.md)** | Reproducible regression benchmark suite for static code navigation pointers. |
| **[Technical Specification](designs/2026-10-31_agent-weave-rust-code-index.md)** | Canonical Rust architecture specification, schema design, and AST resolution rules. |
| **[Memory Architecture](designs/2026-11-05_agent-weave-memory-retrieval.md)** | Design of the zero-token project and global memory subsystem (`aw mem`). |

---

## Policies & Releases

- **[Agent Skill (`skills/aw-first/SKILL.md`)](../skills/aw-first/SKILL.md)**: Agent-facing prompt and routing instructions.
- **[Changelog](../CHANGELOG.md)**: Release history, schema migrations, and breaking changes.
- **[Security Policy](../SECURITY.md)**: Vulnerability disclosure, sandbox confinement, and privacy model.
- **[Contributing](../CONTRIBUTING.md)**: Development setup, MSRV 1.90 requirements, and verification gates.
- **[Code of Conduct](../CODE_OF_CONDUCT.md)**: Community standards and participation guidelines.
