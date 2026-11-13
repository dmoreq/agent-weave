# Developer & Agent Usage Guide

This guide details practical workflows for using `engram` (`eg`) during software engineering tasks. It covers code navigation, anti-reinvention defense, multi-agent handoffs, and developer preferences.

---

## Workflow Scenarios

### Scenario 1: Refactoring a Function

When modifying a function signature or behavior, you must verify all call sites and understand potential blast radius before making edits.

#### 1. Inspect Immediate Surroundings
Use `eg context` to inspect the target definition, signatures, direct callers, and outgoing calls in a single sub-2ms query:

```bash
eg context run_build
```

#### 2. Trace Inbound Callers
Examine incoming call sites across the codebase:

```bash
# Direct callers (depth 1)
eg callers run_build

# Transitive callers across multiple hops
eg callers run_build --depth 2 --limit 100
```

Each returned row provides exact `path:line qualname [depth=N]` pointers. Inspect the specified line in your editor; do not waste tokens reading entire files.

#### 3. Trace Execution Routes
Verify how execution flows between two distant components:

```bash
eg path run_build fnv1a64
```

`eg path` returns the shortest chain of resolved call sites from the source to destination. To query the reverse reachability, swap the arguments:

```bash
eg path fnv1a64 run_build
```

#### 4. Audit Downstream Blast Radius
Identify every file that depends on the module you plan to modify:

```bash
# Files that call into src/build.rs
eg impact src/build.rs

# Detailed call sites across calls, imports, and inheritance
eg impact src/build.rs --sites --edges all --depth 2
```

---

### Scenario 2: Preventing Code Bloat (The Anti-Reinvention Workflow)

