# CLI reference

Use the [usage guide](usage.md) to choose a workflow. This page describes the command interface defined in [`src/cli.rs`](../src/cli.rs), with behavior from the query and maintenance implementations.

## Find the command for your task

| Task | Commands |
|---|---|
| Create or refresh an index | `build` |
| Inspect a definition and its neighbors | `context`, `callers`, `callees`, `path` |
| Inspect dependencies | `impact`, `imports` |
| Find definitions and symbols | `search`, `symbols` |
| Detect anti-reinvention candidates | `reuse`, `unresolved` |
| Read repository summaries | `stats` |
| Persist and retrieve project notes | `mem write`, `mem query`, `mem get`, `mem delete`, `mem export`, `mem gc` |
| Diagnose or remove local data | `doctor` |
| Inspect recorded usage | `usage` |
| Connect an MCP client | `mcp` |

## Trust contract

A successful query is ground truth for the rows it returns. Re-run or read
source when output is capped, ambiguous, stale, or empty; those are coverage
limits, not routine re-verification. Treat `reuse` and `unresolved`
as candidates.

## Invocation and global options

```text
aw [GLOBAL_OPTIONS] <COMMAND> [ARGUMENTS] [OPTIONS]
```

| Option | Default | Meaning |
|---|---|---|
| `--root <PATH>` | Nearest ancestor of the working directory containing `.git` | Select the repository root; an explicit root must be a directory and need not contain `.git` |
| `--no-journal` | Off | Disable the usage journal for this invocation |
| `-h`, `--help` | Off | Print command help and exit successfully |
| `-V`, `--version` | Off | Print the binary version and exit successfully; top-level option |

`--root` and `--no-journal` work before or after a subcommand. `aw help` and `aw help <COMMAND>` also show help. Every subcommand supports `-h` and `--help`.

Names accept an exact bare name or a full printed qualified name. If no exact match exists, a dotted suffix such as `Class.method` resolves when unique; multiple suffix matches stay ambiguous. Pointers and partial segments are not accepted.

File arguments use root-relative paths with `/`, not paths relative to the invoking subdirectory. Where `--path <PATH>` is offered, it selects an exact file or a directory and descendants, not an arbitrary string prefix.

Omit `--path` for the whole index. Use a nonempty repository-relative scope without `..` components, backslashes, or an absolute prefix.

### Shared limits and switches

- Unless overridden below, `--limit <N>` defaults to **50** and clamps values above **10000** to **10000**. Zero is a usage error.
- `context --limit` defaults to **10** and clamps at **50**, independently for callers and callees.
- `--depth <N>` clamps to **1–10**, including clamping zero to one. Defaults differ by command.
- Boolean switches default to off. Optional filters default to absent.
- Generated-file tags use recognized suffixes or markers in the first five lines, not a general generated-code classifier. They remain in inventories and summaries but do not have a query switch.

## Build and maintain the index

### `build`

```text
aw build [--force] [--max-parse-error-rate <FRACTION>]
```

Build `.aw/index.db` using an atomic replacement. Reuse unchanged extraction data when possible; a fully unchanged build can leave the database untouched.

| Option | Default | Meaning |
|---|---|---|
| `--force` | Off | Re-extract all files and bypass the parse-quality abort |
| `--max-parse-error-rate <FRACTION>` | `0.05` | Abort if the fraction of indexed files with parse errors exceeds this threshold; require a finite value in `[0, 1]` |

Print a build summary with file, symbol, edge, parse-error, unresolved-call, carried-file, and timing fields. An abort does not publish a partial replacement index; `--force` still validates the threshold argument.

### `doctor`

```text
aw doctor [--invalidate]
```

Without options, report database integrity, versions, parse errors, changed indexed files, size, and leftover build files. This report needs an index.

`--invalidate` defaults to off. When set, reset the derived `.aw/` state: the index, journals, locks, and orphan files are removed; no index is required. The project-memory store `.aw/memory.db` is primary data and is **kept** (with its crash-recovery sidecars); export it with `aw mem export` first if you want a copy. When no memory store exists the whole directory is removed, as before. Source files and the executable remain unchanged.

## Inspect definitions and calls

### `context <NAME>`

