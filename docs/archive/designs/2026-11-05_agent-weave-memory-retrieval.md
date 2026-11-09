# aw — Project Memory Retrieval

Status: Phase 1 approved and implemented (2026-11-05). Owner decisions on the
approval gates: OQ1 — `mem_write` + `mem_query` join the default MCP
`CORE_TOOLS` (get/delete/gc/export stay behind `--all-tools`); OQ6 — this
design replaces graft-learning Decision B1 (`aw note add`), and the
graft-learning doc's B1 status updated to `replaced` in the same landing.
This is a companion design to the canonical
[aw design](2026-10-31_agent-weave-rust-code-index.md); it adds one capability
(project memory) and leaves every existing index contract untouched.

- Author: Quy Doan (design compiled from the ZeroMem research digests, 2026-11-05)
- Date: 2026-11-05
- Storage baseline assumed: schema v9 (`src/extract.rs:21`), aw 0.4.0 working tree
- Prior art in tree: `docs/zeromem_apfel_architecture.md` (memory/reasoning plane split),
  `docs/archive/designs/2026-11-05_graft-learning-decisions.md` Decision B1 (superseded — see Status)
- OQ5 is resolved: `aw mem export` ships in
  Phase 1 (restore = idempotent `mem write` replay of the export).

## Problem statement

