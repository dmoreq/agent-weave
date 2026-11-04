# CLI reference

Use the [usage guide](usage.md) to choose a workflow. This page describes the command interface defined in [`src/cli.rs`](../src/cli.rs), with behavior from the query and maintenance implementations.

## Find the command for your task

| Task | Commands |
|---|---|
| Create or refresh an index | `build` |
| Inspect a definition and its neighbors | `context`, `callers`, `callees`, `path` |
| Inspect dependencies | `impact`, `imports`, `implementers`, `tests` |
| Find definitions and files | `search`, `symbols`, `resolve`, `files` |
| Investigate cleanup candidates | `dead`, `clones`, `complexity` |
| Read repository summaries | `stats`, `hotspots`, `report` |
| Diagnose or remove local data | `doctor` |
| Inspect recorded usage | `usage` |
| Connect an MCP client | `mcp` |

## Trust contract

A successful query is ground truth for the rows it returns. Re-run or read
source when output is capped, ambiguous, stale, or empty; those are coverage
limits, not routine re-verification. Treat `tests`, `dead`, and `unresolved`
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
- `hotspots --limit` defaults to **10** and clamps at **100**. Zero is a usage error.
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

`--invalidate` defaults to off. When set, remove the **entire `.aw/` directory**, including journals and any other files there; no index is required. Source files and the executable remain unchanged.

## Inspect definitions and calls

### `context <NAME>`

```text
aw context <NAME> [--limit <N>]
```

Print a definition range, kind, qualified name, signature, direct caller sites, and direct callees. `--limit` defaults to 10 per section, maximum 50.

Resolved callees point to definitions; ambiguous callee entries can point to the originating site and carry `[ambiguous]`. Missing sections do not fail a query when the target exists. This is static context, not a complete runtime call graph.

### `callers <NAME>`

```text
aw callers <NAME> [--limit <N>] [--depth <N>]
```

Print sites that call the symbol, with the enclosing caller's qualified name. `--depth` defaults to 1 for direct callers; larger depths follow the reverse call closure and append `[depth=N]` to each row. Each caller symbol appears once at its shortest depth; a symbol reached by a shorter route is not repeated on a longer route, and its inbound edges are expanded once. The queried symbol is the closure seed and is excluded from transitive rows, so a self-call is visible only with `--depth 1`. `--limit` uses the shared default and cap. Possible ambiguous edges are tagged; they are not proven calls to the target.

### `callees <NAME>`

```text
aw callees <NAME> [--limit <N>] [--depth <N>]
```

Print outgoing callees; resolved rows point to definitions and include callee identity. `--depth` defaults to 1 for direct callees; larger depths follow the forward call closure. `--limit` uses the shared default and cap.

### `path <FROM> <TO>`

```text
aw path <FROM> <TO> [--depth <N>]
```

Find one shortest forward route over resolved call edges. Print a static-analysis caveat, call-site pointers in hop order, and the symbol chain. `--depth` defaults to 10; there is no `--limit` option.

Swap arguments for the reverse question. Equal endpoints or no route within the depth limit return exit 1. Ambiguity identifies the failing endpoint; dynamic dispatch can hide routes.

## Inspect dependencies and test candidates

### `impact <FILE>`

```text
aw impact <FILE> [--sites] [--depth <N>] [--edges <KINDS>]
    [--context] [--explain] [--limit <N>]
```

Find files that depend on the indexed target file, not its outbound dependencies. Default rows group by caller file and include `(N sites)`; `--limit` uses the shared default and cap. An empty default (calls-only) result names how many files import or inherit from the target and points at `--edges imports,inherits`.

| Option | Default | Meaning |
|---|---|---|
| `--sites` | Off | Show individual relationship sites instead of grouping by file |
| `--depth <N>` | `1` | Reverse dependency traversal depth |
| `--edges <KINDS>` | `calls` | Comma-separated `calls`, `imports`, `inherits`, or `all`; do not combine `all` with explicit kinds |
| `--context` | Off | Append the enclosing callable when available |
| `--explain` | Off | Append relationship kind and depth |

Unresolved imports and bases are omitted from extended dependency traversal. Dynamic calls can hide dependents even when only calls are selected.

### `imports <FILE>`

```text
aw imports <FILE> [--limit <N>]
```

List outbound resolved import targets for an indexed file. External and unresolved imports are omitted. `--limit` uses the shared default and cap.

### `implementers <NAME>`

```text
aw implementers <NAME> [--limit <N>]
```

Find direct resolved inbound inheritance or implementation relationships. Dynamic inheritance and cross-file Rust implementation blocks can be missing. `--limit` uses the shared default and cap.

### `tests <NAME_OR_FILE>`

```text
aw tests <NAME_OR_FILE> [--depth <N>] [--limit <N>]
```

Find statically reachable test candidates for a symbol or exact indexed file. `--depth` defaults to 10; `--limit` uses the shared default and cap.

