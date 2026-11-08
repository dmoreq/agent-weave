---
name: aw-first
description: >-
  Routing policy for code agents with the aw code index available. Query aw
  (CLI or MCP agent-weave) first for callers, callees, impact, call paths,
  symbol surroundings, imports, helper search, duplicate detection, and memory.
---

# aw-first Routing Policy

`aw` indexes supported source files' symbols and static relationships into `.aw/index.db`.
Results describe the indexed, resolved subset, not every runtime relationship.
See [Evaluation](../../docs/EVALUATION.md) for measurement scope and limitations,
and [Agent integration](../../docs/agent-integration.md) for host setup.

## The Three Pillars of Agent Navigation & Hygiene

All coding agents on this system adhere to three mandatory lifecycle invariants:

### Pillar 1: Graph Navigation First (Pre-Read)
For structural code questions, **query `aw` first** before dumping entire source files or running codebase-wide greps. Read the pointed-to source lines before making behavioral claims or edits; the index is not a substitute for source evidence.

| Question / Intent | Command / Tool | Purpose & Usage |
|---|---|---|
| Index currency | `aw build` | Rebuild after source edits before trusting pointers (~0.02s incremental rebuild). |
| Who calls this symbol | `aw callers <name>` | Pointer list `path:line qualname`; add `--depth N` for the transitive reverse call closure (tagged `[depth=N]`). |
| What does this call (transitive) | `aw callees <name> --depth N` | Outgoing call closure over resolved static edges. |
| Does A reach B, via which call sites | `aw path <from> <to>` | Shortest route over resolved call edges. Swap arguments for reverse query. |
| Symbol surroundings | `aw context <name>` | Definition range + signature + direct callers + direct callees in one call. |
| Which files depend on a file | `aw impact <file>` | Files that call into target; add `--edges calls,imports,inherits --depth N` for extended closure. |
| What does this file import | `aw imports <file>` | Outbound resolved imports for an indexed file. |
| Symbols & definitions | `aw symbols <file> [--outline]` | Deterministic definition inventory and signatures of one indexed file or directory. |

### Pillar 2: Anti-Reinvention (Pre-Write Search & Post-Edit Reuse)
- **Pre-Write Helper Discovery**: Run `aw search '<terms>'` before authoring new helpers or utility functions. Prioritize existing canonical functions with high caller counts (`[callers=N]`) to avoid reinventing existing logic.
- **Post-Edit Duplicate Audit**: Run `aw reuse [paths...]` after creating or editing functions. `aw` runs four calibrated detectors (callee footprint $C_{dup} \ge 80\%$, interface token similarity, client bypass, 1-caller wrappers) to ensure no redundant copies were introduced.

### Pillar 3: Project Memory Lifecycle (Pre-Task Recall & Post-Fix Capture)
- **Pre-Task Recall**: Run `aw mem query` (or MCP `mem_query({})`) at the start of a session or when exploring an unfamiliar area. Without arguments, it seamlessly lists current project and global decisions, gotchas, and conventions.
- **Post-Fix & Architecture Capture**: When you resolve a non-obvious bug or establish an architectural pattern, persist it before ending your turn:
  `aw mem write <key> --kind gotcha|decision --body <prose> [--tags ...] [--refs ...] [--global]`
  Use `--global` for cross-project preferences or machine-wide conventions.

---

## Trust Contract

Treat resolved navigation rows as candidates, even when the index is current and the result is not capped or ambiguous. Static resolution can produce both false positives and missing relationships.

Avoid repeating a complete lookup merely for navigation. Verify individual relationships against source when the answer requires confidence in them.

Verify with source or grep when:
- the output is explicitly marked as "candidates only" (`aw reuse`, `aw unresolved`),
- the result is capped (`showing N of M`), stale, ambiguous, or empty,
- the question asks for a field the output does not carry,
- you need behavioral, safety, or review evidence, or
- a row conflicts with current source.

### Commands returning resolved navigation rows
`callers`, `callees`, `context`, `imports`, `impact`, `path`, `symbols`, `search`

