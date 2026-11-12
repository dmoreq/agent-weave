# Architecture: Memory Store

Project memory persists agent-authored notes — decisions, gotchas, facts, and
tasks — across index rebuilds and across sessions. Grounded in research from
Zero-Mem ([arXiv:2607.29377](https://arxiv.org/abs/2607.29377)), Agent Zero Memory
([arXiv:2608.29606](https://arxiv.org/abs/2608.29606)), and Agent-Native Memory Systems
([arXiv:2606.24775](https://arxiv.org/abs/2606.24775)), `eg` operationalizes memory
with **zero LLM tokens**, immutable AST provenance anchors, and sub-millisecond
SQLite performance.

This page describes the storage shape, the retrieval scoring, the reference-currency
model, and the confinement rules.

## Data Management Model & Two Stores

Under the formal agent-native memory system framework $\mathcal{M}_{sys} = \langle \mathcal{R}, \mathcal{S}, \mathcal{Q}, \mathcal{U} \rangle$ ([arXiv:2606.24775](https://arxiv.org/abs/2606.24775)), `eg` implements representation $\mathcal{R}$ as raw interaction notes, extraction $\mathcal{S}$ through caller-provided schemas, retrieval $\mathcal{Q}$ via FTS5 BM25 + AST graph expansion, and update $\mathcal{U}$ through content-hashed revision chains and TTL-based GC:

| Store | Location | Scope |
|---|---|---|
| Local | `.engram/memory.db` | One repository's decisions, gotchas, and procedures |
| Global | `~/.engram/memory.db` | Cross-project conventions and developer profile |

`$ENGRAM_GLOBAL_DIR` overrides the global location. The pre-rename `.aw/`
store is no longer read: this release removed that compatibility layer, so a
repository still holding notes only in `.aw/memory.db` must be migrated to
`.engram/memory.db` before upgrading.

Memory is **primary data**, unlike the index. It is never rebuilt from source
and is never reset by `eg doctor --invalidate`. As proven by Zhou et al. (2026),
preserving raw human/agent notes avoids the destructive information loss of write-time
LLM summarization while maintaining Pareto-optimal query latency (< 2ms).

## Schema

[`schema/memory.sql`](../../schema/memory.sql) defines one `memories` table, two
FTS5 tables, and one internal state table:

- `key` — caller-chosen stable identity; a rewrite appends a revision.
- `rev` — 1-based revision, unique per key; the current row is the highest
  non-superseded revision.
- `kind` — closed set: `decision`, `gotcha`, `fact`, `task`, `procedure`.
- `scope` — `durable`, or `session` with a TTL deadline in `expires_at`.
- `body` — prose, capped at 8192 bytes; secrets are rejected at the trust
  boundary.
- `tags`, `refs` — bounded token lists (16 items each) used by retrieval.
- `content_hash` — FNV-1a 64 over kind, body, tags, refs, and scope, so an
  identical replay is idempotent instead of a new revision.
- `superseded_by`, `superseded_at` — cross-key and same-key replacement chain.
- `fts_version` — `0` when an older binary wrote the row, `2` when this binary
  indexed it in both FTS tables.
- `memories_fts` — legacy `unicode61` FTS5 over the frozen ASCII tokenizer.
- `memories_fts_v2` — `unicode61 remove_diacritics 2` FTS5 over Unicode tokens,
  so Vietnamese tones fold and `đ` maps to `d`.
- `memory_meta` — `fts_epoch` (tokenizer contract) and `fts_v2_complete`
  (safe-to-read marker). Both are internal; missing rows mean "use legacy".

`refs` stay out of both FTS texts; the reference signal covers them separately.
A v2 read happens only when the table exists, the epoch matches, and the
completeness marker is `1`. A write open reconciles missing or old-binary rows
in 5,000-row chunks; a crash leaves the marker at `0`, so reads stay on the
legacy table instead of silently omitting notes.

`PRAGMA user_version` carries `MEMORY_SCHEMA_VERSION` and a compiled
required-structure manifest is checked at open. An older store that still has
every required column and the FTS table opens read-write (additive evolution).
A newer store, or one missing required structure, exits `5`.

Secondary indexes live in the same file (not in `schema/indexes.sql`): memory
writes are single-row transactions with no bulk-insert phase to post-index.

## Retrieval & Graph Propagation

Grounded in [Zero-Mem](https://arxiv.org/abs/2607.29377), `eg mem query` consumes **0 LLM tokens**. When `.engram/index.db` is available, query tokens seed a bounded breadth-first traversal along resolved `calls` edges (depth $\le 3$, fanout 64, decay $0.5^{depth}$, max 256 visited symbols) to co-retrieve notes anchored to downstream helpers.

Candidate notes are ranked by fusing min-max normalized features:

| Signal | Weight | Meaning |
|---|---|---|
| Lexical | 0.50 | Inverted FTS5 BM25 rank over key, tags, and body tokens |
| Reference | 0.30 | Token overlap between `--ref` and the note's `refs` |
| Recency | 0.20 | Bounded decay `1 / (1 + age_days)` |
| Graph | 0.25 | Call-graph decay $0.5^{depth}$ for notes referencing reachable symbols |

When no graph candidates exist or `.engram/index.db` is absent, the graph feature normalizes to a constant offset, preserving exact lexical/reference/recency ordering with zero failure risk.

Ties break on score, then `created_at`, `key`, and `rev`, so identical inputs
produce an identical order. `eg mem query` with no terms lists current notes by
recency instead of ranking.

Without `--global` the search covers the local store and the global store;
`--global` restricts it to the global store.

## Reference Currency & Citation Locks

Implementing the **Citation-Lock** and provenance model of [Agent Zero Memory](https://arxiv.org/abs/2608.29606), notes anchor to exact codebase entities through `--refs`. Every anchor represents an immutable provenance triple $\pi(x) = \langle \text{source}, \text{timestamp}, \text{raw evidence pointer} \rangle$.

At query time, anchors are dynamically verified against `.engram/index.db`:

- `[stale-ref]` — the anchor was modified or deleted after note creation.
- `[modified-ref]` — symbol modified at a later timestamp; re-auditing required.
- `[broken-ref]` — symbol no longer exists in the AST index.
- `eg mem touch <key>` refreshes the timestamp without creating a revision once
  the reader has re-audited the anchor.

`eg context <name>` co-retrieves gotchas and decisions that reference the
symbol, and `eg build` emits a stderr note when modified files touch guarded
areas.

## Lifecycle & Consolidation Guard

- **Consolidation Guard (Write)** — before persisting, `eg mem write` tokenizes body,
  tags, and refs through the memory tokenizer, queries the top 32 FTS candidates, and computes
  Jaccard similarity without English glue words. At $\ge 80\%$ similarity under another key,
  it appends an advisory hint suggesting `--supersedes <old>` ($659\text{ \mu s}$ overhead).
  The write always succeeds (exit 0).
- **Write** — append a revision, update both FTS rows, and commit in one
  transaction. `--touch` refreshes `created_at` (the last assertion date) when
  the content hash matches.
- **Touch** — `eg mem touch <key>` refreshes `anchored_at` (the last
  verification date) after the reader re-audits the anchor, without a revision.
- **Supersede** — `--supersedes <key>` marks the older key's current row and
  records the replacement without deleting history.
- **GC** — `eg mem gc` purges expired session notes and superseded revisions
  older than the 30-day retention window, including both FTS rows.
- **Reindex** — `eg mem reindex` rebuilds the derived Unicode index and
  `fts_version` markers. Notes are primary data and are never touched; a
  read-only store can be upgraded with this command without a write.
- **Rollback** — an older binary ignores `memories_fts_v2` and `memory_meta`
  and keeps using the legacy table. A repair runs on the next write open or
  through `eg mem reindex` after re-upgrading. `--legacy` forces legacy terms
  for diagnosis without changing the binary.
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