Candidates carry the index's test classification: test paths (`tests/`, `test/`, `__tests__/`), test-shaped names (`test_*`, `test` + uppercase), test-shaped files (`test_*.py`, `*_test.py`, `*_test.go`, `*_test.dart`, `*.test.*`, `*.spec.*`), Rust `#[test]`/`#[cfg(test)]` items, and TS `describe`/`it` scopes. Fixtures, generated tests, and indirect tests can be missed. This command does not run tests.

## Find symbols and files

### `search <TERMS>`

```text
aw search <TERMS> [--limit <N>]
```

Rank symbols using BM25 token search over names, qualified names, file paths, and signatures. Quote multiple words as one argument, for example `aw search 'parse tree'`.

| Option | Default | Meaning |
|---|---|---|
| `--limit <N>` | `50` | Maximum displayed matches, cap 10000 |

camelCase and snake_case split into tokens. Normal search uses prefix terms and keeps at most 32 tokens, with a warning when truncated. It does not search bodies, comments, or docstrings. Rows carry a location, qualified name, and available signature.

### `symbols <FILE_OR_DIRECTORY>`

```text
aw symbols <FILE_OR_DIRECTORY> [--outline] [--metrics] [--limit <N>]
```

List indexed definitions by path and line. `--outline` adds signatures; `--metrics` adds per-symbol syntax metrics. Both default to off and can be combined. `--limit` uses the shared default and cap. Accept `.` or `./` as the repository root.

### `resolve --stdin`

```text
aw resolve --stdin
```

Read one name per line from standard input. `--stdin` defaults to off but is required; there are no other command-specific options.

Ignore blank lines and trimmed lines starting with `#`. Accept at most 1 MiB input, 4096 bytes per original line, and 1000 retained names. Print `found`, `ambiguous`, or `missing` rows; ambiguity renders at most 50 candidates per name.

Exit 0 when all names resolve uniquely, 1 for missing names with no ambiguity or for empty input, and 2 if any name is ambiguous. Ordinary index and input errors retain their standard exit codes.

### `files`

```text
aw files [--path <PATH>] [--limit <N>]
```

List indexed paths with build-time symbol counts, byte sizes, and generated tagging. `--path` defaults to absent; `--limit` uses the shared default and cap. This inventory does not check current source freshness.

## Investigate candidates and gaps

### `dead`

```text
aw dead [--exclude-tests] [--path <PATH>] [--limit <N>]
```

List candidate functions, methods, classes, traits, and interfaces without indexed inbound calls, imports, inheritance, or typed-receiver (`type_ref`) references. Rows render `path:line qualname`. Relevant ambiguous references also suppress candidates; members under traits and interfaces are excluded by policy.

`--exclude-tests` drops rows carrying the index's test classification (test paths, test-shaped names and files, Rust `#[test]`/`#[cfg(test)]` items, TS `describe`/`it` scopes), not test-runner discovery. All switches default to off, `--path` defaults to absent, and `--limit` uses the shared default and cap.

Always treat the output as candidates. Entry points, external consumers, dynamic dispatch, star imports, module-level imports, type references, and attribute references are not proven absent.

### `unresolved`

```text
aw unresolved [--names] [--path <PATH>] [--limit <N>]
```

By default, summarize unresolved callees by file and include common unresolved names. `--names` lists unresolved relationship sites as `path:line kind name` instead; it defaults to off.

`--path` defaults to absent; `--limit` controls displayed files or site rows and uses the shared default and cap. `[internal]` marks unresolved call names also present in the index. External calls contribute to counts too.

Summary counts are build-time data with no freshness check. Site mode checks displayed locations for changes; locations unavailable from extraction are reported separately on stderr.

### `complexity`

```text
aw complexity [--min-cyc <N>] [--min-loc <N>] [--min-branches <N>]
    [--min-nest <N>] [--path <PATH>] [--limit <N>]
```

Rank functions and methods by descending cyclomatic proxy, then effective lines of code. All thresholds must hold.

| Option | Default | Meaning |
|---|---|---|
| `--min-cyc <N>` | `1` | Minimum callable cyclomatic proxy |
| `--min-loc <N>` | `0` | Minimum effective lines of code |
| `--min-branches <N>` | `0` | Minimum branch-node count |
| `--min-nest <N>` | `0` | Minimum nesting metric |
| `--path <PATH>` | None | Restrict file or directory scope |
| `--limit <N>` | `50` | Maximum displayed callables, cap 10000 |

Rows label metrics `cyc`, `branches`, `nest`, and `loc`. These syntax-tree proxies are not cognitive complexity and are not comparable to the file-level metric used by hotspots. Effective LOC excludes blank lines, comment-only lines, and Python docstrings.

### `clones`

```text
aw clones [--min-lines <N>] [--path <PATH>] [--limit <N>]
```

