# Documentation Index

Welcome to the `engram` (`eg`) technical documentation.

`eg` is a consultative, offline code knowledge index, real-time anti-reinvention
engine, and zero-token persistent memory store for AI coding agents.

---

## User & Agent Guides

| Guide | Description |
|---|---|
| **[Getting Started](getting-started.md)** | Installation, first 5 minutes with the 3 USPs, language coverage, and index reset. |
| **[Usage Guide](usage.md)** | Task-oriented engineering workflows: refactoring, anti-reinvention, multi-agent handoffs, and developer preferences. |
| **[CLI Reference](cli-reference.md)** | Complete syntax, flags, exit codes, and output contracts for the core commands and memory operations. |
| **[Agent Integration](agent-integration.md)** | MCP setup for Claude Desktop, Cursor, Antigravity, and the `engram-first` agent routing policy. |
| **[Concepts](concepts.md)** | Symbols, call edges, confidence tiers, staleness detection, and static analysis limits. |

---

## Architecture

Current, maintained specifications of the shipping system:

| Document | Description |
|---|---|
| **[Code Index](architecture/code-index.md)** | Storage layout, build lifecycle, incremental fast path, query engines, and exit codes. |
| **[Memory Store](architecture/memory-store.md)** | Note schema, revision chains, FTS5 retrieval scoring, reference currency, and confinement. |

---

## Evaluation

| Document | Description |
|---|---|
| **[Evaluation](evaluation.md)** | Empirical measurements: recall benchmarks, token savings, latency, and methodology limits. |
| **[Pointer Benchmark](../bench/README.md)** | Reproducible regression benchmark suite for static code navigation pointers. |
| **[Evidence Vault](baselines)** | Frozen ground-truth fixtures, dated measurement manifests, and review records. Historical measurements, not new verification. |

---

## Policies & Releases

- **[Agent Skill (`skills/engram-first/SKILL.md`)](../skills/engram-first/SKILL.md)**: Agent-facing prompt and routing instructions.
- **[Changelog](../CHANGELOG.md)**: Release history, schema migrations, and breaking changes.
- **[Security Policy](../SECURITY.md)**: Vulnerability disclosure, sandbox confinement, and privacy model.
- **[Contributing](../CONTRIBUTING.md)**: Development setup, MSRV 1.90 requirements, and verification gates.
- **[Code of Conduct](../CODE_OF_CONDUCT.md)**: Community standards and participation guidelines.

---

## Archive

Historical design specifications and review reports are preserved under
[`docs/archive/`](archive) for reference. They describe the system as it was at
their date and can contradict the current implementation; the architecture pages
above are authoritative.
