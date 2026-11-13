# CLI Reference

Comprehensive reference for the `engram` (`eg`) command-line interface.

All commands operate as consultative inspection tools: zero runtime network calls, zero background daemons, zero file watchers, and zero modifications outside `.engram/` (or `~/.engram/` for global memory).

---

## Global Options

Global options can be specified before or after any subcommand:

```bash
eg [GLOBAL_OPTIONS] <SUBCOMMAND> [OPTIONS]
```

| Option | Description |
|---|---|
| `--root <PATH>` | Select repository root (defaults to nearest `.git` ancestor of current directory). |
| `--no-journal` | Disable invocation logging in `.engram/usage.log` for this execution. |
| `-h`, `--help` | Print help information and exit. |
| `-V`, `--version` | Print binary version and exit. |

---

## The 10 Core Commands

### 1. `eg build`
Compile or incrementally update the local SQLite AST index (`.engram/index.db`). Uses content-addressed hashing to skip unchanged files and atomically swaps the database file upon completion.

```bash
eg build [--force] [--max-parse-error-rate <FRACTION>]
```

| Option | Default | Description |
|---|---|---|
| `--force` | `false` | Re-extract all files from scratch (bypasses fast-path cache and parse abort). |
| `--max-parse-error-rate` | `0.05` | Maximum allowable fraction of files with parse errors (0.0 to 1.0) before aborting. |

- **Exit Codes**: `0` on success; `4` if build lock is held; `7` if parse error threshold is exceeded.

---

### 2. `eg callers`
Identify inbound call sites that invoke the specified function, method, or class.

```bash
eg callers <NAME> [--depth <N>] [--limit <N>]
```

| Option / Argument | Default | Description |
|---|---|---|
| `<NAME>` | Required | Symbol name or qualified name (`Class.method` or `module.func`). |
| `--depth <N>` | `1` | Transitive closure depth (1 to 10). Transitive hops are tagged `[depth=N]`. |
| `--limit <N>` | `50` | Maximum results displayed (1 to 10000). |

- **Output Format**: `{path}:{line} {qualname} [depth=N]`

---

### 3. `eg callees`
List outgoing function and method calls made by the specified symbol.

```bash
eg callees <NAME> [--depth <N>] [--limit <N>]
```

| Option / Argument | Default | Description |
|---|---|---|
| `<NAME>` | Required | Symbol name or qualified name. |
| `--depth <N>` | `1` | Transitive depth for outgoing call closure (1 to 10). |
| `--limit <N>` | `50` | Maximum results displayed (1 to 10000). |

- **Output Format**: Points to destination definition ranges over resolved edges.

---

### 4. `eg context`
Single-call consolidated symbol surroundings: definition range, signature, direct callers, and direct callees.

```bash
eg context <NAME> [--limit <N>]
```

| Option / Argument | Default | Description |
|---|---|---|
| `<NAME>` | Required | Symbol name or qualified name. |
| `--limit <N>` | `10` | Maximum callers and callees shown per section (1 to 50). |

- **Output Format**: Structured sections with definition header, direct callers, and direct callees.

---

### 5. `eg path`
Find the shortest directed call route between two symbols over resolved call graph edges.

```bash
eg path <FROM> <TO> [--depth <N>]
```

| Option / Argument | Default | Description |
|---|---|---|
| `<FROM>` | Required | Originating symbol name or qualified name. |
| `<TO>` | Required | Destination symbol name or qualified name. |
| `--depth <N>` | `10` | Maximum hops to search (1 to 10). |

- **Output Format**: Sequence of hop call sites with `path:line` pointers and symbol chain. Swap arguments for reverse reachability.

---

### 6. `eg impact`
Reverse dependency analysis: identify files that depend on or call into a specified target file.

```bash
eg impact <FILE> [--sites] [--depth <N>] [--edges <KINDS>] [--limit <N>]
```

| Option / Argument | Default | Description |
|---|---|---|
| `<FILE>` | Required | Repository-relative path of the indexed file. |
| `--sites` | `false` | Display individual call-site pointers instead of grouping by file. |
| `--depth <N>` | `1` | Reverse traversal depth (1 to 10). |
| `--edges <KINDS>` | `calls` | Comma-separated edge kinds: `calls`, `imports`, `inherits`, or `all`. |
| `--limit <N>` | `50` | Maximum results displayed (1 to 10000). |

---

### 7. `eg imports`
List outbound resolved imports declared by a specific file.

