# Use aw for code navigation

Use aw to narrow the source you need to read. Verify decisions against source and tests; indexed relationships are not a complete program model.

The examples below use aw's repository after `aw build`. They show commands, not recorded output or guaranteed nonempty results.

## Choose a query

| Your task | Start with | Without aw |
|---|---|---|
| Find an unfamiliar symbol | `aw search 'parse tree'` | Search names and read matches with grep |
| Read a file's structure | `aw symbols src/build.rs --outline` | Read the file or use an editor outline |
| Inspect a function before editing | `aw context run_build` | Read its definition, then search callers and calls |
| Find callers | `aw callers run_build` | Search references and reject non-call matches |
| Follow transitive callers | `aw callers run_build --depth 2` | Follow each reverse call hop manually |
| Follow outgoing calls | `aw callees run_build --depth 2` | Follow each call manually |
| Trace a route between functions | `aw path run_build fnv1a64` | Read and follow candidate call chains |
| Inspect dependent files | `aw impact src/hash.rs` | Search references and imports across files |
| Inspect imports | `aw imports src/build.rs` | Read import declarations and resolve targets |
| Find implementations of a type | `aw implementers <name>` | Read the type hierarchy and implementation blocks |

Trust the rows a successful query returns for the navigation question you
asked; do not repeat the same lookup with grep. Use source reads for behavioral
claims, edits, and for capped, ambiguous, stale, or empty results. Treat
`dead` and `unresolved` as candidates.

See the [CLI reference](cli-reference.md) for all options and maintenance commands.

## Inspect a change's surroundings

Start with a fresh index, then inspect the target definition and direct neighbors:

```sh
aw build
aw context run_build
aw callers run_build --limit 100
aw impact src/build.rs --sites
```

`context` returns a definition range, caller sites, and callee definitions. Its per-section limit defaults to 10; an empty caller or callee section still permits a successful result.

`callers` identifies call sites, not all textual references. `--depth N` follows the resolved reverse call closure and tags rows with their shortest depth. Each caller symbol appears once at its shortest depth; alternate longer routes are not repeated. The queried symbol is the closure seed and is excluded from transitive rows, so a self-call is visible only with `--depth 1`. `impact` reverses the dependency direction: it finds files that depend on the target file, excluding that file itself.

By default, impact uses calls and emits one row per caller file with a site count. When that default finds nothing but the target file has importers or inheritors, the empty result names those counts and points at `--edges imports,inherits`. Use `--sites` for individual sites, or extend the relationships and depth explicitly:

```sh
aw impact src/hash.rs --edges all --depth 3
```

Do not treat this list as every file that can break. Unresolved calls, external callers, imports, and inheritance relationships can be missing.

## Follow execution routes

```sh
aw callees run_build --depth 2
aw path run_build fnv1a64
```

`callees` shows a bounded forward call closure. `path` finds one shortest forward route through resolved call edges, with call-site pointers and a symbol chain.

Swap the two arguments to ask the reverse question. A missing route means no resolved route was found within the depth limit, not that execution cannot reach the destination.

Open each hop before changing behavior. Dynamic dispatch, callbacks, and macros can hide routes from static extraction.

## Find definitions and read structure

```sh
aw search 'parse tree'
aw search run_build
aw symbols src --outline
```

Quote multiword search terms: the CLI accepts one positional search string. Search ranks identifier, path, and signature tokens; it outputs concise interface cards (`path:line kind qualname signature [callers=N]`). It does not search source bodies, comments, or docstrings.

`symbols` lists definitions by path and line. Signatures are whitespace-collapsed and capped at 160 characters, so open the source for full declarations and bodies.

## Detect anti-reinvention candidates

```sh
aw reuse
aw reuse src
```

- `reuse` detects candidate duplicate functions, callee footprint overlap, and utility abstraction opportunities across the codebase.

Do not refactor code solely because it appears here. Confirm semantics, visibility, and existing tests first.

## Read repository summaries

```sh
aw stats
aw stats --path src
```

`stats` describes the index, not current runtime behavior; `--path` scopes every counter to one prefix but leaves `db_bytes` and the version fields global.

## Remember what a session settled

aw carries a project-memory store, `.aw/memory.db`, for the knowledge a coding
session produces: settled decisions, gotchas that cost real debugging time,
and task wrap-ups that are not yet in committed docs. Notes are agent-authored
prose — aw never originates one, and every mem command opens with the caveat
that notes are candidates, never proof of current behavior.