AI coding agents frequently recreate utility functions that already exist in the codebase. Grounded in [RepoReuse](https://arxiv.org/abs/2609.35357) (*Ma et al., 2026*), `eg` enforces a two-moment anti-reinvention loop to counter the **Exploration Gap** (where agents fail to discover existing helpers) and resolve the **Historical Memory Paradox** (where dumping full source code triggers copy-paste duplication up to 70.7%–94.7%).

#### 1. Pre-Write Discovery
Before authoring a new helper (e.g., reading a bounded file or computing a hash), search the index:

```bash
eg search "read bounded file"
```

`eg search` returns **Concise Interface Cards** (signatures, types, and caller counts) ranked by caller frequency (`BM25 + log(1 + callers)`):

```text
src/walk.rs:42 fn read_bounded(path: &Path, max_bytes: usize) -> Result<Vec<u8>> [callers=14]
```

The `[callers=14]` metric immediately identifies `read_bounded` as the established, canonical helper. Supplying interface cards closes the exploration gap without context window bloat.

#### 2. Post-Edit Defense
After modifying or adding code, rebuild the index:

```bash
eg build
```

If duplicate helpers or bypasses are suspected, run `eg reuse`:

```bash
eg reuse
# Or scope to specific paths:
eg reuse src/walk.rs src/parse.rs
# Scope to the symbol containing one line:
eg reuse src/walk.rs:42
# Extend to second-hop wrappers:
eg reuse --deep-callee
# Propose a signature before writing it:
eg reuse --propose "def parse_token(value)" --callees normalize
# Opt-in coverage detectors:
eg reuse --coverage class,unresolved,tests
# Show detector and reason codes:
eg reuse --explain
```

Every candidate prints `confidence=low|provisional|medium|high`. A target or canonical file changed since the build downgrades the candidate and adds a freshness warning; an unindexed new file in an empty scope prints `freshness unknown (new-file)`. Use `--limit` (1–100) and the summary counters to see how many candidates were omitted or filtered by `--min-confidence`.

For an intentional exception, write a local memory note keyed `reuse-allow/<target>-><canonical>` and add `--refs` for the source files. High-confidence pairs cannot be suppressed; `--show-suppressed` keeps the candidate visible and reports the count. `--judge` is optional and degrades to static output when `eg-judge` or its model is unavailable.

`eg reuse` executes four structural detectors across files and within the same file:
- **D1: Callee Footprint Overlap ($C_{dup} \ge 80\%$)**: Flags functions that invoke virtually identical sets of subroutines (symmetric containment with size ratio guard). Add `--deep-callee` to extend the footprint to second-hop callees (weight `0.5`, threshold 0.85), catching wrappers that forward through an intermediate helper. Closures wider than 512 weighted callees are skipped as hubs, and deep mode evaluates at most 30,000 candidate pairs per invocation, keeping the opt-in scan bounded.
- **D2: Interface Token Overlap ($\ge 60\%$)**: Surfaces functions with overlapping parameter names, types, and return signatures.
- **D3: Raw Client Bypass**: Detects callers invoking raw primitives directly when a high-level wrapper exists.
- **D4: 1-Caller Wrappers**: Highlights trivial single-caller forwarders that inflate codebase complexity.

---

### Scenario 3: Multi-Agent Session Handoff & Project Decisions

Persistent project knowledge allows multiple agents or future sessions to resume work without re-investigating settled architectural decisions.

#### 1. Record a Settled Decision
When an architectural decision or technical gotcha is established, persist it with `eg mem write`:

```bash
eg mem write resolver-confidence-tiers --kind decision \
  --body "The resolver keeps confidence tiers; AMBIGUOUS edges keep dst_id NULL." \
  --refs src/extract/resolve.rs
```

#### 2. Code-Anchored References & Stale Alerts
Grounded in the citation-lock principles of [Agent Zero Memory](https://arxiv.org/abs/2608.29606), the `--refs` flag anchors the note to specific AST symbols or file paths as an immutable provenance pointer. If a subsequent commit alters or moves the referenced symbol, `eg mem query` and `eg context` automatically flag the note with a `[stale-ref]` alert, preventing stale architectural assumptions.

#### 2b. Duplicate Guard & Graph Expansion
Before writing, `eg mem write` verifies note novelty via Jaccard token similarity against existing notes. At 80% or more similarity, it appends an advisory hint naming the existing key and suggests `--supersedes` to prevent memory bloat while preserving execution flow.

Operationalizing [Zero-Mem](https://arxiv.org/abs/2607.29377), `eg mem query` consumes 0 LLM tokens. When `.engram/index.db` exists, it follows resolved `calls` edges outward from the query's symbols (depth up to 3, decay `0.5^depth`). Notes anchored to downstream callees surface automatically without requiring exact lexical overlap. If the index is missing, the query safely falls back to plain FTS5 ranking.

#### 2c. Time-Bounded Recall, Explanations, and Scope
Bound a handoff or regression window with half-open UTC bounds. Dates accept `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM:SSZ`; `@<unix-seconds>` is available for scripts:

```bash
eg mem query --all --since 2026-11-11 --until 2026-11-12
eg mem export --since @1794441600 > handoff.jsonl
```

Add `--explain` to a ranked query to see why each note matched. Labels are causal and bounded: `lexical match`, `ref src/memory/rank.rs`, `graph depth 2`, and `age 3d`; scores and note bodies are never printed:

```bash
eg mem query resolver --explain
```

Use `--local` when only repository notes matter; it never falls back to the global store. Unicode retrieval matches `bo nho` to `Bộ nhớ` through the derived Unicode index, while `--legacy` forces the old ASCII index for diagnosis:

```bash
eg mem query deploy --local
eg mem query "bo nho"
eg mem query "bo nho" --legacy
```

`eg mem reindex` rebuilds the derived Unicode index without touching notes. `eg mem export --scan` reports suspected secrets on stderr and still exits 0; stdout stays valid JSONL for restore.

#### 3. Record Multi-Agent Task Handoffs
When wrapping up a session or passing tasks between agents, write a scoped task note:

```bash
eg mem write handoff-doc-standardization --kind task --scope session --ttl 86400 \
  --body "Standardized README.md and getting-started.md. Next agent must review cli-reference.md." \
  --refs docs/cli-reference.md
```

Session notes carry a time-to-live (`--ttl`, default 7 days) and automatically expire, keeping durable memory clean.

#### 4. Resume Context in Next Session
At the start of a new session, query existing project knowledge:

```bash
eg mem query handoff
eg mem query "resolver tiers"
```

---

### Scenario 4: Developer Profile & Cross-Project Conventions

Persist developer preferences, style constraints, and cross-project rules once, without polluting repository-specific files.

#### 1. Write Global Preferences
Add the `--global` flag to store notes in `~/.engram/memory.db`:

```bash
eg mem write dev-profile --kind decision --global \
  --body "Developer preferences: Rust 1.90+, ASD-STE100 technical writing tone, zero speculative abstractions."
```

#### 2. Layered Retrieval
Querying memory searches **both** local repository notes (`.engram/memory.db`) and global preferences (`~/.engram/memory.db`):

```bash
eg mem query profile
```

- **Precedence & Shadowing**: Local notes take precedence over global notes. A local note with the same key shadows the global note.
- **Visual Badging**: Global notes are explicitly marked with `[global]` in query results.
- **Restricted Search**: To search only the global store, pass `eg mem query --global <terms>`.

---

### Scenario 5: Git-Versioned Portable Memory

Markdown export turns durable notes into reviewable files, and import restores them after a fresh clone or a machine reset.

```bash
eg mem export --format markdown > docs/memory.md
git add docs/memory.md && git commit -m "docs: capture project memory"
```

Restore the notes into `.engram/memory.db`:

```bash
eg mem import docs/memory.md
```

- **Reject-on-conflict**: If a key already exists locally with different content, import skips it and exits 2; pass `--force` to open a new revision. Identical content is a no-op.
- **Provenance**: Imported rows render with `[imported]` and are untrusted data, never instructions.
- **Disaster recovery**: JSONL streams pipe directly, with no intermediate file:
  `eg mem export --format jsonl | eg mem import - --format jsonl`.

---

## Output Interpretation and Edge Cases

| Signal / Result | Meaning | Recommended Action |
|---|---|---|
| **Exit code 2 (Ambiguous Name)** | Multiple symbols match bare name | Copy the full qualified name (e.g., `Class.method`) from stdout candidates and re-run. |
| **Exit code 1 (No Results / Missing Target)** | Symbol or file not found in index | Verify spelling, path, file extension, or run `eg build`. |
| **`showing N of M`** | Output was truncated at limit | Pass `--limit <N>` (up to 10000) or narrow with `--path`. |
| **Changed-Result Warning** | Displayed files changed since build | Run `eg build` to update the index, then repeat query. |
| **`[ambiguous]` tag** | Unresolved dynamic or overloaded call | Inspect source to confirm exact runtime call target. |
| **`[stale-ref]` tag** | Referenced AST anchor was modified | Audit the note body against updated source code. |

---

## Fair Workflow Comparison

When comparing agent performance with and without `eg`, ensure fair measurement:
1. **No-eg Baseline**: Use `grep`, `find`, and direct whole-file reads. Measure tokens consumed and tool iterations.
2. **eg-Assisted**: Use `eg search`, `eg context`, `eg callers`, and targeted line reads (`sed -n` or line ranges).
3. **Telemetry Tracking**: Run `eg usage --quality` to observe p50/p95 query latency, total tool calls, and estimated tokens saved.

---

## Local & Global Telemetry Controls

`eg` records anonymous invocation telemetry dual-written locally to `<repo>/.engram/usage.log` and globally across all repositories to `~/.engram/usage.log`.

```bash
# View local repository telemetry
eg usage --quality

# View machine-wide telemetry across all repositories and worktrees
eg usage --global

# Read-only health and effectiveness report (local or machine-wide)
eg audit
eg audit --global --json

# Render and open interactive multi-repo HTML dashboard
eg-dashboard --open

# Disable journaling for a single command
eg --no-journal callers run_build

# Disable journaling globally via environment
export ENGRAM_NO_JOURNAL=1
```

Memory stores gain an additive `anchored_at` column (memory schema v2) on the next write; `eg mem touch` refreshes that anchor and preserves the note's creation date. Telemetry logs are strictly local and automatically rotate at 5 MiB. Repository rows are a live inventory: an entry without a live repository identity — a deleted repository, a legacy line, a call that resolved no repository — is skipped by every read and purged from the log on the next `eg dashboard` or `eg doctor` run, so it disappears from `eg usage --global`, `eg monitor`, and the dashboard, totals included. There is no unattributed bucket.

The dashboard and `eg monitor` report one row per repository: calls recorded in a linked git worktree fold into the main repository's row (resolved from the worktree's `.git` pointer file), with a per-worktree breakdown behind an expandable row in the repository table, and `from <main-repo>` provenance in the Recent Queries feed. The global dashboard additionally treats repository roots under the OS temp directory (`/tmp`, `/private/tmp`, `/var/tmp`, `$TMPDIR`) as scratch identities — probe fixtures and throwaway checkouts: they never reach the repository tables, and `eg dashboard` start drops their lines from the global journal.

`eg audit` applies fixed windows, baselines, and thresholds to the retained journal, index, and memory refs. It is read-only: it never prunes, rotates, or writes, and it is excluded from its own telemetry. A run reports health (`status`) separately from coverage (`data_status`), so missing data is never presented as health. See [health](health.md) for the check registry, exit codes, redaction rules, and the daily automation contract.
