# Architecture: Code Index

The code index answers structural questions — who calls this, what does this
call, which files depend on this file — without running the program, a language
server, or a network call. This page describes how the index is built, stored,
queried, and invalidated.

## Storage

| Artifact | Location | Lifetime |
|---|---|---|
| Code index | `.engram/index.db` | Rebuilt by `eg build`; disposable |
| Build lock | `.engram/build.lock` | Held for the duration of one build |
| In-progress index | `.engram/index.db.new` | Renamed onto `index.db` on success |
| Git-ignore marker | `.engram/.gitignore` | Written once, content `*` |

Schema files live in [`schema/`](../../schema):

- [`schema/index.sql`](../../schema/index.sql) — tables, FTS virtual table, and
  the `user_version` contract.
- [`schema/indexes.sql`](../../schema/indexes.sql) — secondary indexes, created
  after the bulk insert so each index is sorted once.

Both are compiled into the binary with `include_str!`; the deployed database is
the only runtime artifact.

`PRAGMA user_version` is stamped with `SCHEMA_VERSION` at build time and
verified on every open. Older and newer databases both exit `5`: the index is
never migrated in place, because a rebuild is faster and cannot corrupt a
partially migrated tree.

## Build lifecycle

1. **Discover** (`src/walk.rs`) — enumerate Git-visible files, honour
   `.gitignore`/`.ignore`, skip `.engram/` and `.aw/` by whole path segment,
   reject symlinked ancestors, and cap each file at 1 MiB.
2. **Hash** (`src/hash.rs`) — FNV-1a 64 over the file bytes, combined with size
   and nanosecond mtime to decide whether a file can be carried forward.
3. **Parse** (`src/parse.rs`) — Tree-sitter grammars for Python, JavaScript,
   TypeScript/TSX, and Rust. A parse error is recorded per file, not fatal.
4. **Extract** (`src/extract/walk/`) — symbols (module, class, function, method,
   trait, interface), signatures capped at 160 characters, and call, import,
   and inheritance sites with their line numbers.
5. **Resolve** (`src/extract/resolve/`) — bind call sites and imports to
   extracted symbols using module/class scopes, aliases, re-exports, and
   receiver typing. One destination yields `EXTRACTED`; several yield
   `AMBIGUOUS`; none stays unresolved.
6. **Insert** (`src/db/inserts.rs`) — fixed-width batches inside one
   caller-owned transaction, then one FTS pass over the inserted symbols.
7. **Publish** (`src/db.rs`) — create secondary indexes, write the meta
   manifest, fsync the new file, then rename it onto `index.db`. A concurrent
   reader sees the old database or the new one, never a partial file.

The whole build holds one advisory `flock` on `.engram/build.lock`. A second
build exits `4` immediately and reports the holder's pid.

### Incremental fast path

Unchanged files (same FNV-1a hash, size, and mtime) carry their symbols and
edges from the previous database. A carried file is re-extracted when:

- a newly added or renamed symbol matches one of its unresolved call sites;
- a file it imports changed; or
- the grammar, signature, or metrics version changed.

The build prints `carried=N` and a per-phase wall-time breakdown. `--force`
skips the fast path and re-extracts everything.

## Query engines

| Surface | Module | Mechanism |
|---|---|---|
| `callers`, `callees`, `path`, `context` | `src/query/callgraph/` | Bounded traversal over resolved `calls` edges, depth `1..10` |
| `impact` | `src/query/impact/` | Reverse closure over `calls`, `imports`, and `inherits`, with a file-level summary |
| `imports`, `symbols`, `unresolved` | `src/query/analysis/` | Direct table scans with deterministic ordering |
| `search` | `src/query/analysis/search.rs` | FTS5 `bm25` over split identifier tokens, then caller-count ranking |
| `reuse` | `src/query/reuse/` | Four static duplicate detectors over the symbol corpus |
| `stats`, `doctor` | `src/insights.rs`, `src/doctor.rs` | Meta counters and integrity/staleness probes |

Every ordering ends with deterministic tie-breakers (`qualname`, `file_path`,
`line_start`) so two binaries over the same database agree byte for byte.

### Reuse detectors

| Id | Detector | Signal |
|---|---|---|
| D1 | Callee footprint containment | One symbol's callee set is contained in another's |
| D2 | Interface and signature token overlap | Near-identical parameter and return types |
| D3 | Raw client / abstraction bypass | A symbol calls a raw client that a canonical wrapper already covers |
| D4 | Guarded pass-through wrapper | A wrapper adds only a guard around one canonical call |

Detector hits are candidates, never proof. `eg reuse` reports the overlap
percentage, the shared items, and the canonical symbol with its caller count.

## Staleness

`file_changed` compares the stored size and mtime against disk. The first line
of pointer output carries `[stale]` when any file in the result set changed
after the build, so a stale answer announces itself instead of looking current.
`eg doctor` reports the repository-wide stale ratio.

## Exit codes

`0` ok · `1` empty result or missing index · `2` usage, ambiguous name, or a
security refusal · `3` corrupt database · `4` build lock held · `5` schema skew
· `6` internal or I/O error · `7` parse-error rate above the threshold.

## Related

- [Memory store architecture](memory-store.md)
- [Concepts](../concepts.md) — symbols, edges, confidence, and pointer output
- [Evaluation](../evaluation.md) — measured recall, latency, and limits