Find duplicate-block candidates using normalized five-line windows. Normalization removes trailing whitespace and blank lines; it does not establish semantic equivalence.

`--min-lines` defaults to 5 and clamps smaller values to 5. `--path` requires both regions under the selected scope and defaults to absent. `--limit` uses the shared default and cap and counts displayed pairs.

Rows identify both inclusive source ranges and enclosing symbols when available. Pair generation retains at most 64 merged regions per window-hash group and 20000 candidate pairs; a warning indicates a lower-bound result when capped.

## Read summaries

### `stats`

```text
aw stats [--path <PATH>]
```

Print build-time file, generated-file, symbol, edge-kind, parse-error, and unresolved-call counts, plus database size and versions. This is not a freshness check. `--path` defaults to absent and scopes every counter to one repo-relative file or directory prefix; a scoped line inserts `scope=<PATH>` after `aw stats:`. `db_bytes` and the version fields always describe the whole index. A scope with no indexed files exits 1.

### `hotspots`

```text
aw hotspots [--limit <N>] [--path <PATH>]
```

Rank indexed files by the sum of normalized file-level decision counts, LOC, 90-day change counts, and 180-day fix-related commit counts. Each metric normalizes within the selected file set; scores from different scopes are not directly comparable.

`--limit` defaults to 10, cap 100; `--path` defaults to absent. Git failures or absent history yield zero change and fix signals. Rows contain score, path, `loc`, `cyc`, `churn90`, and `fix180`; these are prioritization heuristics, not measured defect risk.

### `report`

```text
aw report [--path <PATH>]
```

Print four sections: index overview, top packages by symbol count, five hotspots, and eight most-called symbols. There is no `--limit` option.

`--path` defaults to absent and scopes the map, hotspots, and fan-in sections; the overview retains global totals and adds scoped counts. Freshness checking samples at most 5000 scoped indexed files.

## Journal and environment

### `usage`

```text
aw usage
```

Summarize invocation counts, errors, latency, empty/stale rates, and ambiguous-pointer counts. No index is needed; an empty journal returns exit 1.

### Local data and controls

| Path or environment variable | Purpose |
|---|---|
| `.aw/index.db` | Local SQLite index |
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

By default the tool list is the eight core navigation tools: `build`, `callers`, `callees`, `path`, `context`, `imports`, `impact`, and `symbols`. `--all-tools` defaults to off; when set, it exposes the full surface of 21 tools, including inventory, analysis, and usage operations.

`doctor` is never exposed as a tool. Global root and journal controls apply. Frames are limited to 8 MiB. Do not send ordinary shell text or log messages into the protocol stream.

See [agent integration](agent-integration.md) for host setup and fallback guidance.

## Interpret output and failures

Locations use repository-relative paths and 1-based line numbers. Definition ranges and clone spans include both endpoints. Header caveats and summary rows are not source pointers.

- A `[ambiguous]` edge is a possible relationship, not a uniquely resolved call.
- Capped result lists report `showing N of M` and usually a stderr cap notice. Context reports its counts in each section heading.
- Many pointer queries warn only about changed files in the displayed, post-cap set. They do not check every indexed file or detect every newly added file.
- An empty result is not proof of absence. Inspect the reason, rebuild, and fall back to source search.
- Runtime diagnostics use `aw: <class>: <detail>` on stderr. Argument-parser errors can use clap's own format. Ambiguous target candidates print on stdout, at most 50.
- `dead` and `clones` retain their candidate caveat on stdout even for empty results; other analysis commands can also emit caveats.

### Exit codes

| Code | Meaning | Action |
|---|---|---|
| `0` | Success, help/version display, or a closed downstream output pipe | Read result-specific caveats and counters |
| `1` | No results, no index, empty journal/input, or missing batch names | Inspect the reason; build or search source as appropriate |
| `2` | Invalid usage or ambiguous name | Fix arguments or use a full printed qualified name |
| `3` | Corrupt index | Rebuild the index |
| `4` | Build lock held | Wait for the running build, then retry |
| `5` | Index schema mismatch | Rebuild with the installed binary |
| `6` | Internal failure, including I/O/database errors or unexpected panic | Preserve the diagnostic and report it |
| `7` | Build exceeded the parse-error-rate threshold | Inspect source syntax and coverage; adjust the threshold only knowingly |

Exit codes describe the operation, not certainty of analysis. `context` can succeed with empty relationship sections.

## Static-analysis boundaries

Only Python, TypeScript/TSX, JavaScript/JSX, and Rust extensions listed in [getting started](getting-started.md#know-what-is-indexed) are indexed. Source files over 1 MiB and `.aw/` are excluded; ignore rules and parse errors can reduce coverage further.

Queries operate on one local repository. Unresolved calls, runtime dispatch, external dependencies, inheritance, exports, framework discovery, and macro behavior can be incomplete. Rebuild after edits; use grep, a language server, source reading, and the normal test runner for questions the index cannot establish.