```bash
eg imports <FILE> [--limit <N>]
```

| Option / Argument | Default | Description |
|---|---|---|
| `<FILE>` | Required | Repository-relative path of the indexed file. |
| `--limit <N>` | `50` | Maximum results displayed (1 to 10000). |

---

### 8. `eg search`
Symbol-level discovery using SQLite FTS5 BM25 search across symbol names, qualified names, file paths, and signatures, weighted by inbound caller popularity.

```bash
eg search <TERMS> [--limit <N>]
```

| Option / Argument | Default | Description |
|---|---|---|
| `<TERMS>` | Required | Space-separated search query. camelCase and snake_case are automatically split. |
| `--limit <N>` | `50` | Maximum results displayed (1 to 10000). |

- **Output Format (Concise Interface Cards)**:
  `{path}:{line} {kind} {qualname} {signature} [callers=N]`
- Surfaces canonical functions first to prevent duplicate logic.

---

### 9. `eg symbols`
List indexed definitions in a file or directory ordered by line number.

```bash
eg symbols <FILE_OR_DIR> [--outline] [--limit <N>]
```

| Option / Argument | Default | Description |
|---|---|---|
| `<FILE_OR_DIR>` | Required | Repository-relative file or directory path. |
| `--outline` | `false` | Append one-line signature column (truncated at 160 characters). |
| `--limit <N>` | `50` | Maximum results displayed (1 to 10000). |

---

### 10. `eg reuse`
Real-time anti-reinvention engine. Detects cross-file and intra-file duplicate functions, callee footprint overlap, client bypasses, and wrapper inflation.

```bash
eg reuse [PATHS]... [--limit <N>] [--deep-callee] [--min-confidence <low|provisional|medium|high>] [--explain]
eg reuse --propose "<signature>" [--callees <a,b>] [--explain]
eg reuse --coverage <class|unresolved|tests> [--explain]
```

| Option / Argument | Default | Description |
|---|---|---|
| `[PATHS]...` | Empty (repo) | File, directory, `file:line`, or unique symbol name to inspect. Multiple paths form a union. |
| `--limit <N>` | `10` | Maximum candidate pairs displayed (1 to 100). |
| `--deep-callee` | `false` | Extend D1's callee footprint to second-hop callees (weight `0.5`). |
| `--min-confidence <tier>` | `low` | Minimum confidence shown: `low`, `provisional`, `medium`, or `high`. |
| `--propose <signature>` | — | Pre-finalize proposal matched against indexed symbols; paths are rejected. |
| `--callees <a,b>` | — | Proposed callee names used with `--propose`. |
| `--coverage <kinds>` | — | Opt-in coverage detectors: `class`, `unresolved`, `tests`. |
| `--show-suppressed` | `false` | Include allowlisted candidates and report the suppressed count. |
| `--judge` | `false` | Ask a local `eg-judge` binary to arbitrate gray-zone candidates. |
| `--explain` | `false` | Append `detector=...; reasons=[...]` to each candidate line. |

Path targets are validated before the scan: absolute paths, `..`, backslashes, and symlinks exit `2`; missing or unindexed targets exit `1`. A `file:line` target selects the innermost production function or method containing that line. Directory targets match at a path-component boundary. When a target or canonical file changed since the build, the candidate is downgraded and the output carries a reuse-specific freshness header.

Proposal mode never creates a file. It matches the proposed name tokens and callees against resolved index data and returns only existing canonical symbols.

Allowlist notes use the key `reuse-allow/<target>-><canonical>` in the local memory store. High-confidence pairs are never suppressed. A note with `--refs` is ignored after any referenced file changes. `--show-suppressed` keeps the candidate visible and adds `N suppressed` to the summary.

Judge mode is best-effort. A missing binary, model, timeout, malformed output, or unknown verdict leaves the static result unchanged and prints one `eg: judge unavailable:` stderr note. Set `EG_JUDGE_BIN` and `EG_JUDGE_MODEL` to pin the executable and checkpoint directory; the child runs with a cleared environment and a 15 s deadline.