```text
aw context <NAME> [--limit <N>]
```

Print a definition range, kind, qualified name, signature, direct caller sites, and direct callees. `--limit` defaults to 10 per section, maximum 50.

Resolved callees point to definitions; ambiguous callee entries can point to the originating site and carry `[ambiguous]`. Missing sections do not fail a query when the target exists. This is static context, not a complete runtime call graph. Does not prove: every runtime caller or callee is present.

### `callers <NAME>`

```text
aw callers <NAME> [--limit <N>] [--depth <N>]
```

Print sites that call the symbol, with the enclosing caller's qualified name. `--depth` defaults to 1 for direct callers; larger depths follow the reverse call closure and append `[depth=N]` to each row. Each caller symbol appears once at its shortest depth; a symbol reached by a shorter route is not repeated on a longer route, and its inbound edges are expanded once. The queried symbol is the closure seed and is excluded from transitive rows, so a self-call is visible only with `--depth 1`. `--limit` uses the shared default and cap. Possible ambiguous edges are tagged; they are not proven calls to the target. Does not prove: no other caller exists; dynamic dispatch can hide callers.

### `callees <NAME>`

```text
aw callees <NAME> [--limit <N>] [--depth <N>]
```

Print outgoing callees; resolved rows point to definitions and include callee identity. `--depth` defaults to 1 for direct callees; larger depths follow the forward call closure. `--limit` uses the shared default and cap. Does not prove: the symbol has no other outgoing calls; external and dynamic calls can be missing.

### `path <FROM> <TO>`

```text
aw path <FROM> <TO> [--depth <N>]
```

Find one shortest forward route over resolved call edges. Print a static-analysis caveat, call-site pointers in hop order, and the symbol chain. `--depth` defaults to 10; there is no `--limit` option.

Swap arguments for the reverse question. Equal endpoints or no route within the depth limit return exit 1. Ambiguity identifies the failing endpoint; dynamic dispatch can hide routes. Does not prove: no route exists at runtime, or that every hop is exercised.

## Inspect dependencies and test candidates

### `impact <FILE>`

```text
aw impact <FILE> [--sites] [--depth <N>] [--edges <KINDS>] [--limit <N>]
```

Find files that depend on the indexed target file, not its outbound dependencies. Default rows group by caller file and include `(N sites)`; `--limit` uses the shared default and cap. An empty default (calls-only) result names how many files import or inherit from the target and points at `--edges imports,inherits`.

| Option | Default | Meaning |
|---|---|---|
| `--sites` | Off | Show individual relationship sites instead of grouping by file |
| `--depth <N>` | `1` | Reverse dependency traversal depth |
| `--edges <KINDS>` | `calls` | Comma-separated `calls`, `imports`, `inherits`, or `all`; do not combine `all` with explicit kinds |
| `--limit <N>` | `50` | Maximum displayed matches, cap 10000 |

Unresolved imports and bases are omitted from extended dependency traversal. Dynamic calls can hide dependents even when only calls are selected. Does not prove: runtime impact, required tests, or effects outside indexed edges.

### `imports <FILE>`

```text
aw imports <FILE> [--limit <N>]
```

List outbound resolved import targets for an indexed file. External and unresolved imports are omitted. `--limit` uses the shared default and cap. Does not prove: the file has no other dependencies; external and unresolved imports are omitted.

## Find symbols

### `search <TERMS>`

```text
aw search <TERMS> [--limit <N>]
```

Rank symbols using BM25 token search over names, qualified names, file paths, and signatures, weighted by inbound caller count. Quote multiple words as one argument, for example `aw search 'parse tree'`.

| Option | Default | Meaning |
|---|---|---|
| `--limit <N>` | `50` | Maximum displayed matches, cap 10000 |

Outputs concise interface cards: `{path}:{line} {kind} {qualname} {signature} [callers=N]`.

camelCase and snake_case split into tokens. Normal search uses prefix terms and keeps at most 32 tokens, with a warning when truncated. It does not search bodies, comments, or docstrings. Does not prove: the symbol does not exist elsewhere; bodies, comments, and docstrings are not searched.

### `symbols <FILE_OR_DIRECTORY>`

