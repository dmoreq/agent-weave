# Architecture: Memory Store

Project memory persists agent-authored notes — decisions, gotchas, facts, and
tasks — across index rebuilds and across sessions. This page describes the
storage shape, the retrieval scoring, the reference-currency model, and the
confinement rules.

## Two stores, one contract

| Store | Location | Scope |
|---|---|---|
| Local | `.engram/memory.db` | One repository's decisions and gotchas |
| Global | `~/.engram/memory.db` | Cross-project conventions and developer profile |

`$ENGRAM_GLOBAL_DIR` overrides the global location; the pre-rename
`$AW_GLOBAL_DIR` is still honoured. A pre-rename `.aw/memory.db` or
`~/.aw/memory.db` is copied into the new location atomically on the first read
or write when the new store is absent. The legacy file is never moved or
deleted, so a downgrade keeps working.

Memory is **primary data**, unlike the index. It is never rebuilt from source
and is never reset by `eg doctor --invalidate`.

## Schema

[`schema/memory.sql`](../../schema/memory.sql) defines one `memories` table and
one FTS5 table:

- `key` — caller-chosen stable identity; a rewrite appends a revision.
- `rev` — 1-based revision, unique per key; the current row is the highest
  non-superseded revision.
- `kind` — closed set: `decision`, `gotcha`, `fact`, `task`, `procedure`.
- `scope` — `durable`, or `session` with a TTL deadline in `expires_at`.
- `body` — prose, capped at 8192 bytes; secrets and credentials are rejected at
  the trust boundary.
- `tags`, `refs` — bounded token lists (16 items each) used by retrieval.
- `content_hash` — FNV-1a 64 over kind, body, tags, refs, and scope, so an
  identical replay is idempotent instead of a new revision.
- `superseded_by`, `superseded_at` — cross-key and same-key replacement chain.
- `memories_fts` — `unicode61` FTS5 over pre-tokenized key, tags, and body.
  `refs` stay out of the FTS text; the reference signal covers them separately.

`PRAGMA user_version` carries `MEMORY_SCHEMA_VERSION` and a compiled
required-structure manifest is checked at open. An older store that still has
every required column and the FTS table opens read-write (additive evolution).
A newer store, or one missing required structure, exits `5`.

Secondary indexes live in the same file (not in `schema/indexes.sql`): memory
writes are single-row transactions with no bulk-insert phase to post-index.

## Retrieval

`eg mem query` fuses three signals, each min-max normalized across the
candidate set:

| Signal | Weight | Meaning |
|---|---|---|
| Lexical | 0.5 | Inverted FTS5 bm25 rank over key, tags, and body tokens |
| Reference | 0.3 | Token overlap between `--ref` and the note's `refs` |
| Recency | 0.2 | Bounded decay `1 / (1 + age_days)` |

Ties break on score, then `created_at`, `key`, and `rev`, so identical inputs
produce an identical order. `eg mem query` with no terms lists current notes by
recency instead of ranking.

Without `--global` the search covers the local store and the global store;
`--global` restricts it to the global store.

## Reference currency

A note can be anchored to a file or symbol through `--refs`. At render time
each anchor is classified against the current index:

- `[stale-ref]` — the anchor moved or the file changed after the note was
  written. `[modified-ref]` and `[broken-ref]` refine the reason.
- `eg mem touch <key>` refreshes the timestamp without creating a revision once
  the reader has re-audited the anchor.

`eg context <name>` co-retrieves gotchas and decisions that reference the
symbol, and `eg build` emits a stderr note when modified files touch guarded
areas.

## Lifecycle

- **Write** — append a revision, update the FTS row, and commit in one
  transaction. `--touch` refreshes `created_at` when the content hash matches.
- **Supersede** — `--supersedes <key>` marks the older key's current row and
  records the replacement without deleting history.
- **GC** — `eg mem gc` purges expired session notes and superseded revisions
  older than the 30-day retention window.
- **Export** — `eg mem export` writes JSONL (default) or fenced Markdown
  (`--format markdown`) for the current notes. Both formats read the same
  expiry-filtered current-row query.
- **Import** — `eg mem import <path>` restores either format (or stdin, `-`) in
  one transaction. Imported rows carry `source=import`. A key whose local
  content differs is skipped (exit 2) unless `--force` opens a new revision.

## Concurrency and failure classes

Writes use one immediate transaction with a 5-second busy timeout. A writer
that loses the lock exits `4` (`memory write in progress — retry`) rather than
reporting a unique-key violation. Corruption maps to exit `3` with a restore
hint, and schema skew maps to exit `5`.

## Confinement and provenance

Writes are confined to `.engram/` and `~/.engram/`. Symlinked store paths are
refused, the global directory is created `0700`, and `memory.db` plus
`usage.log` are `0600` on Unix. A repository that commits `.engram/` to Git is
rejected with exit `2` (`eg: security: ...`): a tracked store would let a
repository inject attacker-authored notes into every agent that reads it.

## Related

- [Code index architecture](code-index.md)
- [Usage: project memory](../usage.md#project-memory)
- [Agent integration](../agent-integration.md)