#### Detectors
- **D1: Callee Footprint Overlap ($C_{dup} \ge 80\%$)**: Identifies functions sharing $\ge 80\%$ of their called functions (symmetric containment with size guard). With `--deep-callee`, the footprint includes second-hop callees, catching wrappers that forward through an intermediate helper.
- **D2: Interface Token Overlap ($\ge 60\%$)**: Flags functions with heavily overlapping signatures and parameter types.
- **D3: Raw Client Bypass**: Warns when callers invoke raw primitives rather than existing canonical wrappers.
- **D4: 1-Caller Wrappers**: Surfaces redundant single-use forwarding functions.
- **D5: Class Shape Overlap**: Opt-in `--coverage class`; same class name and method overlap across files.
- **D1L: Unresolved Call Coverage**: Opt-in `--coverage unresolved`; an unresolved bare-name call matches an indexed production symbol.
- **DT: Test Helper Overlap**: Opt-in `--coverage tests`; a test helper shares name tokens with a production symbol.

---

## Project & Global Memory (`eg mem`)

Zero-token persistent memory store for architectural decisions, gotchas, session handoffs, and developer preferences.

### `eg mem write`
Persist an agent-authored note under a stable key identity.

```bash
eg mem write <KEY> --body <TEXT> --kind <KIND> [OPTIONS]
```

| Option / Argument | Default | Description |
|---|---|---|
| `<KEY>` | Required | Stable note identifier ($\le 200$ chars, alphanumeric/dashes). |
| `--body <TEXT>` | Required | Note body prose ($\le 8192$ bytes, never verbatim source dumps). |
| `--kind <KIND>` | Required | Note category: `decision`, `gotcha`, `fact`, `task`, or `procedure`. |
| `--tags <TOKENS>` | None | Whitespace-separated tags ($\le 16$ tags, $\le 32$ chars each). |
| `--refs <NAMES>` | None | Whitespace-separated AST qualnames or paths ($\le 16$ refs, $\le 256$ chars each). |
| `--scope <SCOPE>` | `durable` | `durable` (never expires) or `session` (expires after TTL). |
| `--ttl <SECS>` | `604800` (7 days)| Expiration lifetime in seconds (only valid with `--scope session`). |
| `--supersedes <KEY>`| None | Mark the specified key as superseded by this note in one atomic transaction. |
| `--touch` | `false` | Refresh `created_at` (last asserted) on the active revision when the content hash matches; no new revision. |
| `--confirm` | `false` | Required for `reuse-allow/...` writes over MCP; keeps allowlist creation an explicit act. |
| `--global` | `false` | Persist to global store (`~/.engram/memory.db`) instead of repository (`.engram/memory.db`). |

When the new note is at least 80% Jaccard-similar to an existing current note under another key, `mem write` appends an advisory `eg: mem hint:` line suggesting `--supersedes`. The write still succeeds; the hint is never a hard failure.

---

### `eg mem query`
Retrieve and rank memory notes using SQLite FTS5 BM25, reference token overlap, and recency decay.

```bash
eg mem query [TERMS] [--all] [--kind <KIND>] [--scope <SCOPE>] [--limit <N>] [--include-superseded] [--global|--local] [--ref <REFERENCE>] [--since <TIME>] [--until <TIME>] [--explain] [--legacy]
```

| Option | Default | Description |
|---|---|---|
| `[TERMS]` | Optional | Search query terms (mutually exclusive with `--all`; empty/whitespace defaults to `--all`). |
| `--all` | `false` | List all active notes ordered by recency. |
| `--kind <KIND>` | None | Filter by note kind (`decision`, `gotcha`, `fact`, `task`, `procedure`). |
| `--scope <SCOPE>` | None | Filter by scope (`durable` or `session`). |
| `--limit <N>` | `10` | Maximum results displayed (1 to 100). |
| `--include-superseded`| `false` | When combined with `--all`, includes superseded note revisions. |
| `--global` | `false` | Restrict search exclusively to global store (`~/.engram/memory.db`). |
| `--local` | `false` | Restrict search exclusively to the project store; mutually exclusive with `--global`. |
| `--ref <REFERENCE>` | None | Filter notes referencing target path or symbol. |
| `--since <TIME>` | None | Inclusive lower bound on `created_at` (last assertion). |
| `--until <TIME>` | None | Exclusive upper bound on `created_at`. |
| `--explain` | `false` | Add one causal `why:` line per shown ranked note; requires terms. |
| `--legacy` | `false` | Force the legacy ASCII index and tokens (diagnostic). |

Time bounds accept `YYYY-MM-DD`, `YYYY-MM-DDTHH:MM:SSZ`, or `@<unix-seconds>`. Bounds are UTC and half-open: `[since, until)`. Invalid dates, timezone offsets, fractional seconds, bare integers, and `since > until` are usage errors (exit 2).