```text
aw symbols <FILE_OR_DIRECTORY> [--outline] [--limit <N>]
```

List indexed definitions by path and line. `--outline` adds signatures. `--limit` uses the shared default and cap. Accept `.` or `./` as the repository root.

## Detect anti-reinvention candidates and gaps

### `reuse`

```text
aw reuse [PATHS]... [--limit <N>]
```

Scan the repository for anti-reinvention opportunities and code reuse candidates. Detects:
- Exact signature and callee footprint duplicates across files.
- Utility functions with high callee overlap.
- Candidate shared abstractions.

| Option | Default | Meaning |
|---|---|---|
| `[PATHS]...` | Empty (whole repo) | Restrict search scope to specific file or directory prefixes |
| `--limit <N>` | `10` | Maximum candidate pairs displayed, cap 100 |

Does not prove: functions are identical in runtime behavior. Inspect implementation before consolidating.

### `unresolved`

```text
aw unresolved [--names] [--path <PATH>] [--limit <N>]
```

By default, summarize unresolved callees by file and include common unresolved names. `--names` lists unresolved relationship sites as `path:line kind name` instead; it defaults to off.

`--path` defaults to absent; `--limit` controls displayed files or site rows and uses the shared default and cap. `[internal]` marks unresolved call names also present in the index. External calls contribute to counts too.

Summary counts are build-time data with no freshness check. Site mode checks displayed locations for changes; locations unavailable from extraction are reported separately on stderr.

## Project memory

aw keeps agent-authored project notes — decisions, gotchas, facts, task
wrap-ups — in `.aw/memory.db`, a separate SQLite store that survives builds,
`doctor --invalidate`, and index schema skew. Notes are prose written by the
calling agent or human; aw never originates one. Every mem command opens with
the caveat `aw: memory — agent-authored notes, candidates only, never proof of
current behavior` on stdout (for `mem export` it goes to stderr so stdout
stays pure JSONL).

### `mem write <KEY>`

```text
aw mem write <KEY> --body <TEXT> --kind <decision|gotcha|fact|task>
    [--tags <TOKENS>] [--refs <NAMES>] [--scope durable] [--ttl <SECS>]
```

Persist one note under a stable identity (`<= 200` chars, no control
characters). The body is agent-authored prose, hard-capped at 8192 bytes —
never a verbatim source dump; source evidence stays in the repository.

| Option | Default | Meaning |
|---|---|---|
| `--body <TEXT>` | required | Note body (`<= 8192` bytes, must not be empty) |
| `--kind <KIND>` | required | One of `decision`, `gotcha`, `fact`, `task` |
| `--tags <TOKENS>` | absent | Whitespace-separated lowercase tokens, at most 16 x 32 chars (stored as a sorted set) |
| `--refs <NAMES>` | absent | Whitespace-separated qualnames / repo paths the note is about, at most 16 x 256 chars (stored as a sorted set) |
| `--scope <SCOPE>` | `durable` | `durable` never expires; `session` expires after the TTL |
| `--ttl <SECS>` | `604800` with `--scope session` | Session lifetime in seconds; only valid with `--scope session` (OQ2 default: 7 days) |
| `--supersedes <KEY>` | absent | Mark the named key's current note as replaced by this one; unknown target exits 1, the same key exits 2 |

Identical content under the same key (compared by an FNV-1a hash over kind,
body, tags, refs, and scope after normalization) is a successful no-op naming
the existing revision — agent retries and framework replays are safe, and a
replayed `--supersedes` does not re-apply. Different content under the same
key writes a new revision in one `BEGIN IMMEDIATE` transaction; the previous
revision is marked superseded and stays out of reads but remains in the store
until `mem gc` retention (30 days from supersession, not creation). A
`--supersedes` target is marked superseded in the same transaction — an
unknown target rolls the whole write back (exit 1). Concurrent writers
serialize on a 5 s busy timeout or exit 4; a `UNIQUE(key, rev)` failure is
treated as a defect (one retry, then exit 6).

### `mem query [TERMS]`

```text
aw mem query <TERMS> [--kind <KIND>] [--scope <SCOPE>] [--limit <N>]
aw mem query --all [--kind <KIND>] [--scope <SCOPE>] [--limit <N>]
```