```sh
# A decision settled after real investigation — write it once:
aw mem write resolver-confidence-tiers --kind decision \
  --body "the resolver keeps confidence tiers; AMBIGUOUS edges keep dst_id NULL" \
  --tags "resolver confidence" --refs src/query/callgraph.rs

# A later session retrieves it by content instead of re-deriving it:
aw mem query confidence tiers

# Rework the decision: same key, new content -> a new revision, nothing overwritten:
aw mem write resolver-confidence-tiers --kind decision --body "confidence tiers v2: ..."

# Cross-project developer profile or conventions:
aw mem write dev-profile --kind decision --body "Preferred stack: Rust." --global
```

Write when a decision was settled, a gotcha cost genuine time, or a task ends
with unmerged conclusions. Use `--global` to persist developer profiles and
cross-project conventions into `~/.aw/memory.db`. Query at session start and
before re-investigating an area. Without `--global`, `aw mem query` searches both
local (`.aw/memory.db`) and global (`~/.aw/memory.db`) stores seamlessly: global
notes carry a `[global]` badge, and local notes shadow colliding global keys.
`mem get <key> [--global]` shows one note with its revision count; `mem delete <key> [--global]`
removes a key; `mem export [--global]` writes a JSONL backup (restore = `mem write`
replay); `mem gc [--global]` purges expired session notes and superseded revisions
older than 30 days, then reports counts.

Transient scratch ("tried X, it failed here") belongs to the session lane:
`aw mem write scratch --kind fact --scope session --body ...` expires after
7 days by default (`--ttl <SECS>` overrides) and disappears from every read
once expired, so durable memory stays clean. `--supersedes <key>` records
that a new note replaces an old one across keys; `--include-superseded`
surfaces replaced revisions when you need the audit trail.
Unlike the index, this store is primary data: it survives `aw build` and
`aw doctor --invalidate`, and the only backup is `aw mem export`.

See [CLI reference](cli-reference.md#project-memory) for the full contracts,
caps, and exit codes.

## Recover from uncertain results

See [Concepts](concepts.md) for the index model behind pointers, confidence, and staleness. The table below describes what each signal does and does not mean.

| Result | Meaning | Next action |
|---|---|---|
| Name is ambiguous; exit 2 | Multiple indexed definitions match | Copy a full printed qualified name; do not merge candidates silently |
| Symbol or path is missing; exit 1 | The index lacks that target | Check spelling, root, supported extensions, ignore rules, and rebuild |
| No callers or route; exit 1 | No matching indexed relationship | Search source and inspect dynamic or external relationships |
| `showing N of M` | Output is capped | Narrow the scope or raise `--limit`; do not treat N as the total |
| Changed-result warning | Displayed source differs from the index | Run `aw build`, then repeat the query |
| No freshness warning | Checked files did not trigger a warning | Rebuild after edits anyway; omitted or new files may not be checked |

Find extraction and resolution gaps before trusting an empty result:

```sh
aw doctor
aw unresolved --path src
aw unresolved --names --path src
git grep -n fnv1a64 -- src tests
```

An `[internal]` unresolved call name also exists in the index and can indicate a resolver gap. External and standard-library calls also contribute to unresolved counts; the unresolved rate is not a whole-program missed-call rate.

## Compare workflows fairly

Two comparisons answer different questions:

- **No aw:** use source reading, grep, editor navigation, and the normal test runner without any aw commands.
- **Feature disabled:** keep other aw commands available, but omit the feature under investigation. For example, compose `symbols`, `callers`, and `callees` instead of `context`.

A feature-disabled result does not establish the benefit of aw over no aw. A reduction in tool calls does not establish lower tokens, latency, cost, or production error rates.

This guide makes no measured savings claim. See [Evaluation](EVALUATION.md) for the scope and limitations of reported measurements.

## Keep local usage data under control

Most commands append local usage metadata. Use `aw --no-journal` or `AW_NO_JOURNAL=1` to disable the usage journal for an invocation.

`aw usage` summarizes recorded activity; see [journal controls](cli-reference.md#journal-and-environment). `aw usage --quality` adds the empty-result split, the capped-result count, and percentages from the existing journal fields.