- **Layered Precedence**: Default queries seamlessly search **both** local repository notes and global preferences. Local notes shadow colliding global keys. Global notes are marked with `[global]`. `--local` restricts to the project store and never falls back to global.
- **Unicode Retrieval**: Notes are indexed in both a legacy ASCII FTS table and a Unicode FTS table using `remove_diacritics 2`. `bo nho` finds `Bộ nhớ`; a bare consonant query such as `nh` does not. Reads use the Unicode table only after a clean reconcile; otherwise they fall back to the legacy table.
- **Explain**: `--explain` emits labels such as `lexical match`, `ref src/memory/rank.rs`, `graph depth 2`, and `age 3d`; it never prints bm25 values, normalized scores, weights, or note bodies.
- **Graph Expansion**: When `.engram/index.db` is readable, query terms seed a bounded traversal of resolved `calls` edges (depth $\le 3$, decay $0.5^{depth}$), so notes attached to downstream callees still surface. A missing or unreadable index silently falls back to plain FTS5 ranking.
- **Citation-Lock Verification**: Automatically verifies `--refs` anchors against `.engram/index.db`. Broken targets missing from the index are badged with `[stale-ref] [broken-ref]`, while targets modified after note creation are badged with `[stale-ref] [modified-ref]`. Candidates are also retrieved directly by matching code-anchored reference tokens.

---

### Additional Memory Operations

| Subcommand | Syntax | Description |
|---|---|---|
| `touch` | `eg mem touch <KEY> [--global]` | Refresh `anchored_at` (last verified) on the active revision without creating a new revision. |
| `get` | `eg mem get <KEY> [--global]` | Fetch current revision and total revision count for an exact key. |
| `delete` | `eg mem delete <KEY> [--global]` | Hard-delete all revisions and FTS rows for a key. |
| `export` | `eg mem export [--format jsonl\|markdown] [--limit <N>] [--global] [--since <TIME>] [--until <TIME>] [--scan]` | Export active notes as JSONL (default) or fenced Markdown to stdout. `--scan` reports suspected secrets on stderr without changing stdout or the exit code. |
| `import` | `eg mem import <PATH> [--format jsonl\|markdown] [--force] [--global]` | Restore a JSONL/Markdown export (use `-` for stdin). Conflicting keys are skipped unless `--force` overwrites; imported rows carry `[imported]`. MCP import is confined to the project store. |
| `gc` | `eg mem gc [--global]` | Purge expired session notes and superseded revisions older than 30 days. |
| `reindex` | `eg mem reindex [--global]` | Rebuild the derived Unicode FTS index; notes are primary data and are untouched. |

---

## Diagnostics and Maintenance

### `eg doctor`
Verify index integrity, versions, parse errors, and changed file ratios.

```bash
eg doctor [--invalidate]
```

- `--invalidate`: Resets derived `.engram/` data (removes `.engram/index.db`, journals, and orphan locks). Primary memory data (`.engram/memory.db`) is safely preserved.

### `eg stats`
Display build-time counts for files, symbols, edges, parse errors, and database bytes.

```bash
eg stats [--path <PATH>]
```

### `eg unresolved`
Analyze where static resolution could not resolve call targets.

```bash
eg unresolved [--names] [--path <PATH>] [--limit <N>]
```

- `--names`: Lists individual unresolved call sites with file:line pointers instead of aggregated per-file counts. `[internal]` indicates an unresolved call to a symbol that exists in the index.

### `eg usage`
Display local performance telemetry, query distribution, latency percentiles, and token savings.

```bash
eg usage [--global] [--quality]
```

- `--global`: Aggregates telemetry across all repositories and worktrees on the machine (`~/.engram/usage.log`), listing live repositories and recent errors for triage. Entries without a live repository identity — deleted repositories, legacy lines, calls that resolved no repository — are skipped by the read and purged from the log on the next `eg dashboard` or `eg doctor` run.
- `--quality`: Appends breakdown of empty results (`empty_ok` vs `empty_error`), capped queries, and p50/p95 latency percentiles.

### `eg audit`
Read-only health and effectiveness report over telemetry, index snapshots, and memory refs. Unlike `eg usage`, it applies the published windows, baselines, and thresholds in [health](health.md), and never prunes or writes.

```bash
eg audit [--json] [--global] [--days N] [--baseline-days N]
         [--all-checks] [--reveal-paths] [--fail-on warn|critical]
```