Rank current notes for the terms: FTS5 BM25 over key/tags/body tokens plus
ref/token-overlap plus recency decay, min-max fused with fixed weights
(0.5/0.3/0.2) and deterministic tie-breaks (score, then created_at, then key,
then revision). The same tokenizer splits camelCase/snake_case on both the
stored text and the query, so `calleeEmptyReason` in a body matches the query
`callee empty reason`. At most 256 FTS candidates are fused per query.
`--all` (mutually exclusive with terms) lists current notes by recency;
`--all --include-superseded` extends the listing with superseded revisions,
each header tagged `[superseded]` (not valid with terms). `--limit` defaults
to 10 and clamps to 1..100. Rows print
`key rev kind scope created_at` followed by the indented body; a capped
result ends with `showing N of M` on stdout and the cap notice on stderr.
`--kind`/`--scope` are plain equality filters. A query without hits exits 1.

### `mem get <KEY>`

Print the current note for the exact key — the same row shape as `mem query`
plus a final `revs: <N>` line with the key's total revision count. Unknown
keys exit 1.

### `mem delete <KEY>`

Hard-delete every revision of the key and their search-index rows in one
transaction, and print `deleted '<KEY>' revs=<N>`. Unknown keys exit 1 and
nothing is created. There is no undo — `mem export` beforehand is the backup.

### `mem export`

```text
aw mem export [--limit <N>]
```

Write current notes as one JSON object per line to stdout — key, rev, kind,
scope, session, body, tags, refs, source, created_at, expires_at — so a
restore preserves the TTL shape. stdout carries ONLY the JSONL; the caveat
goes to stderr. Without `--limit` every current note is exported; with
`--limit <N>` the N most recent by `created_at` (deterministic). The caller
redirects the output to a file outside `.aw/` — aw writes only under `.aw/`.

Restore = replay `aw mem write` per line into a fresh store (idempotent by
content-hash dedup; session rows would re-derive `--ttl` as
`expires_at - created_at`). Restore is content-exact, not a bit-exact
replica: `created_at` and rev are re-stamped by the replay.

### `mem gc`

Reclaim disk in one transaction: expired session notes are deleted with their
search-index rows, and superseded revisions whose supersession is older than
the 30-day retention constant are purged (the clock is `superseded_at`, so a
revision superseded yesterday but created 40 days ago survives). Prints
`expired=<N> removed, superseded=<M> removed, durable=<K> rows`. Missing
store exits 1; nothing is created.

### Memory exit and data contracts

Missing `.aw/memory.db` on a read path exits 1 with
`aw: no-memory: no .aw/memory.db under <root> — write with aw mem write`. A
corrupt store exits 3. A store newer than the binary, or one missing required
structure, exits 5 with a message naming `aw mem export` as the recovery path
— memory cannot be rebuilt from source, unlike the index, so an older
structurally complete store still opens read-write. Every mem command
journaled carries `body_len`, never the body text. Expired session notes are
invisible to every read regardless of `gc`; the purge only reclaims disk.

## Read summaries

### `stats`

```text
aw stats [--path <PATH>]
```

Print build-time file, generated-file, symbol, edge-kind, parse-error, and unresolved-call counts, plus database size and versions. This is not a freshness check. `--path` defaults to absent and scopes every counter to one repo-relative file or directory prefix; a scoped line inserts `scope=<PATH>` after `aw stats:`. `db_bytes` and the version fields always describe the whole index. A scope with no indexed files exits 1.

## Journal and environment

### `usage`

```text
aw usage [--quality]
```

Summarize invocation counts, errors, latency, empty/stale rates, and ambiguous-pointer counts. No index is needed; an empty journal returns exit 1.

`--quality` adds the empty-result split (`empty_ok` for exit 0, `empty_error` for non-zero), the capped-result count (`total > shown`), and the `empty_ok` percentage. It reads only fields already present in `.aw/usage.log`; the journal schema and the default summary do not change.

### Local data and controls