Agent sessions are amnesiac. Decisions settled in one session ("the resolver
keeps confidence tiers; AMBIGUOUS edges keep `dst_id` NULL"), recurring gotchas
("schema skew exits 5 both directions; never migrate in place"), and task
wrap-ups ("E1.2 gate passed after the `callee_empty_reason` fix") are re-derived
or re-pasted every session. The static `AGENTS.md` carries only durable,
hand-curated conventions; it cannot hold session-derived knowledge, and the
usage journal (`.aw/usage.log`) records *that* commands ran, not *what was
learned*. aw already owns the code graph; it has no place for the knowledge an
agent produces about that graph. The premise that this costs real re-work is
unmeasured today — Phase 1 records the usage evidence (write/query counts,
recall rate on a frozen oracle) before any tuning claims.

## The ZeroMem model — what it is, and what transfers

**Honest correction first.** The user-facing narrative "elephant → 12 goldfish
→ editor → resolver, iterated loops" does not describe the `ptaranat/zeromem`
repo. The research digests establish: zeromem is a single-pass deterministic
pipeline (ingest → profile → route → two views → fuse → closure → calibrate,
one pass per query, zero LLM calls), with no elephant/goldfish/editor/resolver
roles and no constant 12 anywhere in code or paper (`candidate_k = 24`,
`top_k = 5`, `window_size = 4`, `episode_beam = 4`, `window_beam = 8`; PPR
iterates `ppr_iters = 30` as arithmetic, not as an agent loop). The
elephant/goldfish vocabulary belongs to `google/elephant-goldfish-model` — an
SDD review loop where the count is caller-defined — not a memory system. The
four-role mapping below is therefore **this design's own placement on aw**,
informed by what the digests identify as transferable.

| Narrative role | aw placement | Mechanism |
|---|---|---|
| elephant | Long-term store: `.aw/memory.db` | Append-only SQLite note store; notes are the source of record, FTS is derived |
| 12 goldfish | Session lane, **no fixed count** | `scope='session'` rows with `expires_at`, purged by `aw mem gc`; the agent host's context window already is the short-term memory — aw only provides the durable spill-over lane |
| editor | Deterministic mechanics in core; synthesis only by the calling agent | Core: revision chains, supersession, dedup by content hash, gc. Text synthesis: the agent reads `aw mem query` results over MCP and writes back a consolidated note that supersedes its inputs — the core never generates text |
| resolver | Deterministic retrieval + conflict resolution | FTS5 BM25 lexical + reference-token overlap (graph link) + recency decay, min-max fused with fixed weights, deterministic tie-breaks; same-key revisions and `--supersedes` links resolve conflicts at result time |

**What transfers from zeromem** (per the digests, each verified applicable to aw):

1. Source of record + derived rebuild: only the note table is authoritative;
   `memories_fts` must remain a pure function of `memories` (enforced by a
   rebuild-and-compare test), the same invariant the build applies to
   `symbols_fts` (`schema.sql:65-68`).
2. Min-max normalize + linear fusion with fixed weights and deterministic
   tie-breaks — mirrors zeromem's two-view fusion (its `rho = 0.6`); aw fuses
   three signals with constants in one place.
3. Bounded evidence: zeromem adds at most 1 bridge + 1 neighbor per main hit
   (budget `2 × top_k`); aw's analog, co-reference expansion, is oracle-gated —
   deferred to Phase 3 and shipped only on a measured recall gap (the FTS text
   already covers key + tags + body, so only the ref-token path motivates it).
4. Supersession at result time: stale rows are excluded when ranking, not
   deleted — same shape as aw's staleness headers.
5. Idempotent replay via content dedup: zeromem dedups by `source_uuid`; aw
   dedups by `(key, FNV-1a content hash)` so a retried write is a no-op
   (`src/hash.rs` FNV-1a already exists).
6. Typed tuning constants in one place (zeromem's `Config` struct); aw has no
   config-file surface, so constants compile in — one editing point, no flags.

**What is deliberately not copied** (digest `doNotCopy`, each mapped to an aw
invariant): dense embeddings + ONNX (zero-network invariant), per-query PPR
power iteration (O(edges) hot path; traversal/SQL is enough at memory scale),
rebuild-everything-on-open (aw has incremental carry-forward and it is the
right trade-off for the index; memory needs no derived-graph rebuild at all),
in-place lazy migration (schema policy: exit 5, never migrate), background
writer/spool threads (no-daemon invariant; zeromem needs a spool because a
writer thread races the server — aw is synchronous in the invoking process),
chat-turn data model + heuristic NER (wrong unit of data; aw's unit is the
keyed note with explicit refs, not mined entities), multi-host integration
matrix (cross-surface gate discipline instead).

## Goals

- G-M1: an agent can persist a decision/gotcha/fact/task note in one call
  (`aw mem write` or MCP `mem_write`) and retrieve it by content in a later
  session (`aw mem query`), with byte-identical output on CLI and MCP.
- G-M2: retrieval is deterministic and zero-LLM: same DB + same query + same
  binary → same ranked output, with recorded tie-break rules.
- G-M3: writes are idempotent under replay; conflicts resolve through revision
  chains and explicit supersession, never silent overwrite.
- G-M4: every aw invariant holds unchanged — no new crate, no unsafe, writes
  confined to `db.rs`/`doctor.rs`/`journal.rs`, additive schema evolution with
  the exit-5 policy, no network/daemon/watcher/hook, full cross-surface sync.
- G-M5: `aw mem query` p95 < 20 ms hot (same budget class as `aw search`,
  recorded measurement in the canonical design's gate registry);
  `aw build` wall time unaffected in every phase.

## Non-goals

- No LLM, embeddings, vector search, PPR, or NER anywhere in aw's core; the
  "editor" synthesis role belongs to the calling agent, invoked over MCP.
- No automatic ingestion: aw never watches sessions, hooks prompts, or reads
  conversation transcripts. The agent decides what to write and composes the body.
- No semantic contradiction detection (deterministically impossible without a
  model); conflict resolution is key-based revision + explicit supersession.
- No cross-repo or user-global memory; the store is per-repo under `.aw/`.
- No in-place migration of `memory.db` — never `ALTER` a live store; version
  acceptance deliberately diverges from the index (read-forward, see Storage
  & schema), but the never-migrate rule is the same.
- No memory tables inside `index.db` (see Alternatives, Decision D1).
- No `--config` file, no per-run tunables; constants compile in.
- Note bodies are agent-authored prose (hard cap 8 KiB) — never verbatim source
  dumps; source evidence stays in the repository itself.

## Architecture

```
agent (CLI / MCP)                                  (no daemon, no hooks)
  | mem_write / mem_query / mem_get / mem_delete / mem_gc / mem_export
  v
src/service.rs  CommandRequest variants + TOOL_SPECS + CORE_TOOLS decision
  |
  +-- src/db.rs        ALL memory.db lifecycle: open/create, user_version gate,
  |                    inserts, revision supersession, FTS maintenance, gc purge
  |                    (write-confinement allowlist unchanged: db.rs/doctor.rs/journal.rs)
  +-- src/memory.rs    pure presentation + scoring over rows db.rs returns
  |                    (no write primitives, no connection opening; fused ranking,
  |                    output rendering, cap/limit)
  +-- src/journal.rs   one usage.log line per mem call (args: key/kind/scope/terms/
                       limit/body_len — never the body)
  +-- src/query.rs     shared staleness-free caveat/cap rendering helpers (reuse)
```

`.aw/` layout after the feature:

```
.aw/index.db      unchanged (schema v9; memory adds nothing to the index build in any phase)
.aw/memory.db     new — memory schema v1, own PRAGMA user_version
.aw/usage.log     gains tool names mem_write/mem_query/mem_get/mem_delete/mem_gc/mem_export
```

No persistent sidecars: `memory.db` runs the default rollback journal, so a
transient `memory.db-journal` exists only inside a write transaction — copying
or backing up `.aw/` between transactions cannot lose a committed note (no
`-wal`/`-shm` files, unlike a WAL regime).

### Component ledger (Cost / Enables / If-cut)

| Component | Cost | Enables | If cut |
|---|---|---|---|
| `memory.db` store (elephant) | ~150 lines in `db.rs`; second file + second `user_version`; `.aw/` deletion hazard documented | Durable knowledge survives `aw build --force`, `doctor --invalidate`, index schema skew | Feature does not exist — nothing else has value |
| FTS5 lexical layer | One virtual table + delta maintenance in the write transaction + purity test | Ranked content recall over bodies/keys/tags | Only exact `mem get <key>` remains; no content recall |
| refs + ref-overlap fusion | 2 columns (`tags`, `refs`); token overlap; ~40 lines | Vocabulary-gap recall via ref/tag tokens (query "authentication" finds a note that only says "login"); notes attach to code entities. Co-reference expansion (second-hop retrieval) is oracle-gated in Phase 3, not shipped blind | Lexical + recency only; recall drops exactly where notes use different words than the query |
| Fusion resolver | ~80 lines pure code in `src/memory.rs`; 3 constants | One deterministic ranking; no LLM, no embeddings | Ad-hoc ordering; measurement becomes meaningless |
| Revision chain (`key`, `rev`, `superseded_by`, `superseded_at`) | Insert-only writes; partial index | Idempotent replay, update history, safe agent-driven "editing", bounded retention | Destructive overwrites; a retried write corrupts state |
| Session lane + TTL + gc (goldfish) | `scope`/`expires_at` columns; `aw mem gc` as the explicit purge command (no build hook — `aw build` never touches `memory.db`) | Auto-forgetting for transient notes (read-time expiry filter) with disk reclamation on demand | Transient notes accumulate on disk or agents must remember to delete them |
| CLI + MCP surfaces + skill section | 6 actions × (CLI, `service.rs`, MCP, docs, skill) per the cross-surface gate | Both agent surfaces with byte parity; routing guidance | CLI-only; MCP-native agents (the primary consumers) excluded |
| Journal integration | 2 arg fields (`body_len`, tool names) | `aw usage --quality` shows memory hit/empty rates from day one | Feature utility unmeasured; G-M5 unverifiable |
| Frozen recall oracle (Phase 3) | One stdlib Python script + fixture | Weight tuning with evidence instead of guesses | Weights stay untested guesses forever |

## Storage & schema

`.aw/memory.db`, memory schema **v1** (independent `PRAGMA user_version`; the
index's v9 is untouched). Additive evolution with a version policy that
deliberately diverges from `index.db` (`schema.sql:5-7`): the index can
command a two-way exit 5 because `aw build` recreates it — memory cannot, and
it is primary data. The mechanism, made concrete:

- **"Structurally complete" is a compiled manifest, not an adjective.** The
  binary carries a required-tables + required-columns list for its own memory
  schema and checks it at open via `pragma_table_info` — the exact `probe_old`
  pattern (`src/db.rs:313` probe, `src/db.rs:338-352` required-columns loop).
  An older `memory.db` that passes the manifest opens for read and write.
- **Writes on an older DB omit the columns it lacks.** The INSERT column list
  is built from `PRAGMA table_info(memories)` at open (the `pragma_table_info`
  query at `src/db.rs:346` is the precedent). Because evolution is additive
  with defaults, an insert omitting a missing column is valid; the cost is
  stated plainly — values only the newer schema knows are not persisted until
  the store is rebuilt at the newer version.
- **Two distinct exit-5 messages.** Newer than the binary: `aw:
  memory-schema-mismatch: memory.db schema vN is newer than the binary expects
  (vM) — restore from an aw mem export backup or remove .aw/memory.db`. Older
  but failing the manifest: `aw: memory-schema-mismatch: memory.db schema vN
  is missing required structure for this binary — restore from an aw mem
  export backup or remove .aw/memory.db`.

The recovery path for unrecoverable states is the Phase 1 `aw mem export`
backup.

```sql
PRAGMA user_version = 1;

CREATE TABLE memories(
    id INTEGER PRIMARY KEY,
    key TEXT NOT NULL,              -- caller-chosen stable identity, <= 200 chars, no control chars
    rev INTEGER NOT NULL,           -- 1-based; new rev = old rev + 1 under one transaction
    kind TEXT NOT NULL,             -- closed set: decision | gotcha | fact | task
    scope TEXT NOT NULL DEFAULT 'durable',   -- durable | session (session accepted from Phase 2)
    session TEXT NOT NULL DEFAULT '',        -- AW_SESSION attribution (env, trimmed/capped 64, journal.rs:75 precedent)
    body TEXT NOT NULL,             -- agent-authored prose, hard cap 8192 bytes
    tags TEXT NOT NULL DEFAULT '',  -- whitespace-separated lowercase tokens, <= 16 tags x 32 chars
    refs TEXT NOT NULL DEFAULT '',  -- whitespace-separated qualnames / repo paths, <= 16 x 256 chars
    content_hash INTEGER NOT NULL,  -- FNV-1a 64 over (kind, body, tags, refs, scope) — idempotent replay
    source TEXT NOT NULL,           -- cli | mcp
    created_at INTEGER NOT NULL,    -- unix seconds
    expires_at INTEGER NOT NULL DEFAULT 0,   -- 0 = never; session rows carry the TTL deadline
    superseded_by INTEGER,          -- id of the replacing rev; NULL = current row
    superseded_at INTEGER NOT NULL DEFAULT 0 -- unix secs when superseded; 0 = current; the gc retention clock
);

CREATE UNIQUE INDEX memories_key_rev ON memories(key, rev);
CREATE INDEX memories_current ON memories(key, rev DESC) WHERE superseded_by IS NULL;
CREATE INDEX memories_expiry ON memories(expires_at) WHERE expires_at > 0;

CREATE VIRTUAL TABLE memories_fts USING fts5(text, tokenize='unicode61');
-- rowid = memories.id; text = mem_tokens(key) + mem_tokens(tags) +
-- mem_tokens(body) joined with spaces — the SAME tokenizer function the
-- query side uses (Resolution algorithm), pre-applied at insert so
-- unicode61's whitespace splitting suffices; camelCase in prose
-- (calleeEmptyReason) is already split at this point. refs are NOT in the
-- FTS text — the ref-overlap signal covers them separately (no double count).
-- Delta-maintained in the same transaction as every insert/supersede/delete.
-- Purity invariant: memories_fts is a pure function of memories (rebuild-and-compare test).
```

Design notes:

- **The FTS row is derived data.** Maintenance is a per-mutation delta
  (delete the superseded rowid, insert the new one) inside the write
  transaction; the purity test proves the invariant the same way fresh-vs-
  carried schema equality is proven for the index.
- **One tokenizer function, two consumers.** `mem_tokens(text)` = strip one
  trailing file extension (refs only — the bare token "rs" must not match
  every query), split camelCase/snake_case boundaries and non-alphanumeric
  runs, lowercase. It feeds (a) the FTS text stored at insert time and (b)
  the query side at search time (FTS query terms and the ref-overlap
  overlap). Query and stored text can never disagree because both pass
  through this single function; there is no second normalization rule in
  this design.
- **`refs` are opaque strings in Phase 1** — not validated against
  `symbols`. Validation is an open question (OQ4); validating would couple
  `mem write` to `index.db` state, which Phase 1 deliberately avoids: every
  `aw mem` command must work when no index exists, when the index is corrupt
  (exit 3 domain), and when the index is schema-skewed (exit 5 domain).
- **No config surface.** Caps and weights are constants in one module — the
  zeromem typed-config pattern minus a config file aw does not have.
- **Durability — a pragma regime new to this repo, not an index precedent**:
  the index build runs `journal_mode = OFF` / `synchronous = OFF` /
  `locking_mode = EXCLUSIVE` on its disposable `.new` sidecar
  (`src/db.rs:276-278`) and opens query connections `SQLITE_OPEN_READ_ONLY`
  (`src/db.rs:246`); no `busy_timeout` exists anywhere in `src/` today because
  a single build lock already serializes index writers. `memory.db` is primary
  data with concurrent agent writers, so it uses the default rollback journal
  (transient `memory.db-journal` inside a write transaction only),
  `synchronous = FULL` (durable per commit — stronger than the WAL+NORMAL
  pairing first drafted), and `busy_timeout = 5000`. No WAL sidecars
  (`-wal`/`-shm`) exist to lose when copying or backing up `.aw/`.

## Surfaces

New command family `aw mem` (one top-level subcommand with nested actions —
keeps the flat `Cmd` style at the top level, per-action `--help` for free):

| CLI | MCP tool | Contract |
|---|---|---|
| `aw mem write <key> --body <text> --kind <decision\|gotcha\|fact\|task> [--tags ...] [--refs ...] [--scope session --ttl <secs>]` | `mem_write` | Validates caps; identical `(key, content_hash)` → no-op success naming the existing rev; different content under the same key → new rev, previous rev marked `superseded_by` + `superseded_at`; exits 2 on cap/kind violations |
| `aw mem query <terms...> [--kind ...] [--scope ...] [--all] [--limit N]` | `mem_query` | `--all` (terms mutually exclusive) lists current rows by recency; otherwise the resolver ranks; default limit 10, clamp 1..100; every row prints `key rev kind scope created_at` + body, tagged `[superseded]` only with `--include-superseded` (Phase 2) |
| `aw mem get <key>` | `mem_get` | Current row for the exact key + revision history count; exit 1 unknown key |
| `aw mem delete <key>` | `mem_delete` | Hard-deletes all revs of the key (row + FTS entry), one transaction; exit 1 unknown key |
| `aw mem gc` | `mem_gc` | Phase 2: purges expired session rows and superseded revs whose `superseded_at` is older than the retention constant (30 days — the clock is the dedicated column, not `created_at`, so a rev superseded yesterday but created 40 days ago survives); before Phase 2 a no-op success; prints `expired=N removed, superseded=M removed, durable=K rows` |
| `aw mem export [--limit N]` | `mem_export` | Writes current revs as one JSON object per line — key, rev, kind, scope, session, body, tags, refs, source, created_at, expires_at — so a restore preserves TTL. Default (no `--limit`) is ALL current rows; `--limit N` takes the N most recent by `created_at` (deterministic). stdout carries ONLY the JSONL — the caveat goes to stderr (see the stream table), so restore pipelines parse stdout directly; the caller redirects to a file outside `.aw/` (write confinement: aw writes only under `.aw/`). Restore = replay `mem write` per line; for session rows `--ttl` is re-derived as `expires_at − created_at` (no extra restore flag), idempotent by content-hash dedup. Restore is content-exact, not a bit-exact replica: `created_at`/rev are re-stamped |

Shared response contract, reused verbatim from the house rules: first-line
caveat `aw: memory — agent-authored notes, candidates only, never proof of
current behavior`; cap rendering `showing N of M`; exit codes 0/1/2/3/4/5/6 as
today (`src/error.rs:3`), where 1 = no results / unknown key / no
`.aw/memory.db` (`aw: no-memory: no .aw/memory.db under <root> — write with aw
mem write`), 3 = corrupt `memory.db`, 4 = `SQLITE_BUSY` after `busy_timeout`
(`aw: locked: memory write in progress — retry`), 5 = memory schema skew
(`aw: memory-schema-mismatch: memory.db schema vN is newer than the binary
expects (vM) — restore from an aw mem export backup or remove .aw/memory.db`),
6 = internal/I/O, which covers `memory.db` open failures (e.g., unreadable
file). Exit 7 (parse-quality) is a build-phase class and unreachable from mem
commands.

Stream assignment (normative — G-M1 byte parity depends on it):

| Line | Stream | Commands |
|---|---|---|
| Caveat `aw: memory — agent-authored notes, candidates only, never proof of current behavior` | stdout, first line | write, query, get, delete, gc |
| `showing N of M` | stdout, last line | query (when capped) |
| `[aw] capped at N results (M total)` | stderr | query (when capped) |
| `expired=N removed, superseded=M removed, durable=K rows` | stdout | gc |
| JSONL data lines | stdout, nothing else on stdout | export — its caveat goes to stderr, so restore pipelines parse stdout directly |

**MCP composition** (approval gate OQ1): the default `CORE_TOOLS` list in
`src/service.rs` either gains `mem_write` + `mem_query` (read/write
are the daily loop; `mem_get`/`mem_delete`/`mem_gc`/`mem_export` stay behind
`--all-tools`, the mechanism `doctor` already uses for exclusion) or memory
stays behind `--all-tools` entirely. Both are one-line changes at the
chokepoint.

**Skill routing**: memory is not a navigation question and MUST NOT enter the
binding seven-command routing table (`skills/aw-first/SKILL.md:18-29`). A
non-mandatory "Project memory" section joins the skill's secondary-tool tier:
write triggers (a decision was settled; a gotcha cost real debugging time; a
task ends with unmerged conclusions), query triggers (session start; before
re-investigating a known area), and the trust note (notes are agent-authored
claims — verify against source before behavioral claims, exactly like the
existing trust contract at `skills/aw-first/SKILL.md:40-64`).

**Cross-surface gate checklist for each landing phase** (per `AGENTS.md` §4):
`src/cli.rs` + `src/main.rs` → `src/service.rs` (`CommandRequest`, `from_cli`,
`from_tool_args`, `TOOL_SPECS`) → `tests/cli.rs` + `tests/mcp.rs` →
`skills/aw-first/SKILL.md` → `README.md`, `docs/getting-started.md`,
`docs/usage.md`, `docs/cli-reference.md`, `docs/agent-integration.md`,
`CHANGELOG.md` → packaging: `schema_memory.sql` joins the `Cargo.toml` include
list; `tests/scripts/allowlist.txt` unchanged (zero new crates);
`tests/scripts/write_path_check.sh` passes unchanged (all write primitives stay
in `db.rs`).

## Memory write path

- **Who writes**: the calling agent (CLI or MCP) or the human. aw itself never
  originates a note.
- **When**: agent-invoked at decision-settling, post-debugging discovery, and
  task wrap-up. No daemon, no hook, no batch importer — the write happens in
  the same synchronous process that the agent spawned.
- **Dedup**: `content_hash` over `(kind, body, tags, refs, scope)` per key. A
  write whose hash equals the key's current rev is a successful no-op
  (returns the existing rev), so agent retries and tool-framework replays are
  safe — the same property zeromem gets from `source_uuid` dedup.
- **Conflict, same key**: different content → new revision in one transaction;
  the previous rev is marked `superseded_by` + `superseded_at` and excluded
  from default reads but retained for audit until the Phase 2 retention rule
  (gc purges revs whose `superseded_at` is older than 30 days). Nothing is
  silently overwritten.
- **Conflict, cross key**: `--supersedes <key>` (Phase 2) marks the other key's
  current rev superseded, recording that the new note replaces it. Semantic
  contradiction without a shared key is out of scope (Non-goals); the resolver's
  recency term plus the trust contract is the only defense, and it is a ranking
  defense, not a truth defense.
- **Session attribution**: `AW_SESSION` (env, trimmed, capped 64) fills
  `memories.session` — the mechanism the usage journal already uses for session
  attribution (`src/journal.rs:75`).
- **Concurrency**: rollback journal + `busy_timeout = 5000`. Every write runs
  under `BEGIN IMMEDIATE` — the write lock is taken BEFORE the current rev is
  read and held across read-compute-insert, so a second writer blocks on
  `busy_timeout` and either serializes after the winner commits (the normal
  outcome at these transaction sizes) or gets `SQLITE_BUSY` → exit 4. `rev`
  is never computed outside the transaction, so racing writers cannot hit the
  `UNIQUE(key, rev)` constraint; if it ever fires it is a defect — one
  in-transaction retry, then exit 6. No build lock is taken (memory writes
  never touch `index.db`). Phase 1 carries the suite's first concurrent-writer
  test — no precedent exists in `tests/cli.rs` today because one build lock
  serializes all current index writers.

## Resolution algorithm (pseudo-code)

```
mem_query(terms, kind?, scope?, limit):          # pure functions over db.rs-returned rows
  rows      = current_rev_rows(kind?, scope?)     # superseded_by IS NULL, not expired
  cand      = top CANDIDATE_CAP=256 by bm25(memories_fts, tokenize(terms))
              # mem_tokens(): the ONE tokenizer of this design — strip one
              # trailing file extension from refs (so ref "src/foo.rs" yields
              # src foo; the bare token "rs" must not match every query),
              # then split camelCase/snake_case boundaries and non-
              # alphanumeric runs, lowercase. The same function pre-tokenizes
              # the FTS text at insert time (Storage & schema), so query
              # terms and stored text always agree; deterministic
  if cand is empty and strict: return exit 1
  for r in cand:
      lex[r] = bm25_rank(r)                     # FTS5 rank, smaller is better → invert to [0,1]
      ref[r] = |tokens(terms) ∩ tokens(r.refs ∪ r.tags)| / |tokens(r.refs ∪ r.tags)|  # 0..1
              # defined as 0 when the ref/tag token set is empty (no division by zero)
  minmax_normalize(lex); minmax_normalize(ref)  # all-equal → 1.0 (zeromem convention; order-invariant)
  rec[r] = 1 / (1 + age_days(r.created_at))     # bounded monotone decay, no magic half-life
  score[r] = 0.5*lex + 0.3*ref + 0.2*rec        # W_LEX/W_REF/W_REC constants, one module
  # co-reference expansion is NOT in Phase 1: it is the zeromem bridge analog,
  # speculative until measured, and without an inverted structure on ref tokens
  # it needs a scan of current rows (schema v1 has no ref-token index — only
  # memories_key_rev, memories_current, memories_expiry, memories_fts). It is a
  # Phase 3 candidate, added only if the oracle shows a recall gap specific to
  # the co-reference query class (a second implicit weight would ship with it).
  result = sort by (score desc, created_at desc, key asc, rev desc)[:limit]
  render: caveat line, per-row `key rev kind scope created_at`, body indented,
          `showing N of M` + cap stderr line, exit 0 (1 when result empty)
```

Properties: deterministic (same DB + query + binary → same output); every
signal is bounded [0,1] before fusion; the single most expensive step is the
FTS5 query over ≤ candidate-cap rows (no expansion scan in Phase 1). Conflict
handling lives in the write path (revs, supersession) and the read filter
(`superseded_by IS NULL`, expired rows excluded) — the zeromem "supersession
at result time" pattern.

## Performance budget

| Operation | Budget | Basis |
|---|---|---|
| `aw mem query` p95, hot | < 20 ms (same class as `aw search` — the canonical design's gate registry records 0.74 ms mean at 2,201 files; process startup dominates below that) | Expected in-process 1-3 ms at ≤ 10k rows (FTS5 over 8 KiB-max bodies, candidate cap 256) |
| `aw mem write` p95 | < 10 ms | One prepared insert + FTS delta + one durable commit (`synchronous = FULL`, rollback journal) |
| `aw mem get/delete` | < 10 ms | Indexed exact-key lookups |
| `aw build` wall (all phases) | No measurable delta vs the 0.4.0 baseline | `aw build` never touches `memory.db` — no build-time purge hook (read-time expiry filtering makes one unnecessary); memory lives in a separate file |
| Storage | Average ~1 KiB per note including FTS; worst-case bound ~16.5 KiB/note (body 8192 + FTS copy + columns) → ~16 MB at 1,000 all-maximal notes, ~1 MB at 1,000 average notes | Body cap 8192 bytes; caps on tags/refs; the Phase 2 retention rule bounds superseded-rev history |
| Scaling line | 10k notes still < 5 ms in-process | Candidate cap bounds the fusion work; FTS5 handles millions of short rows |

No daemon, no background thread: every number above is synchronous in the
invoking process. Cold-start is process spawn (~3-5 ms), identical to every
existing subcommand.

## Rollout phases

### Phase 1 — durable store + resolver (stands alone)

Ships: `memory.db` v1 (all columns; CLI accepts `--scope durable` only,
`expires_at = 0`), `mem write/query/get/delete/export`, resolver (lexical +
refs + recency fusion + caps + caveats — no co-reference expansion), journal
integration, MCP parity, skill "Project memory" section, all docs, `aw doctor`
gains a `memory.db` presence/integrity/size line (read-only report), and the
`doctor --invalidate` semantics change (see Risks).

Acceptance criteria (all run before merge):

1. `cargo test --locked --test cli mem_` — byte-exact fixtures: write/query/get
   shapes, cap + `showing N of M`, empty result exit 1, missing-db exit 1,
   idempotent double-write (one row), revision supersession, kind/cap exit 2,
   `user_version` newer than the binary → exit 5, a structurally incomplete
   older schema → exit 5, truncated `memory.db` → exit 3, and mem commands
   work with no `index.db` present.
2. Export round-trip: `aw mem export` stdout parses as pure JSONL (zero
   non-JSON lines) and replayed as `mem write` calls into a fresh
   `memory.db` restores the same note SET — sorted-by-key equality of
   (key, kind, scope, body, tags, refs, derived ttl); `created_at`/rev are
   re-stamped by replay (documented restore semantics, not a bit-exact
   replica).
3. `doctor --invalidate` preserves `memory.db` and a `mem query` still works
   afterwards; the updated `--help` documents the new reset scope
   (contract-bump entry).
4. Concurrent writers — both cases specified: (a) two processes writing the
   SAME key with different bodies — the loser either lands as the next rev
   after the winner commits (serialization under `busy_timeout`) or exits 4,
   never a `UNIQUE(key, rev)` failure; (b) two processes writing different
   keys — both succeed. The suite's first concurrent-writer test (the index
   has none: one build lock serializes it).
5. `cargo test --locked --test mcp mem_` — MCP results byte-identical to CLI
   for `mem_write`/`mem_query`/`mem_get`/`mem_delete`/`mem_export`; journal
   rows carry `surface:"mcp"`.
6. FTS purity test — rebuild `memories_fts` from `memories` in a test
   transaction and assert equality (the derived-table invariant).
7. `bash tests/scripts/write_path_check.sh` and
   `bash tests/scripts/allowlist_check.sh Cargo.lock` — both pass unchanged.
8. `hyperfine --warmup 3 'aw mem query <frozen-term>'` against a generated
   1,000-note fixture — p95 < 20 ms (target; measured value recorded in the
   phase evidence, command attached).
9. Recall smoke: a frozen 10-query/10-expected-key fixture in
   `tests/fixtures/memory/` reaches recall >= 0.8 (formalization is Phase 3).
10. `aw usage --quality` shows `mem_write`/`mem_query` calls and the
    `mem_query` empty rate; journal lines contain `body_len`, never body text.

### Phase 2 — lifecycle: session lane, TTL, gc, supersession links

Ships: `--scope session --ttl <secs>` (default TTL pending OQ2),
`aw mem gc` (purges expired session rows and revs whose `superseded_at` is
older than the 30-day retention constant, plus their FTS entries), CLI/MCP
`--supersedes <key>`, `aw mem query --include-superseded`, docs + skill update
in the same commit. No build-time purge hook: `aw build` never touches
`memory.db` in any phase — a hook would couple the two lifecycles (the same
reason memory-in-index.db is rejected in Decision D1-B) and its ordering
against a parse-quality abort (exit 7) would be unspecified, while read-time
expiry filtering already makes expired rows invisible; gc stays an explicit
agent- or human-invoked command whose only added value is disk reclamation.

Acceptance criteria:

1. Unit tests with an injected clock: expiry boundary correctness; `aw mem gc`
   prints `expired=N removed, superseded=M removed, durable=K rows` and
   hard-deletes rows + FTS entries.
2. `--supersedes` chain test: target key's current rev becomes superseded,
   link visible via `--include-superseded`, unknown target exits 1.
3. Retention test: revs whose `superseded_at` is older than 30 days are purged
   by gc; a rev superseded yesterday but created 40 days ago survives (the
   clock is `superseded_at`, not `created_at`); newer revs survive.
4. Determinism test: `mem_query` output byte-identical across two runs and
   across a fresh-process replay of the same write sequence (idempotent replay).

### Phase 3 — measurement and tuning

Ships: `tests/scripts/mem_recall.py` (stdlib Python only, per the standing
experiment-plan constraint) over a frozen oracle (queries → expected keys,
with negative controls), a Wilson-interval recall report, and — only if the
bar fails — a recorded weight retune of `W_LEX/W_REF/W_REC` with before/after
numbers. Co-reference expansion is re-evaluated here: it re-enters the
resolver only if the oracle shows a recall gap specific to the co-reference
query class. Extends `docs/evaluation.md`.

Acceptance criteria:

1. `python3 -I tests/scripts/mem_recall.py --aw <bin> --fixture <frozen>` —
   recall >= 0.8 with Wilson 95% interval reported; the negative control
   (shuffled term-to-key mapping) must fail the gate.
2. Any weight change lands with measured before/after recall in the evidence
   file; constants stay in the single module.
3. Feature-retention check from the journal (`aw usage --quality`, which
   already splits per tool): write/query rates over the measurement window
   are recorded; a window with zero `mem_write` calls makes the feature a
   recorded drop candidate rather than a permanent fixture.
4. Co-reference expansion lands only with the oracle evidence above; absent
   evidence it stays cut.
5. No cross-surface change is expected; if any output shape changes, the full
   cross-surface gate checklist re-runs.

## Risks

| Risk | Severity | Mitigation |
|---|---|---|
| Two destruction paths hit memory: `rm -rf .aw/` (the documented one-command exit) and `aw doctor --invalidate`, whose current contract deletes the whole `.aw/` directory (`src/cli.rs:211-214` help text: "Delete the entire .aw directory and all its contents"; `src/doctor.rs:25` `remove_dir_all`) — the graft-learning Decision B1 already recorded this exact hazard for primary data ("`aw doctor --invalidate` would discard primary data, not only a rebuildable index") | High (data loss, silent) | (1) `aw mem export` ships in Phase 1 (restore = idempotent write replay); (2) fixed disposition: `doctor --invalidate` deletes everything under `.aw/` EXCEPT `memory.db` — a deliberate semantics change (reset the derived index and journals, keep primary memory), documented in `--help` + CHANGELOG with a Phase 1 acceptance test; `rm -rf .aw/` remains the human's own action, documented prominently in docs + CHANGELOG |
| `memory.db` corruption is unrecoverable (memory is not derivable from source, unlike the index) | High, low probability | Rollback journal + `synchronous = FULL` (durable per commit), `aw doctor` integrity line (Phase 1), exit-3 mapping, and the Phase 1 `aw mem export` backup |
| Stale or wrong notes poison later sessions | Medium | Recency ranking, revision supersession, `mem delete` escape hatch, and the output caveat "candidates only, never proof" — the same trust posture as navigation rows (`skills/aw-first/SKILL.md:40-64`); verification against source stays mandatory |
| Ref rot (symbols renamed, paths moved) makes `refs` stale | Medium | Phase 1 stores refs unvalidated and matches them as tokens (no index dependency); optional write-time validation via the index is OQ4 |
| Second DB doubles the skew/corruption surface | Medium | Exit-3 corrupt handling shared; version policy deliberately diverges (read-forward for older structurally complete schemas — Storage section); one owner module (`db.rs`) for both lifecycles; the two pragma regimes are documented (index build = disposable sidecar OFF/OFF, memory = FULL durability) |
| Concurrent writers (two agent sessions) — a regime with no precedent in the repo (one build lock serializes all index writers) | Low | Rollback journal + `busy_timeout = 5000` + exit 4 on exhaustion; Phase 1 acceptance carries the suite's first concurrent-writer test; no build-lock coupling |
| Journal privacy leak through bodies | Medium | Bodies are never journaled; `args` carry `key/kind/scope/terms/limit/body_len` only — extends the existing "names/paths/scalars, never contents" rule |
| Unbounded growth: write-time caps bound note size but nothing bounded row count — superseded durable revs were "retained for audit" indefinitely, and the same gotcha written under two different keys is not detected (deterministic cross-key dedup is impossible without a model; consolidation stays the agent's job, Decision D3-A) | Medium | `aw mem gc` (Phase 2) purges revs whose `superseded_at` is older than 30 days and prints durable/superseded counts (folded into gc — no extra `mem stats` subcommand); expired rows are read-invisible regardless; `aw usage --quality` measures per-tool write/query rates (the journal summary already splits per tool); Phase 3 carries the recorded feature-retention check |
| Cross-surface drift | Medium | The gate checklist above; the `service.rs` chokepoint generates MCP schemas from `TOOL_SPECS` (rendered by `src/mcp.rs`), so parity is structural, not copy-paste |

## Alternatives considered

Presentation rule: mechanisms + trade-offs + failure conditions only, no
ranking — the owner chooses.

### Decision D1 — where the memory store lives

- **A. Separate `.aw/memory.db`** (the design's mechanism): own
  `user_version`, own pragmas, untouched by index builds. Trade-off: a second
  file, a second version policy, ~150 lines in `db.rs`; memory still dies with
  `rm -rf .aw/`. Failure condition: none unique to this option beyond the
  shared `.aw/` hazard.
- **B. Memory tables inside `index.db`, carried forward per build**: one file,
  one lock, one version. Trade-off: every build must copy memory rows into the
  fresh DB (O(memory) per build), and `aw build --force`, `doctor --invalidate`,
  `rm -rf .aw/`, and every v9→v10 exit-5 window take memory down together with
  the index. Failure condition: a schema-skewed or corrupt index makes notes
  unreadable exactly when the agent needs them (mid-migration, mid-debugging).
  The no-build-hook rule (Phase 2) keeps the chosen option consistent with
  this rejection: no phase couples memory to the build.
- **C. Append-only JSONL under `.aw/`** (usage-log pattern): simplest writes,
  trivially diffable. Trade-off: no FTS5 (the resolver degenerates to a
  full-scan filter), no SQL-level revision/dedup, updates rewrite the file.
  Failure condition: O(n) scans per query as notes accumulate; the resolver's
  deterministic ranking becomes hand-rolled code.

### Decision D2 — the "12 goldfish" short-term slots

- **A. Session lane with TTL, no slot count** (the design's mechanism):
  `scope='session'` + `expires_at`, gc purges. Trade-off: forgetting depends on
  the agent or human running the explicit `aw mem gc` (no build hook —
  Decision D1-B consistency), not on slot arithmetic. Failure condition: an
  agent that never runs gc pays disk for expired rows; retrieval is unaffected
  because the resolver filters expired rows at read time.
- **B. Fixed 12-slot ring per session**: deterministic eviction at exactly 12
  items. Trade-off: requires session identity and stateful bookkeeping on every
  write — a session lifecycle aw does not have without a daemon; and the digests
  show no source for 12 (zeromem has `candidate_k 24`/`top_k 5`; the EGM count
  is caller-defined). Failure condition: eviction discards a note the agent
  still needs, keyed to an arbitrary constant.
- **C. Durable-only, no session lane**: simplest schema. Trade-off: transient
  notes ("tried X, it failed") either pollute durable memory or require agents
  to remember deletion. Failure condition: durable memory fills with expired
  scratch work the recency term keeps surfacing.

### Decision D3 — who plays "editor" (merge/summarize)

- **A. Deterministic mechanics in aw + agent-side synthesis over MCP** (the
  design's mechanism): core does revs/dedup/supersede/gc; the calling agent
  reads query output and writes a consolidated superseding note. Trade-off:
  consolidation quality depends on the calling agent; no automatic rollup.
  Failure condition: agents never consolidate — old revs accumulate (bounded
  by the Phase 2 retention rule; visible via `--include-superseded`) and
  same-gotcha notes under different keys are never detected, because
  deterministic cross-key duplicate detection is impossible without a model;
  the journal's per-tool write-rate measurement plus the Phase 3 retention
  check is the feedback loop, not a fix.
- **B. Core-side deterministic "summarization"** (e.g., merge tags, truncate
  bodies): no LLM, automatic. Trade-off: lossy text surgery on agent-authored
  prose inside aw. Failure condition: garbled or truncated notes become the
  source of record — violating the raw-note principle the ZeroMem digest marks
  as zeromem's core correctness property.
- **C. External tool post-processes `memory.db` directly**: full summarization
  power outside aw. Trade-off: writes outside the `db.rs`/`doctor.rs`/
  `journal.rs` confinement and outside aw's audit trail. Failure condition:
  the write-confinement invariant (checked by `tests/scripts/write_path_check.sh`)
  becomes unenforceable; schema evolves without aw's version gate.

### Decision D4 — resolver shape

- **A. Linear min-max fusion of (lexical, ref-overlap, recency), fixed weights,
  deterministic tie-breaks** (the design's Phase 1 mechanism); bounded co-ref
  expansion is a Phase 3 candidate gated on oracle evidence. Trade-off: three
  constants to justify (Phase 3 owns evidence). Failure condition: a bad
  weight distribution misranks — detectable by the frozen oracle, correctable
  by retune.
- **B. BM25 only + recency tie-break**: least code. Trade-off: notes sharing a
  code entity with the query context rank no better than prose noise; the
  graph-link differentiation vs "grep my own notes" disappears. Failure
  condition: vocabulary-gap queries (query words absent from the note body)
  return nothing.
- **C. Two-view profile-then-route like zeromem** (query profile regex → route
  between views → fuse): the faithful zeromem transplant. Trade-off: a query
  classification subsystem for a store measured in hundreds of rows — profile
  signals (answer type, temporal cues, session boundaries) have no analog in
  keyed notes. Failure condition: misrouted queries skip a view; the added
  surface must itself be tested for determinism with no measured benefit yet.

## Open questions

- OQ1 (RESOLVED, owner 2026-11-05): `mem_write` + `mem_query` join the
  default `CORE_TOOLS` in `src/service.rs`; `mem_get`/`mem_delete`/`mem_gc`/
  `mem_export` stay behind `--all-tools`.
- OQ2 (owner): default TTL for `--scope session` when `--ttl` is omitted —
  candidate values 24 h / 7 days, or mandatory explicit `--ttl`.
- OQ3 (owner): `kind` taxonomy — keep the closed set
  (`decision | gotcha | fact | task`) validated with exit 2, or accept arbitrary
  kinds and grow the set additively per release.
- OQ4 (owner): `refs` validation — strict (resolve each ref against `symbols`
  at write time, exit 2 on unknown; couples `mem write` to `index.db` state)
  or free-form token matching only (current design).
- OQ6 (RESOLVED, owner 2026-11-05): this design replaces graft-learning
  Decision B1 (`aw note add`); B1's status line in
  `docs/archive/designs/2026-11-05_graft-learning-decisions.md` reads `replaced` in
  the same landing as Phase 1. (OQ5 from the first draft is resolved — see
  the Status block: `aw mem export` ships in Phase 1.)