- `--json`: One JSON document on stdout; diagnostics are fixed codes on stderr.
- `--global`: Use `~/.engram/usage.log` across live repositories. Local scope is the default and conflicts with `--root`.
- `--days N` / `--baseline-days N`: Evaluation and preceding baseline windows (defaults 7 and 7; `--days` 1–90, `--baseline-days` 3–90).
- `--all-checks`: Human output includes passing checks. Journal arguments and absolute repository paths are never printed.
- `--reveal-paths`: Human output replaces `repo-<hash16>` aliases with paths; rejected with `--json`.
- `--fail-on`: Exit 1 when health reaches `warn` or `critical`. A normal report exits 0 even when warnings exist; missing telemetry exits 1.

The command is CLI-only, like `doctor`; it is not an MCP tool and does not appear in mandatory agent routing. See [health](health.md) for the check registry, baseline rules, redaction, and daily automation contract.

### Dashboard Launcher (`eg-dashboard`)
Convenience launcher for the native dashboard: it resolves the installed `eg`
binary (falling back to `cargo run` in a source checkout) and executes
`eg dashboard --global "$@"`.

```bash
eg-dashboard [dashboard flags]
```

- All flags pass through to `eg dashboard`: `--port <PORT>`, `--host <HOST>`, `--open`, `--local`.
- The launcher adds no binary: `eg dashboard` remains the single native implementation.

### Native Dashboard (`eg dashboard`)
Serve the interactive 3-tab web dashboard (Telemetry, Anti-Reinvention, Agent Memory) with real-time SSE streaming. Built into the native Rust binary with zero Python dependencies. By default, telemetry is aggregated globally across all repositories (`~/.engram/`).

```bash
eg dashboard [--port 7890] [--host 127.0.0.1] [--open] [--local]
```

- `--port <PORT>`: Port to bind the local dashboard web server (default: 7890).
- `--host <HOST>`: Loopback interface to bind (default: 127.0.0.1).
- `--open`: Automatically opens the dashboard in your default browser.
- `--local`: Scope telemetry exclusively to the current repository (`.engram/usage.log`) instead of global.

Entries without a live repository identity — deleted repositories, legacy lines, calls that resolved no repository — disappear from the repository tables, the active-repo count, the Recent Queries feed, and the live SSE stream, and are purged from the log at startup, matching `eg usage --global`.

### Live Query Monitor (`eg monitor`)
Stream real-time query events directly in your terminal with colored ANSI output, agent attribution tags, and zero dependencies. Defaults to global telemetry across all repositories with an initial backlog of recent events.

```bash
eg monitor [--repo <NAME>] [--errors-only] [--local]
```

- `--repo <NAME>`: Restrict streaming query output to a specific repository.
- `--errors-only`: Filter for critical errors (exit code >= 3).
- `--local`: Scope streaming exclusively to the current repository (`.engram/usage.log`).

---

## Model Context Protocol (`eg mcp`)

Serve `eg` as an MCP stdio server over newline-delimited JSON-RPC 2.0.

```bash
eg mcp [--all-tools]
```

- **Default (Core Surface)**: Exposes 13 high-ROI tools: `build`, `callers`, `callees`, `path`, `context`, `imports`, `impact`, `symbols`, `reuse`, `search`, `usage`, `mem_write`, and `mem_query`.
- `--all-tools`: Exposes all 17 tools, adding the 4 extended memory management tools: `mem_get`, `mem_delete`, `mem_gc`, and `mem_export`.

---

## Exit Codes

| Code | Status | Meaning | Recovery Action |
|---|---|---|---|
| `0` | Success | Command completed successfully. | Inspect results or continue workflow. |
| `1` | Empty / Missing | No results found, target missing, or index absent. | Verify name/spelling, run `eg build`, or write note. |
| `2` | Usage / Ambiguous | Invalid arguments or bare name matched multiple symbols. | Correct arguments or copy qualified name from candidate list. |
| `3` | Corrupt Database | SQLite database corruption detected. | Run `eg build` (for index) or restore memory from JSONL backup. |
| `4` | Lock Contention | Build lock held by another process or busy timeout. | Wait for concurrent operation to finish and retry. |
| `5` | Schema Skew | Index or memory database version mismatch. | Run `eg build` to recreate index; memory provides export path. |
| `6` | Internal Error | Unexpected I/O error or internal failure. | Inspect stderr diagnostic. |
| `7` | Parse Abort | File parse failure rate exceeded threshold. | Audit syntax or increase `--max-parse-error-rate`. |