### Commands whose output is candidates
`reuse`, `unresolved`, `mem query`, `mem get`

This contract covers navigation answers only. Source reads remain required for behavioral claims, edits, safety, and review.

---

## Secondary & Maintenance Tools

Reach for these when the routed commands do not answer the question:

| Question | Command |
|---|---|
| Directory definitions | `aw symbols <dir> --outline` — definition inventory across directory prefix |
| Where did resolution fail | `aw unresolved` — summaries; `aw unresolved --names` lists sites; `[internal]` indicates resolver gaps |
| Repository totals | `aw stats [--path <dir>]` — build-time summary, not a freshness check |
| Telemetry & Quality | `aw usage [--global] [--quality]` — query counts, latency distributions, token savings, and duplicate warnings |

---

## Project Memory Operations

`aw` carries agent-authored project notes in `.aw/memory.db` (local) and developer profile/cross-project preferences in `~/.aw/memory.db` (global) — decisions, gotchas, facts, and conventions that survive rebuilds:

- **Write** (`aw mem write <key> --body <prose> --kind decision|gotcha|fact|task [--tags ...] [--refs ...] [--global]`):
  Record when a decision is settled after real investigation, a gotcha cost debugging time, or a task ends with conclusions not yet in committed documentation. Use `--global` to persist cross-project conventions or developer profiles into `~/.aw/memory.db`.
- **Query** (`aw mem query [terms]`; without terms lists by recency; add `--global` to search only global store):
  Run at session start and before re-investigating an area worked on previously. Without `--global`, queries seamlessly search **both** local and global stores: global rows carry `[global]`, and local notes take precedence (shadowing colliding global keys).
- **Maintain**:
  Rewrite under the same key to update a note revision; `--supersedes <key>` records a cross-key replacement; `--scope session` (default TTL 7 days, `--ttl` overrides) keeps transient scratch out of durable memory. Use `aw mem get <key> [--global]`, `aw mem delete <key> [--global]`, `aw mem export [--global]` (JSONL backup), and `aw mem gc [--global]` (purge expired session notes and >30-day superseded revisions).

---

## Handling Contract Behaviors

1. **Exit 2 (Ambiguous Name)**: A bare name matched several symbols. `aw` prints `path:line qualname` candidates. Re-query with the fully qualified name (`Class.method` or `module.function`) — never guess. Grepping the bare name instead silently mixes all of them.
2. **`showing N of M` on the last line**: When N < M, the result was capped. List commands default to 50; `context` defaults to 10 per section (max 50). Re-run with `--limit <M>` where supported (max 10000). If the cap still excludes results, narrow supported path filters or inspect source.
3. **`aw path` outcomes**: Endpoints accept an exact bare name or full qualname; a unique dotted suffix such as `Class.method` also resolves. `symbol '<name>' not in index` indicates the input form was wrong or the symbol is absent. Recover with `aw search` or `aw symbols <file>`. Exit 2 names the failing endpoint.
4. **Static-Surface Caveats**: Every `aw` result covers the indexed, resolved subset only. Successful cases do not establish complete repository coverage; a negative is not proof.

---

## Staleness Discipline

After you edit code, run `aw build` before further navigation queries.
If a query prints:
`aw: K of N result files changed since build — run aw build`
rebuild before trusting those pointers. Staleness checks cover displayed result files, not every file that could change the answer.

---

## When aw is NOT the Tool

- Semantic questions ("where is behavior X implemented") — `aw` resolves names and identifiers, not arbitrary behavioral natural language descriptions. Use search or grep.
- Dynamic dispatch, untyped attribute calls, external library calls — static resolution can leave these unresolved. Inspect source or grep for dynamic behaviors.
- Code in another repository — the index covers the current local repository only.

---

## Context Hygiene

- Pointers lead with `path:line`; open only the lines you need (`sed -n` or line range reads), not whole files.
- `aw symbols <file> --outline` replaces reading full files when you only need the definition inventory and signatures.