| Path or environment variable | Purpose |
|---|---|
| `.aw/index.db` | Local SQLite index (derived; `aw build` recreates it) |
| `.aw/memory.db` | Project-memory store (primary data; agent-authored notes) |
| `.aw/usage.log` | Invocation metadata: tool, arguments, outcome, latency, and result counts |
| `AW_NO_JOURNAL=1` | Disable the journal; equivalent to `--no-journal` |
| `AW_SESSION` | Optional journal attribution, trimmed and capped at 64 characters |
| `AW_ARM` | Optional comparison-group attribution, trimmed and capped at 64 characters |

`aw` 0.3 wrote `.aw/usage-results.jsonl`; 0.4 no longer writes or reads it, and the file can be deleted manually.

Journals stay local and rotate after exceeding 5 MiB to a `.1` generation. Writes are best-effort and do not change command results on failure. They can contain repository paths, symbol names, and displayed signatures; treat them as project data.

The CLI does not journal `doctor`, `usage`, or the MCP server invocation itself. MCP tool calls can be journaled separately. Opting out does not delete existing data.

## Serve an MCP client

### `mcp`

```text
aw mcp [--all-tools]
```

Serve newline-delimited JSON-RPC 2.0 over stdin/stdout until input closes. A client starts this process; aw does not install host configuration or a background service.

By default the tool list is the eight core navigation tools — `build`, `callers`, `callees`, `path`, `context`, `imports`, `impact`, and `symbols` — plus the memory write/query loop `mem_write` and `mem_query`. `--all-tools` defaults to off; when set, it exposes the full surface of 23 tools, adding the inventory, analysis, and usage operations plus `mem_get`, `mem_delete`, `mem_gc`, and `mem_export`.

`doctor` is never exposed as a tool. Global root and journal controls apply. Frames are limited to 8 MiB. Do not send ordinary shell text or log messages into the protocol stream.

See [agent integration](agent-integration.md) for host setup and fallback guidance.

## Interpret output and failures

Locations use repository-relative paths and 1-based line numbers. Definition ranges include both endpoints. Header caveats and summary rows are not source pointers.

- A `[ambiguous]` edge is a possible relationship, not a uniquely resolved call.
- Capped result lists report `showing N of M` and usually a stderr cap notice. Context reports its counts in each section heading.
- Many pointer queries warn only about changed files in the displayed, post-cap set. They do not check every indexed file or detect every newly added file.
- An empty result is not proof of absence. Inspect the reason, rebuild, and fall back to source search.
- Runtime diagnostics use `aw: <class>: <detail>` on stderr. Argument-parser errors can use clap's own format. Ambiguous target candidates print on stdout, at most 50.
- `dead` retains its candidate caveat on stdout even for an empty result; other analysis commands can also emit caveats.

### Exit codes

| Code | Meaning | Action |
|---|---|---|
| `0` | Success, help/version display, or a closed downstream output pipe | Read result-specific caveats and counters |
| `1` | No results, no index, no memory store, unknown memory key, empty journal/input, or missing batch names | Inspect the reason; build, write a note, or search source as appropriate |
| `2` | Invalid usage or ambiguous name | Fix arguments or use a full printed qualified name |
| `3` | Corrupt index or corrupt `memory.db` | Rebuild the index; restore memory from an `aw mem export` backup |
| `4` | Build lock held, or a memory write timed out on the busy lock | Wait for the running build or write, then retry |
| `5` | Schema mismatch — index: rebuild with the installed binary; memory: restore from an `aw mem export` backup | Follow the message for the store that failed |
| `6` | Internal failure, including I/O/database errors or unexpected panic | Preserve the diagnostic and report it |
| `7` | Build exceeded the parse-error-rate threshold | Inspect source syntax and coverage; adjust the threshold only knowingly |

Exit codes describe the operation, not certainty of analysis. `context` can succeed with empty relationship sections.

## Static-analysis boundaries

Only Python, TypeScript/TSX, JavaScript/JSX, and Rust extensions listed in [getting started](getting-started.md#know-what-is-indexed) are indexed. Source files over 1 MiB and `.aw/` are excluded; ignore rules and parse errors can reduce coverage further.

Queries operate on one local repository. Unresolved calls, runtime dispatch, external dependencies, inheritance, exports, framework discovery, and macro behavior can be incomplete. Rebuild after edits; use grep, a language server, source reading, and the normal test runner for questions the index cannot establish.
