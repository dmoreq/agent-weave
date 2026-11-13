# Hindsight-Derived Memory Capabilities

- Author: Quy Doan
- Status: Implemented (2026-11-12; Gate 2 cycles 1-3 complete; baselines recorded in `docs/baselines/memory-parity-*.json`)
- Date: 2026-11-12
- Baseline binary: `eg` 1.0.4 (`Cargo.toml`)
- Predecessor: `docs/designs/2026-11-12_concepts-brief_hindsight-memory-parity.md`
- Review report: `docs/reports/2026-11-12_goldfish-review_hindsight-memory-parity.md`

## Overview

This spec fixes one measured defect and adds three deterministic memory capabilities that Hindsight models with an LLM pipeline:

- Unicode-aware retrieval, so unaccented Vietnamese queries find accented notes without consonant-skeleton collisions.
- `--since` / `--until` time windows on `mem query` and `mem export`.
- `--explain` causal labels for ranked results.
- `--local` project-scope isolation and an advisory `mem export --scan`.

Hindsight (vectorize-io/hindsight, MIT) is a server-based agent-memory system that extracts facts with an LLM and retrieves with vectors, BM25, a graph, and time filters. This spec borrows only its information model: time as a retrieval axis, provenance for a match, and store isolation. No LLM call, embedding, network path, daemon, dependency, or further `MEMORY_SCHEMA_VERSION` bump is introduced. `MEMORY_SCHEMA_VERSION` is already 2 in 1.0.4; this spec adds structures without changing it.

"Parity" means capability parity for the four items above, not parity with Hindsight's LLM pipeline. Gate 2 is the five-lens Goldfish review required by `doubt-driven-development`.

## Terms

| Term | Meaning |
|---|---|
| Current row | A `memories` row with `superseded_by IS NULL` and `expires_at = 0` or in the future (`src/db/memory_query.rs:24-42`) |
| Legacy FTS | `memories_fts`, tokenized by `mem_tokens_legacy` (ASCII only); the table 1.0.4 reads and writes |
| v2 FTS | `memories_fts_v2`, tokenized by `mem_tokens` (Unicode) with `unicode61 remove_diacritics 2` |
| `fts_version` | Per-row marker: `0` = written by an older binary, `2` = indexed in both tables by this binary |
| `fts_epoch` | Tokenizer contract version; `2` in this spec. A mismatch forces a full rebuild |
| Complete | `memory_meta.fts_v2_complete = "1"`; v2 is safe to read |
| Repair | Idempotent reconciliation of both FTS tables against current `memories` rows |
| Purity | Each FTS table has exactly one row per current `memories` row, with that table's tokenizer output |
| Oracle | A frozen note/query fixture with expected keys and a negative control (`tests/scripts/mem_recall.py`) |
| BM25 | SQLite FTS5 relevance rank; smaller is better |
| CUJ | Critical User Journey |

## The Problem

`mem_tokens` keeps only `is_ascii_alphanumeric` (`src/memory/tokens.rs:78`). `Bộ nhớ dự án` is indexed as `b nh d n`, so the ASCII query `bo nho` (tokens `bo nho`) cannot match it. Vietnamese words that differ only in tone or vowels also collapse to the same consonant skeleton and collide. Measured: `eg mem query "bo nho"` returns `no-results` after writing the note.

Two further defects block the planned work:

- The secret detector substring-matches `sk-` (`src/memory/note.rs:80-85`). Measured: `task-1 pending` and `risk-averse reviewer` both exit 2.
- `mem query` cannot filter by `created_at` (`schema/memory.sql:25`), so handoffs cannot be bounded.

The frozen memory baseline is 20/20 recall@5 with a 0.050 negative control (`docs/evaluation.md:285`). This work is not a recall regression fix; it closes measured coverage gaps at zero LLM cost.

## Goals

- A bilingual oracle (10 notes, 10 queries, created and hashed in Phase 0 before any code change) scores 10/10; an unaccented query matches an accented note; a bare consonant query (`b`, `nh`) does not; the negative control stays below the gate threshold.
- `--since` / `--until` apply before candidate caps. A fixture with 300 out-of-window matches and one in-window match returns the in-window note.
- `--explain` adds exactly one `why:` line per shown ranked note. Without the flag, no `why:` line appears and the row shape is unchanged. Labels are limited to `lexical match`, `ref <path>`, `tag <token>`, `graph depth <N>`, and `age <N>d`.
- `--local` returns only project-store rows; a missing project store exits 1 with the existing no-store error.
- `mem export --scan` reports exported notes containing suspected secrets on stderr, exits 0, and leaves stdout byte-identical.
- A 1.0.4-created store opens, reads, and writes after migration without a `user_version` change; an emulated 1.0.4 row with `fts_version = 0` is repaired by one `mem write`/`eg mem gc`/`eg mem reindex`.
- At 5,000 notes: import < 1 s, full repair < 1 s, and `mem write` p95 no more than 1 ms above the recorded 1.0.4 baseline. `tests/scripts/mem_scale.py` records all three and exits 1 on regression.

## Non-Goals

- Persisting corroboration edges, trust scores, or duplicate detection.
- LLM extraction, reflection, embeddings, vector search, servers, banks, disposition traits, or config files.
- MCP resources, `eg brief`, trigram fallback, or module digests.
- Changing `created_at` mutability, revision semantics, retention, or GC.
- Changing the journal envelope, rotation, or `eg audit` aggregation; additive `args` keys are in scope.
- Blocking export on a suspected secret.
- Version bump, release, or tag.

## Decisions

| ID | Decision | Rationale |
|---|---|---|
| D1 | Dual FTS tables; new binaries query v2 only when complete; per-row `fts_version` repair; no `user_version` bump | Eliminates skeleton collisions, keeps 1.0.4 compatibility, and repairs mixed-version writes. A union arm was rejected because BM25 is not comparable across tokenizers and the legacy arm reintroduces the collision. |
| D2 | Corroboration cut | Ref-sharing is not independent evidence and depends on candidate cap, limit, window, and store scope. |
| D3 | Duplicate detection cut | `eg audit` forbids keys; the write-time advisory hint remains. |
| D4 | `created_at` = last assertion (refreshed by `mem write --touch`); `anchored_at` = last verification (refreshed by `mem touch`) | Avoids a behavior change and a schema migration; windows mean "asserted since". |

## Open Questions

None.

## Critical User Journeys

### CUJ 1: Unaccented query finds an accented note

- Given: `eg mem write viet --body "Bộ nhớ dự án lưu quyết định kiến trúc"`, which created v2 through the write path
- When: `eg mem query "bo nho"`
- Then: exit 0 and the key `viet` appears; `eg mem query "bộ nhớ"` returns the same key

### CUJ 2: Consonant query does not collide

- Given: the same note
- When: `eg mem query "nh"` or `eg mem query "b"`
- Then: exit 1 with the caveated no-results error; the note is not returned

### CUJ 3: Time-bounded handoff

- Given: notes asserted on 2026-11-10 and 2026-11-12
- When: `eg mem query --all --since 2026-11-11`
- Then: only the 2026-11-12 note appears; `--until` is exclusive; a window with no rows exits 1

### CUJ 4: Explain without scores

- Given: a ranked query with the index present
- When: `eg mem query resolver --explain`
- Then: each shown note carries one `why:` line with only fixed labels, no bm25 value, normalized score, weight, or body text

### CUJ 5: Project scope only

- Given: local and global stores, both current
- When: `eg mem query --local deploy`
- Then: no `[global]` row appears; a missing project store exits 1 with the existing no-store error

### CUJ 6: Legacy secret is reported, export still succeeds

- Given: a current note seeded outside the validator with `sk-` plus 24 characters
- When: `eg mem export --scan > backup.jsonl`
- Then: stdout is unchanged valid JSONL, stderr names the row and field, and exit is 0

### CUJ 7: Older binary after migration

- Given: a migrated store and an emulated 1.0.4 write (legacy text only, `fts_version = 0`)
- When: the next new-binary write open runs, or the operator runs `eg mem reindex`
- Then: the row is indexed in v2 and an unaccented query finds it; the emulated 1.0.4 query still works through the legacy table

### CUJ 8: Partial v2 never serves reads

- Given: a store whose v2 table exists but `fts_v2_complete = "0"`
- When: `eg mem query "bo nho"`
- Then: the query uses the legacy table with legacy terms and returns the same results as 1.0.4; no row is silently omitted

## Technical Plan

```text
write (new): note -> mem_tokens_legacy -> memories_fts     ; fts_version = 2
                  -> mem_tokens        -> memories_fts_v2
write (1.0.4): note -> mem_tokens_legacy -> memories_fts   ; fts_version = 0 (default)
write open:  meta -> epoch/tokenizer check -> repair current rows in 5,000-row chunks
             -> set fts_v2_complete = 1
read:        complete ? (mem_tokens, v2) : (mem_tokens_legacy, legacy)
             --legacy forces the legacy pair
```

### P0 — Shared date math

`src/time.rs` (Create) holds `pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32)` and `pub(crate) fn days_from_civil(y: i64, m: u32, d: u32) -> i64`, moved from `src/audit/render.rs:271`. `src/audit/render.rs` imports `civil_from_days`; `src/lib.rs` adds `mod time;`.

### P1 — Secret detector

`src/memory/secret.rs` (Create):

```rust
pub(crate) fn suspected_secret(text: &str) -> Option<&'static str>
pub(crate) fn contains_suspected_secret(text: &str) -> bool
```

- Boundary = start of text or a preceding character that is not alphanumeric.
- Fixed precedence and labels: `openai-key`, `github-token`, `aws-access-key`, `jwt`, `private-key`, `slack-token`, `bearer-token`, `database-url`.
- `openai-key`: `sk-` at a boundary, then a maximal run of at least 16 `[A-Za-z0-9_-]`.
- `github-token`: `ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_` + at least 20 alphanumerics; or `github_pat_` + at least 22 `[A-Za-z0-9_]`.
- `aws-access-key`: `AKIA` or `ASIA` at a boundary, then exactly 16 uppercase alphanumerics, and the next character is not uppercase alphanumeric.
- `jwt`: a token starting `eyJ`, three dot-separated segments, each at least 10 `[A-Za-z0-9_-]`.
- `private-key`: `-----BEGIN ` present and `PRIVATE KEY-----` later.
- `slack-token`: `xoxb-`, `xoxp-`, `xoxa-`, `xoxr-`, or `xoxs-` + at least 10 `[A-Za-z0-9-]`.
- `bearer-token`: case-insensitive `bearer` + one space + at least 20 `[A-Za-z0-9._~+/-]`.
- `database-url`: `://` with an authority containing a password separator `:` immediately before `@` before the first `/`.
- Accepted false-negative classes, stated in the module doc: high-entropy secrets without a known prefix, AWS secret access keys, and secrets split across fields.
- `note.rs` keeps every validation call site (key, body, tags, refs, import) calling the wrapper; `SUSPECTED_SECRET_MSG` is unchanged.
- Test vectors are built at runtime (`format!("ghp_{}", "a".repeat(36))`) so `tests/scripts/hygiene_check.sh` does not flag the source.
- The existing `memory::tests::secret_rejection_covers_key_body_tags_refs` is replaced: `sk-12345`, `ghp_abcdef`, and `eyJhbGciOi` are now accepted; full-length vectors are rejected.

### P2 — Timestamp semantics in every surface

Canonical wording: `created_at` = last assertion (refreshed by `mem write --touch`); `anchored_at` = last verification (refreshed by `mem touch`).

| File | Change |
|---|---|
| `src/cli.rs:319-320` | Touch help becomes "Refresh anchored_at (last verified) without revision churn" |
| `src/service/tools.rs:295-298` | `mem_write.touch` description: "Refresh created_at (last asserted) when content_hash matches" |
| `src/service/tools.rs:363` | `mem_touch` description: "Refresh anchored_at (last verified) without revision churn" |
| `src/service/mem_ops.rs:75-76` | `run_mem_touch` doc: "Refresh `anchored_at`, leaving `created_at` untouched" |
| `docs/cli-reference.md:253` | `mem touch` row: anchored_at wording |
| `skills/engram-first/SKILL.md:94` | anchored_at wording |
| `docs/architecture/memory-store.md` Lifecycle | Add the two definitions |
| `schema/memory.sql` header | "user_version 2; additive v2 FTS structures; no further bump" |

`docs/usage.md:239` and `docs/cli-reference.md:218` already use the correct wording; no change.

### B1 — Dual FTS with completeness marker

`src/memory/tokens.rs` (Modify):

- `mem_tokens(text)`: keep `c.is_alphanumeric()`, map `đ`/`Đ` to `d`, split camelCase with Unicode case predicates, split letter/digit runs, lowercase with `to_lowercase()`, then keep only characters that are alphanumeric. FTS metacharacters cannot survive.
- `mem_tokens_legacy(text)`: exact 1.0.4 body.
- `mem_fts_text` / `mem_fts_text_legacy`; `mem_query_terms` / `mem_query_terms_legacy`.
- `mem_ref_tokens` / `mem_ref_tag_tokens` use `mem_tokens`.
- Test `mem_tokens_legacy(s) == mem_tokens(s)` for ASCII inputs.
- Exact vectors: `mem_tokens("Bộ nhớ dự án") == ["bộ","nhớ","dự","án"]`; `mem_tokens("đường") == ["dường"]`; `mem_tokens_legacy("Bộ nhớ") == ["b","nh"]`.

`schema/memory.sql` (Modify):

```sql
CREATE TABLE IF NOT EXISTS memory_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE VIRTUAL TABLE memories_fts_v2 USING fts5(text, tokenize='unicode61 remove_diacritics 2');
```

- `memories` gains `fts_version INTEGER NOT NULL DEFAULT 0` as the last column, matching `ALTER TABLE` order.
- Fresh stores create both tables and `memory_meta`; `fts_version` defaults to 0 and the new write path sets 2.
- `memories_fts_v2`, `memory_meta`, and `fts_version` are NOT added to `MEMORY_REQUIRED`; `memories_fts` remains the only required FTS object.
- Add `CREATE INDEX memories_created ON memories(created_at)` and the partial index `CREATE INDEX memories_pending ON memories(id) WHERE fts_version < 2`.

`src/db/memory_store.rs` (Modify):

- `const MEM_FTS_EPOCH: &str = "2";`
- `pub(crate) fn memory_fts_v2_usable(conn: &Connection) -> rusqlite::Result<bool>`: probe `object_exists_result` for `memories_fts_v2` and `memory_meta` before any `SELECT`; true only when both exist, `memory_meta.fts_epoch = "2"`, and `memory_meta.fts_v2_complete = "1"`. A missing table is false; a probe or read error propagates (exit 6).
- `pub(crate) fn ensure_memory_fts_v2(conn: &Connection, force: bool) -> Result<u64>` runs at the end of `open_memory_conn_path` (all five write opens) after `migrate_memory_schema`, and from `eg mem reindex` with `force = true`:
  1. Create `memory_meta` if absent.
  2. If `force`, or the epoch differs, or `sqlite_master.sql` for v2 lacks `remove_diacritics 2`: set `fts_v2_complete = "0"`, `fts_epoch = "2"`, `DROP TABLE IF EXISTS memories_fts_v2`, create it, `UPDATE memories SET fts_version = 0`, and run a full reconcile.
  3. Reconcile loop, 5,000 rows per `BEGIN IMMEDIATE` transaction. If any pending row is found, the first chunk transaction sets `fts_v2_complete = "0"` before inserting; a crash or error mid-repair therefore leaves reads on the legacy table:
     - Select current rows needing work: `SELECT id, key, tags, body FROM memories WHERE superseded_by IS NULL AND (expires_at = 0 OR expires_at > ?1) AND (fts_version < 2 OR id NOT IN (SELECT rowid FROM memories_fts) OR id NOT IN (SELECT rowid FROM memories_fts_v2)) ORDER BY id LIMIT 5000`.
     - For each row: `INSERT OR REPLACE INTO memories_fts(rowid, text) VALUES (?1, ?2)` with `mem_fts_text_legacy`; `INSERT OR REPLACE INTO memories_fts_v2(rowid, text) VALUES (?1, ?2)` with `mem_fts_text`; `UPDATE memories SET fts_version = 2 WHERE id = ?1`.
     - On a full reconcile, first delete rows from both FTS tables that are not current `memories` ids (orphan sweep).
     - Commit each chunk; a failure rolls back only that chunk and returns exit 6. The next open resumes.
  4. When the selection is empty, set `fts_v2_complete = "1"` in the same transaction as the final chunk. On a store with no pending rows, set it in the first transaction.
  5. Emit progress to stderr only when more than one chunk ran: `eg: memory: reindexed <n> rows so far`. The command `eg mem reindex` prints exactly `reindexed <n> rows` to stdout; the automatic open repair never writes to stdout.
- Concurrency: `BEGIN IMMEDIATE`; `CREATE VIRTUAL TABLE IF NOT EXISTS`; lock timeout maps to exit 4 with the existing retry text.
- If the meta table or v2 table is absent (read-only store), the query path uses legacy; no write is attempted.

`src/db/memory_write.rs` (Modify):

- `mem_fts_text_legacy(key, tags, body)` goes to `memories_fts`; `mem_fts_text(key, tags, body)` goes to `memories_fts_v2`.
- `fts_version = 2` is added to the single `insert_revision` column list and bind list (`src/db/memory_write.rs:275-292`); `try_write_revision` and `memory_write_batch` both inherit it. The two-attempt `UNIQUE(key, rev)` retry loop is unchanged; both FTS inserts run inside each attempt.
- Supersede deletes both FTS rows; `mem delete` and `mem gc` delete both, including the GC `DELETE` of superseded rows.
- A v2 insert failure rolls back the note transaction (exit 6).

`src/db/memory_query.rs` (Modify):

- `memory_fts_candidates(conn: &Connection, match_query: &str, window: &TimeWindow, now: i64, v2: bool, cap: usize) -> Result<Vec<(i64, f64)>>`.
- Exact SQL for v2; the legacy twin uses `memories_fts`:
  `SELECT memories_fts_v2.rowid, bm25(memories_fts_v2) FROM memories_fts_v2 JOIN memories m ON m.id = memories_fts_v2.rowid WHERE memories_fts_v2 MATCH ?1 AND m.superseded_by IS NULL AND (m.expires_at = 0 OR m.expires_at > ?2) AND (?3 = -1 OR m.created_at >= ?3) AND (?4 = -1 OR m.created_at < ?4) ORDER BY rank, memories_fts_v2.rowid LIMIT ?5`
  Aliasing the FTS table breaks `bm25` (verified on SQLite 3.54); use the qualified table name. The legacy twin is the same statement with `memories_fts` substituted for `memories_fts_v2`, including the current-row and window predicates.
- `mem_ranked` calls `memory_fts_v2_usable` once. Pairing is fixed by mode:

| Mode | FTS table and terms | Ref candidate tokens | Ref/tag overlap |
|---|---|---|---|
| v2 usable | `memories_fts_v2` + `mem_query_terms` | `mem_tokens` | `mem_ref_tag_tokens` (Unicode) |
| v2 not usable | `memories_fts` + `mem_query_terms_legacy` | `mem_tokens_legacy` | `mem_ref_tag_tokens_legacy` |
| `--legacy` | `memories_fts` + `mem_query_terms_legacy` | `mem_tokens_legacy` | `mem_ref_tag_tokens_legacy` |

- `mem_ranked(conn, joined_terms, filter, is_global, index_conn, legacy: bool)` selects the row above; `legacy` comes from `MemQueryArgs.legacy`. `mem_candidate` gains the same `legacy: bool` and uses `mem_ref_tag_tokens_legacy` when true.
- `find_consolidation_match` (`src/memory/guard.rs:84`) uses the store's mode (v2 usable or legacy) and passes `TimeWindow::default()`. `consolidation_tokens` and `consolidation_similarity` gain `legacy: bool` so the Jaccard set matches the candidate set. It keeps its best-effort `.ok()?` behavior: a query error yields no hint and never fails the write. Its fixture inserts legacy text into `memories_fts` and Unicode text into `memories_fts_v2`.
- The read path propagates a MATCH error (exit 3/6). Recovery is `eg mem reindex` or any write open.

### A1 — Time window

`src/memory/time.rs` (Create):

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TimeWindow { pub since: Option<i64>, pub until: Option<i64> }

pub fn parse_time_bound(raw: &str, command: &str, flag: &str) -> Result<i64>
pub fn validate_window(window: &TimeWindow, command: &str) -> Result<()>
```

Accepted forms, exactly:

- `@<non-negative decimal>`: Unix seconds; overflow is usage exit 2.
- `YYYY-MM-DD`: 10 characters, year 1970-9999, month 1-12, day valid for the month and leap year; UTC midnight.
- `YYYY-MM-DDTHH:MM:SSZ`: 20 characters, hour 0-23, minute and second 0-59; UTC.

Rejected: bare integers (including `20261111`), timezone offsets, fractional seconds, leap second `60`, trailing characters, non-existent dates. Every rejection is `EgError::Usage` with `<command>: <flag> expects YYYY-MM-DD, YYYY-MM-DDTHH:MM:SSZ, or @<unix-seconds> (got '<raw>')`. `validate_window`: `since > until` is `<command>: --since must not be after --until`; equal bounds are allowed and produce an empty window.

Semantics: `created_at >= since` and `created_at < until`, UTC seconds.

Exact predicates, with `-1` sentinel:

- `memory_current_rows(conn, filter, limit)`: `AND (?6 = -1 OR created_at >= ?6) AND (?7 = -1 OR created_at < ?7)`.
- `pub(crate) fn memory_ref_candidate_rows(conn: &Connection, tokens: &[String], window: &TimeWindow, now: i64, cap: usize) -> Result<Vec<MemRow>>`: append `AND (expires_at = 0 OR expires_at > ?{n+1}) AND (?{n+2} = -1 OR created_at >= ?{n+2}) AND (?{n+3} = -1 OR created_at < ?{n+3})` after the token placeholders.
- `pub(crate) fn memory_current_keys(conn: &Connection, keys: &[String], window: &TimeWindow, now: i64) -> Result<Vec<String>>`: same predicates.
- `pub(crate) fn memory_fts_candidates(conn: &Connection, match_query: &str, window: &TimeWindow, now: i64, v2: bool, cap: usize) -> Result<Vec<(i64, f64)>>`: as above, filtered before `LIMIT`.
- `memory_row_is_current` gains the Rust mirror: `window.since.map_or(true, |s| row.created_at >= s) && window.until.map_or(true, |u| row.created_at < u)`.
- `MemFilter` gains `pub window: TimeWindow`; eight literals set `window: TimeWindow::default()` explicitly: `guard.rs:92`, `guard.rs:164`, `mem_query.rs:29`, `mem_ops.rs:137`, `graph.rs:280`, `memory_tests.rs:38`, `memory_tests.rs:128`, `memory_gc_tests.rs:69`. `MemFilter` does not derive `Default`.

Mode coverage:

| Command/mode | Path | Window applies |
|---|---|---|
| `mem query` ranked | `mem_ranked` -> FTS/ref candidates | yes, before cap |
| `mem query --all` | `memory_current_rows` | yes |
| merged query | both stores + `memory_current_keys` | yes on all three |
| `mem export` | `memory_current_rows` | yes |
| `mem get/delete/touch/write` | no listing | no |

CLI/service:

- Query flags `--since`, `--until`, `--local`, `--explain`, `--legacy`; export flags `--since`, `--until`, `--scan`.
- `CommandRequest::MemQuery { since, until, local, explain, legacy }`; `MemExport { since, until, scan }`; `MemReindex { global }`.
- Parsing lives in `src/service/from_cli/memory.rs` and `src/service/args.rs`, both calling `parse_time_bound` and `validate_window`. `--legacy` is a diagnostic override: it forces the legacy table and legacy terms and is recorded in the journal.
- MCP `mem_query`: `since` Str, `until` Str, `local` Bool, `explain` Bool, `legacy` Bool; `mem_export`: `since` Str, `until` Str. `ArgReader::s_opt_spec` rejects non-strings; `ArgReader::b_spec` rejects non-bools. Messages: `mem_query: 'since' must be a string`, `mem_query: 'local' must be a boolean`.
- Conflict: `mem query: --local and --global are mutually exclusive` (exit 2) from one service check (`StoreScope::from_flags`), so CLI and MCP emit identical bytes.
- Validation order, one site in `run_mem_query`: (1) `StoreScope::from_flags` conflict; (2) `parse_time_bound` for each present bound, then `validate_window`; (3) the existing `validate_mem_query` rules (`--all` with terms, no terms and no `--all`, `--include-superseded` without `--all`); (4) `explain && effective_all`.
- `MemQueryArgs` gains `pub explain: bool` and `pub legacy: bool`.
- MCP `ParamSpec` table: `mem_query` adds `since` (Str, optional, "UTC bound YYYY-MM-DD, YYYY-MM-DDTHH:MM:SSZ, or @unix-seconds"), `until` (Str, optional, same), `local` (Bool, optional, "restrict to the project store"), `explain` (Bool, optional, "add one why: line per shown ranked note"), `legacy` (Bool, optional, "use legacy ASCII tokens (diagnostic)"); `mem_export` adds `since` and `until` (Str, optional); `mem_reindex` has `global` (Bool, optional) and `core: false`.
- `ArgReader::s_opt_spec(&self, tool: &str, key: &str) -> Result<Option<String>>`: absent or null -> `Ok(None)`; string -> `Ok(Some)`; anything else -> usage `<tool>: '<key>' must be a string`. `ArgReader::b_spec(&self, tool: &str, key: &str) -> Result<bool>`: absent or null -> `Ok(false)`; bool -> value; anything else -> usage `<tool>: '<key>' must be a boolean`.
- Journal: `mem_query_args` adds `since`, `until` (parsed epoch decimal or empty), `local`, `explain`, `legacy`; the `MemExport` arm of `journal_args_memory` adds `since`, `until`, `scan` in that order after `global`; a new `MemReindex` arm journals `global`. `eg audit` is unchanged.

### A2 — Explain

`src/memory/rank.rs` (Modify): `Candidate` gains `pub from_fts: bool`.

`src/service/mem_ops.rs` (Modify): `mem_ranked` collects `fts_ids: HashSet<i64>` from the FTS hits before merging, then passes `from_fts = fts_ids.contains(&id)` to `mem_candidate`; ref-only rows and graph extras pass `false`.

`src/service/explain.rs` (Create): `pub(super) fn build(candidates: &[Candidate], query_terms: &str, now: i64, index_present: bool) -> HashMap<(bool, i64), String>`. `index_present` is `args.index_conn.is_some()`; when false, no graph label is emitted for any row.

- Key: `(is_global, row.id)`.
- Labels, fixed order, joined by `"; "`:
  1. `lexical match` when `from_fts`.
  2. `ref <raw>`: the first whitespace-separated ref whose `mem_ref_tokens` intersects the query tokens, truncated to 60 chars. If the raw ref triggers `suspected_secret`, print `ref <redacted>`. If no ref matches, the first matching tag token as `tag <token>` truncated to 40 chars.
  3. `graph depth <N>` when `graph_raw > 0`; `N = round(-log2(graph_raw))` clamped to 1-3.
  4. `age <N>d`; `N = max(0, (now - created_at) / 86400)`.
- Omit absent labels. No bm25, normalized score, weight, ratio, or body text.
- `src/service/mem_query.rs`: `--explain` requires terms; `--explain` with effective `--all` exits 2 `mem query: --explain requires terms`. `render_ranked` builds the map only when `args.explain` and passes `Some(&map)`; `--all` passes `None`. `render_mem_rows` gains `explain: Option<&HashMap<(bool, i64), String>>` and appends `  why: <labels>` after the row body.
- Without `--explain`, no `why:` line and no new row shape.

### A3 — Local scope

`src/service/scope.rs` (Create): `pub(super) enum StoreScope { Merged, Local, Global }` and `from_flags(local: bool, global: bool) -> Result<Self>` returning the conflict error.

`src/service/mem_query.rs`: `Local` opens only the project store through `crate::db::open_memory_query(&root)`; a missing store returns its existing error (exit 1). No merge, no shadowing probe, no global fallback.

### B2 — Export scan

`src/service/mem_export.rs` (Create): move `run_mem_export` out of `mem_ops.rs`; add window, scan, and journal fields.

- Rows: the exported rows after window and limit, in export order. `--limit` therefore narrows the scan; an empty export exits 1 before any scan output.
- Fields, fixed order: `key`, `body`, `tags`, `refs`. `session` is not scanned.
- Output to stderr, after the existing caveat:
  - N = 0: nothing.
  - N = 1: `eg: scan: 1 note contains suspected secrets`.
  - N > 1: `eg: scan: <N> notes contain suspected secrets`.
  - One line per note: `  <key>: <field>[, <field>...]`; when `key` is the matching field, print `  row <id>: key`.
- Never print the matched value, never change stdout, never change exit 0.
- Journal `scan` as `"true"` / `"false"`.
- CLI-only. MCP `mem_export` does not scan; `docs/agent-integration.md` states the exception: agents run the CLI for a scanned backup.

### Reindex

`src/service/mem_reindex.rs` (Create) holds `pub(super) fn run_mem_reindex(req: &CommandRequest, root_arg: Option<&Path>) -> Result<ArmOut>`. It opens the store through `open_mem_conn(root_arg, global, MemoryMode::Mutation)` (a missing store exits 1), calls `ensure_memory_fts_v2(&conn, true)`, prints exactly `reindexed <n> rows` to stdout, journals `mem_reindex`, and exits 0. The open runs the automatic repair first; the forced pass then rebuilds. If both run, `<n>` is the forced-pass count and stderr may carry the automatic progress line. The MCP tool returns the same stdout text. CLI and MCP `mem_reindex` (`global` Bool, non-core, only under `--all-tools`) share `CommandRequest::MemReindex`.

## Alternatives Considered

### Alternative A: Hard-cut `MEMORY_SCHEMA_VERSION` bump

- Pros: one table, simplest query path.
- Cons: every older binary exits 5 on every store, reads included; export cannot restore `anchored_at`, superseded chains, or revision numbering.
- Rejection: incompatible with Goal 6; not a viable peer.

### Alternative B: One table with legacy + Unicode tokens

- Pros: one table, one query.
- Cons: legacy skeleton tokens remain in the same index, so a bare `nh` still matches `Bộ nhớ`.
- Rejection: fails Goal 1 by construction.

### Alternative C: Legacy and v2 union with `MIN(bm25)`

- Pros: every row visible regardless of writer.
- Cons: BM25 is not comparable across tokenizers and corpora; the legacy arm reintroduces collisions.
- Rejection: v2-only plus repair is deterministic and collision-free.

### Alternative D: Rust fold table or `unicode-normalization`

- Pros: one tokenizer.
- Cons: dependency or incomplete script coverage.
- Rejection: SQLite folding covers Vietnamese tones; `đ` is a two-character map.

### Alternative E: Trigger-maintained single FTS table

- Pros: one table, automatic maintenance.
- Cons: SQL triggers cannot run the Rust camelCase tokenizer; a trigger-only table loses camelCase splitting and the legacy compatibility text.
- Rejection: cannot preserve both token contracts.

### Alternative F: Persisted corroboration edges

- Pros: cheap read-time counts.
- Cons: no lifecycle; sybil-inflatable; Jaccard measures duplication.
- Rejection: ref-sharing is not independent evidence; cut.

### Alternative G: Blocking export scan

- Pros: prevents a legacy secret from leaving.
- Cons: export is the documented JSONL backup; a false positive strands it.
- Rejection: advisory stderr plus the fixed detector.

### Alternative H: Keep the five-prefix detector, add boundaries only

- Pros: smallest change.
- Cons: misses fine-grained GitHub PATs, Slack, JWTs without `eyJ`, bearer tokens, and database URLs.
- Rejection: the trust boundary needs the eight pattern families; the false-positive corpus bounds the cost.

## Risks

| Risk | Impact | Mitigation | Test |
|---|---|---|---|
| Partial v2 after a crash | Reads silently omit rows | `fts_v2_complete` marker; reads use legacy until complete | `partial_v2_falls_back_to_legacy` |
| Old binary writes after migration | New binary misses the row until repair | `fts_version = 0` triggers repair on the next write open; `eg mem reindex` is explicit | Emulated old-row test |
| Old binary supersedes a row | Stale v2 row could starve the cap | Candidate SQL filters current rows before `LIMIT`; orphan sweep on full rebuild | 300-stale-rows test |
| v2 insert failure | Write fails | Transaction rollback, exit 6, no partial note | Fault-injection test |
| v2 corruption | Ranked query fails | Fail closed; `eg mem reindex` repairs | Repair test |
| Tokenizer epoch drift | Stale text kept | `fts_epoch` mismatch forces full rebuild | Epoch-change test |
| Ranking drift | Reordered results | Bilingual oracle; ASCII oracle unchanged; non-ASCII delta recorded | `mem_recall.py` |
| Backfill exceeds the 5 s busy timeout | Concurrent writers exit 4 | 5,000-row chunks, resumable, progress on stderr | 5k/50k timing |
| Detector false negatives | Secret leaves on export | Eight pattern families; advisory scan; documented best-effort | Pattern table |
| Detector false positives | Ordinary note rejected | Boundary + length rules; false-positive corpus | Accept table |
| Storage growth | v2 bytes | Measured on 5k notes; recorded | Scale script |
| 500-LOC hygiene gate | Final gate fails | Pre-planned splits | `hygiene_check.sh` |

## Non-Functional Requirements

- Security: no network; no secret value in diagnostics or `why:` refs; scan names fields only; MCP export has no scan (documented exception).
- Performance: one extra FTS insert/delete per write; one FTS query per ranked query; repair scans only pending rows. At 5,000 notes: import < 1 s, full repair < 1 s, write p95 within 1 ms of the 1.0.4 baseline. Recorded by `mem_scale.py`.
- Storage: v2 duplicates the legacy FTS for current rows. The 1.0.4 legacy FTS is 1.38 MB of a 2.94 MB store at 5,000 notes; the v2 size is measured and recorded.
- Compatibility: 1.0.4 reads and writes the legacy table; no `user_version` change; `fts_version` and v2 are not in `MEMORY_REQUIRED`.
- Rollback: downgrade the binary; 1.0.4 ignores v2 and `fts_version`. `eg mem reindex` rebuilds v2 after a re-upgrade. `--legacy` is the runtime diagnostic switch.
- Determinism: one FTS table per query; fixed explain label order; UTC bounds.
- SQLite: `remove_diacritics 2` requires SQLite >= 3.30; a unit test asserts `sqlite_version()`.

## Detailed Implementation Plan

### Phase 0 — Shared date math and fixture freeze

1. Create `src/time.rs` with both civil-date functions and tests; add `mod time;` to `src/lib.rs`; update `src/audit/render.rs` to import `civil_from_days`.
2. Create `tests/fixtures/memory/oracle-v3-vietnamese.json` with the exact schema `{seed, limit, threshold, notes:[{key, kind, body, tags, refs}], queries:[{terms, expect}], sha256}`; record the SHA-256 of the canonical bytes in `docs/baselines/memory-parity-oracle-v3.json` at first run. Do not edit the fixture after this step.
3. Verify: `cargo test --locked --lib time`.

### Phase 1 — Tokenizer module, detector, and documentation

1. Rewrite `src/memory/tokens.rs`: Unicode `mem_tokens`, `mem_tokens_legacy`, both text/query functions, exact vector tests. Until Phase 2, keep `mem_fts_text` and `mem_query_terms` delegating to the legacy functions so existing stores and the full suite remain unchanged.
2. Create `src/memory/secret.rs`; shrink `note.rs` to wrapper calls; replace the outdated `memory.rs` secret test.
3. Create `src/memory/time.rs` with `TimeWindow`, parser, validation, tests.
4. Apply the P2 wording table.
5. Verify: `cargo test --locked --lib memory::tokens`, `cargo test --locked --lib memory::secret`, `cargo test --locked --lib memory::time`, `cargo test --locked`.

### Phase 2 — Dual FTS, metadata, and repair

1. Add `fts_version`, `memory_meta`, `memories_fts_v2`, `memories_created`, and `memories_pending` to `schema/memory.sql`.
2. Replace the `migrate_memory_schema` early return with this exact ordered body inside one `BEGIN IMMEDIATE` / `COMMIT`, returning on any error after `ROLLBACK`:
   1. `ALTER TABLE memories ADD COLUMN anchored_at INTEGER NOT NULL DEFAULT 0` when absent.
   2. `ALTER TABLE memories ADD COLUMN fts_version INTEGER NOT NULL DEFAULT 0` when absent.
   3. `CREATE TABLE IF NOT EXISTS memory_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL)`.
   4. `CREATE INDEX IF NOT EXISTS memories_created ON memories(created_at)`.
   5. `CREATE INDEX IF NOT EXISTS memories_pending ON memories(id) WHERE fts_version < 2`.
   6. `PRAGMA user_version = 2` (`MEMORY_SCHEMA_VERSION`), last.
   Duplicate-column errors are tolerated only through `is_already_exists`; index creation uses `IF NOT EXISTS`; every other error propagates as exit 6.
3. Add `memory_fts_v2_usable` and `ensure_memory_fts_v2`; call `ensure` from `open_memory_conn_path`. Switch `mem_fts_text` and `mem_query_terms` to the Unicode functions.
4. Write both FTS tables in `memory_write.rs` and set `fts_version = 2`; delete both on supersede, delete, and GC.
5. Change `memory_fts_candidates` to the final signature with `TimeWindow::default()` and `now` at existing call sites; update `mem_ranked` and `find_consolidation_match` with the fixed mode pairing. Update the three-argument test calls in `src/db/memory_tests.rs:298` and `src/db/memory_gc_tests.rs:28`.
6. Add `eg mem reindex`: `src/service/mem_reindex.rs`, the `CommandRequest::MemReindex` variant, the `run_memory` arm in `src/service/run.rs`, CLI, MCP, and journal.
7. Add `mod request; mod explain; mod scope; mod mem_export; mod mem_reindex;` to `src/service.rs`; create `src/service/request.rs` with the moved `CommandRequest` and `pub use request::CommandRequest;`.
8. Extend the purity test to both tables; add partial-v2, epoch-change, repair, stale-cap, and migration tests.
9. Verify: `cargo test --locked --lib db::memory_tests`, `cargo test --locked --lib db::memory_gc_tests`, `cargo test --locked --lib memory::tokens`.

### Phase 3 — Time window

1. Add `window` to `MemFilter` at all eight sites; add the Rust condition to `memory_row_is_current`; apply the SQL predicates.
2. Add the predicates and signatures for `memory_current_rows`, `memory_ref_candidate_rows`, `memory_current_keys`, and `memory_fts_candidates`.
3. Add CLI flags, request fields, MCP args (`s_opt_spec`, `b_spec`), journal args, and validation order.
4. Create `tests/cli/memory_parity/` with `mod.rs`, `time.rs`, `explain.rs`, `scan.rs`; add `mod memory_parity;` to `tests/cli/main.rs`.
5. Verify: `cargo test --locked --lib memory::time`, `cargo test --locked --test cli memory_parity::time`.

### Phase 4 — Explain and local scope

1. Add `Candidate.from_fts`; collect `fts_ids`; update the `Candidate` literals in `src/memory.rs:292` and `src/service/mem_ops.rs:364`.
2. Create `src/service/explain.rs`; add `MemQueryArgs.explain` and `.legacy`; render `why:` lines.
3. Create `src/service/scope.rs`; add `--local`, the conflict check, and the no-fallback branch.
4. Verify: `cargo test --locked --test cli memory_parity::explain`.

### Phase 5 — Scan, surfaces, and baselines

1. Create `src/service/mem_export.rs`; move `run_mem_export` and import it in `src/service/run.rs`; add window and scan.
2. Add `tests/scripts/mem_scale.py`, `tests/scripts/old_binary_compat.sh`, and `tests/mcp/memory_window.rs` with `mod memory_window;`.
3. Update the documentation surfaces, including the tool count 19 -> 20 and the seven maintenance tools.
4. Build release and run the four baselines (write baseline, oracle-v2, oracle-v3, scale).
5. Run the final gate.

### File Ledger

#### `src/time.rs` (Create)

- Scope: shared UTC date math.
- Change: `civil_from_days`, `days_from_civil`, tests.
- Rationale: one implementation for audit formatting and memory parsing.

#### `src/lib.rs` (Modify)

- Scope: module registration.
- Change: add `mod time;`.
- Rationale: new shared module.

#### `src/audit/render.rs` (Modify)

- Scope: date formatting.
- Change: import `crate::time::civil_from_days`; delete the local copy.
- Rationale: reuse.

#### `src/memory/time.rs` (Create)

- Scope: time grammar.
- Change: `TimeWindow`, `parse_time_bound`, `validate_window`, tests for ISO, RFC 3339, `@epoch`, invalid calendar, overflow, bare integers, order.
- Rationale: one parser for CLI and MCP.

#### `src/memory/secret.rs` (Create)

- Scope: secret detection.
- Change: `suspected_secret`, wrapper, eight pattern families, runtime-built test vectors, false-positive corpus.
- Rationale: trust boundary; keeps `note.rs` and `memory.rs` under 500 lines.

#### `src/memory/note.rs` (Modify)

- Scope: validation call sites.
- Change: call the secret wrapper; keep messages and normalization.
- Rationale: detector moves out.

#### `src/memory.rs` (Modify)

- Scope: module declarations and tests.
- Change: `pub mod secret; pub mod time;`; replace the outdated secret test; add `from_fts: false` to the `Candidate` literal at `:292`.
- Rationale: public memory surface and compile.

#### `src/memory/tokens.rs` (Modify)

- Scope: tokenizer.
- Change: Unicode `mem_tokens` with `đ -> d`; legacy and Unicode text/query functions; `mem_ref_tag_tokens_legacy`; exact vector tests; ASCII equivalence.
- Rationale: single source for both indexes.

#### `src/memory/rank.rs` (Modify)

- Scope: candidate shape.
- Change: add `from_fts: bool`; no score change.
- Rationale: explain needs the matching arm.

#### `src/memory/guard.rs` (Modify)

- Scope: guard filter and candidate call.
- Change: `window: TimeWindow::default()` at both sites; mode-paired `memory_fts_candidates`; `legacy` on `consolidation_tokens`/`consolidation_similarity`; fixture writes both tables.
- Rationale: guard behavior tracks the new index.

#### `src/memory/graph.rs` (Modify)

- Scope: ref candidates and test literal.
- Change: pass the window to `memory_ref_candidate_rows`; add `window` to the `MemFilter` literal.
- Rationale: compile and consistency.

#### `schema/memory.sql` (Modify)

- Scope: fresh-store DDL.
- Change: `fts_version` last column, `memory_meta`, `memories_fts_v2`, `memories_created`, `memories_pending`; header wording.
- Rationale: fresh stores do not wait for migration.

#### `src/db/memory_store.rs` (Modify)

- Scope: write-open migration.
- Change: `MEM_FTS_EPOCH`, `memory_fts_v2_usable`, `ensure_memory_fts_v2`, exact `migrate_memory_schema`, call from `open_memory_conn_path`.
- Rationale: additive, complete, resumable.

#### `src/db/memory_write.rs` (Modify)

- Scope: FTS writes.
- Change: `insert_revision` writes legacy text to `memories_fts` and Unicode text to `memories_fts_v2`; `supersede_current` and `delete_memory_ids` delete both; `fts_version = 2` is added once to the shared `insert_revision` column and bind lists.
- Rationale: purity and compatibility.

#### `src/db/memory_query.rs` (Modify)

- Scope: read paths.
- Change: `MemFilter.window`, the Rust window condition in `memory_row_is_current`, exact single-table candidate SQL, window predicates, dual deletes, `memory_current_keys` window, GC v2 delete.
- Rationale: one read rule.

#### `src/db/query_state.rs` (Modify)

- Scope: table probe.
- Change: add `pub(super) fn object_exists_result(conn: &Connection, name: &str) -> rusqlite::Result<bool>`; `object_exists` delegates to it with `unwrap_or(false)`.
- Rationale: `memory_fts_v2_usable` must distinguish absent from probe failure.

#### `src/db/memory_tests.rs` (Modify)

- Scope: tests.
- Change: update the `MemFilter` literals and the `memory_fts_candidates` call at `:298`; purity, partial-v2, epoch, repair, window, stale-cap, migration, schema-parity tests.
- Rationale: compile and coverage.

#### `src/db/memory_gc_tests.rs` (Modify)

- Scope: tests.
- Change: `MemFilter` literal; update the `memory_fts_candidates` call at `:28`; assert v2 deletion.
- Rationale: compile and coverage.

#### `src/cli.rs` (Modify)

- Scope: flags and help.
- Change: query `--since`, `--until`, `--local`, `--explain`, `--legacy`; export `--since`, `--until`, `--scan`; `MemAction::Reindex`; touch help.
- Rationale: public surface.

#### `src/service/request.rs` (Create)

- Scope: request shapes.
- Change: move the `CommandRequest` enum here; add the new fields and `MemReindex`; `src/service.rs` re-exports it.
- Rationale: `service.rs` is 487 lines; the new fields and test literals would exceed 500.

#### `src/service.rs` (Modify)

- Scope: module root.
- Change: add `mod request; mod explain; mod scope; mod mem_export; mod mem_reindex;`; `pub use request::CommandRequest;`; update the enum literals in tests.
- Rationale: keep the public path and declare the new modules.

#### `src/service/run.rs` (Modify)

- Scope: dispatch.
- Change: import `mem_export::run_mem_export`; add the `CommandRequest::MemReindex { .. } => run_mem_reindex(req, root_arg)` arm; keep the `unreachable!` fallback.
- Rationale: a new variant without an arm panics at runtime.

#### `src/service/mem_reindex.rs` (Create)

- Scope: reindex command.
- Change: `run_mem_reindex(req, root_arg)`; open through `open_mem_conn(..., MemoryMode::Mutation)`; `ensure_memory_fts_v2(&conn, true)`; stdout `reindexed <n> rows`.
- Rationale: upgrade and repair surface.

#### `src/service/from_cli.rs` (Modify) and `src/service/from_cli/memory.rs` (Create)

- Scope: CLI mapping.
- Change: move `mem_write_request`/`mem_query_request` as `pub(super)`; add `pub(super) fn mem_export_request` and `pub(super) fn mem_reindex_request`; parse the new fields; add `mod memory;`; add `since`/`until`/`scan` to the `MemExport` journal arm and `global` to a new `MemReindex` arm.
- Rationale: stay under 500 lines and keep journal assignment explicit.

#### `src/service/args.rs` (Modify)

- Scope: MCP mapping.
- Change: `s_opt_spec`, `b_spec`; parse `since`, `until`, `local`, `explain`, `legacy`; export bounds; `mem_reindex`; validation order.
- Rationale: typed rejection and byte parity.

#### `src/service/tools.rs` (Modify) and `src/service/tools/memory.rs` (Create)

- Scope: MCP schemas.
- Change: move the `mem_*` specs as `pub(super) const` items; reference them in `TOOL_SPECS`; add `since`, `until`, `local`, `explain`, `legacy`, `mem_reindex`; add `mod memory;`.
- Rationale: a const slice cannot be spliced; individual consts can be referenced.

#### `src/service/mem_query.rs` (Modify)

- Scope: execution.
- Change: `StoreScope`, `--legacy` branch, explain wiring, validations.
- Rationale: one implementation for both surfaces.

#### `src/service/mem_ops.rs` (Modify)

- Scope: ranking.
- Change: pass `v2`, mode-paired terms, and window; collect `fts_ids`; update the `Candidate` literal at `:364`; remove `run_mem_export` (moved).
- Rationale: stay under 500 lines.

#### `src/service/mem_export.rs` (Create)

- Scope: export and scan.
- Change: `run_mem_export` moved; window; scan contract; journal fields.
- Rationale: isolate scan and keep `mem_ops.rs` under 500.

#### `src/service/scope.rs` (Create)

- Scope: store selection.
- Change: `StoreScope`, `from_flags`.
- Rationale: one conflict check.

#### `src/service/explain.rs` (Create)

- Scope: explain labels.
- Change: `build`, label rules, ref redaction.
- Rationale: keep `mem_query.rs` under 500.

#### `src/service/mem_write.rs` (Modify)

- Scope: query journal args.
- Change: `mem_query_args` adds `since`, `until`, `local`, `explain`, `legacy`; the export and reindex journal arms live in `src/service/from_cli.rs`.
- Rationale: audit can read filter state without a schema change.

#### `tests/cli/main.rs` (Modify)

- Scope: registration.
- Change: `mod memory_parity;`.
- Rationale: compile the new tests.

#### `tests/cli/memory_parity/mod.rs`, `time.rs`, `explain.rs`, `scan.rs` (Create)

- Scope: end-to-end acceptance.
- Change: shared temp-repo/seed helpers; CUJ 1-8; SQL seeding for timestamps, legacy secrets, old-binary rows; format invariant tests.
- Rationale: keep each file under 500 lines.

#### `tests/mcp/main.rs` (Modify) and `tests/mcp/memory_window.rs` (Create)

- Scope: MCP parity.
- Change: `mod memory_window;`; `since`/`until`/`local`/`explain`/`legacy`/`reindex`; typed rejection.
- Rationale: cross-surface contract.

#### `tests/fixtures/memory/oracle-v3-vietnamese.json` (Create)

- Scope: bilingual oracle, frozen in Phase 0.
- Change: schema `{seed, limit, threshold, notes:[{key, kind, body, tags, refs}], queries:[{terms, expect}], sha256}`; 10 notes, 10 queries, negative-control seed, threshold 1.0; never edited after Phase 0.
- Rationale: Goal 1 and a tamper-evident baseline.

#### `tests/scripts/mem_scale.py` (Create)

- Scope: scale measurement.
- Change: `--eg`, `--notes 5000`, `--seed 20261112`, `--writes 100`, `--writes-only`, `--baseline <json>`, `--json`; generate JSONL, import, time import, drop v2 + meta complete + `fts_version`, time one `eg mem write` (which triggers the automatic repair), run 100 writes for p50/p95, record `notes`, `import_seconds`, `repair_seconds`, `write_p50_ms`, `write_p95_ms`, `store_bytes`, `fts_legacy_bytes`, `fts_v2_bytes`, `sqlite_version`; with `--baseline`, exit 1 when `write_p95_ms > baseline_p95 + 1.0` or `import_seconds >= 1` or `repair_seconds >= 1`. `--writes-only` measures the pre-change 1.0.4 write baseline without touching v2.
- Rationale: reproducible perf evidence and a real gate.

#### `tests/scripts/old_binary_compat.sh` (Create)

- Scope: 1.0.4 compatibility.
- Change: takes `--old <path>` and `--new <path>`; creates a store with the new binary, runs `mem query` and `mem write` with the old binary, asserts exit 0 and no `user_version` change, then runs a new-binary query that sees the old write.
- Rationale: proves the reverse direction that an emulated v1 store cannot.

#### `docs/baselines/memory-parity-oracle-v2.json`, `memory-parity-oracle-v3.json`, `memory-parity-scale.json`, `memory-parity-write-baseline.json` (Create)

- Scope: measurement record.
- Change: producer commands below; exact schemas are those emitted by the scripts.
- Rationale: no unbacked claim.

#### `README.md`, `docs/cli-reference.md`, `docs/usage.md`, `docs/architecture/memory-store.md`, `docs/agent-integration.md`, `skills/engram-first/SKILL.md`, `CHANGELOG.md` (Modify)

- Scope: public surfaces.
- Change: new flags, dual-index behavior, timestamp definitions, detector scope, migration/reindex note, MCP scan exception; `docs/agent-integration.md` tool count 19 -> 20 and seven maintenance tools; `docs/cli-reference.md:218` names `created_at`; `src/db/memory_store.rs:187` doc says schema v2.
- Rationale: `AGENTS.md` §5.

#### Inspected, no change

- `src/db.rs` (glob re-export), `src/journal.rs` (free-form args), `src/audit/collect.rs` (success and empty counts skip non-zero exits; `ops_critical` and `lock_errors` still record 3-7).

### Test Matrix

Tests reuse `tests/cli/common.rs` (`TempRepo`, `eg`, `mem_write`, `mem_out`) and `tests/mcp/common.rs` (`mem_fixture`, `cli_mem`, `spawn_with_global`). Every global-store test sets `ENGRAM_GLOBAL_DIR` to the per-repo `.test_global` path so `~/.engram` is never touched. The stale-cap test depends on `CANDIDATE_CAP = 256` (`src/memory.rs:24`), the guard's `take(32)` (`src/memory/guard.rs:83`), and `CONSOLIDATION_THRESHOLD_PCT = 80` (`src/memory/guard.rs:9`).

| Test | File | Setup | Assertion | Command |
|---|---|---|---|---|
| `accepts_ordinary_words` | `src/memory/secret.rs` | `task-1`, `risk-averse`, `desk-lamp`, `sk-learn`, `application.configuration.properties` | `None` | `cargo test --locked --lib memory::secret` |
| `rejects_each_pattern` | `src/memory/secret.rs` | runtime-built vectors for all eight labels | expected label | same |
| `accepts_userinfo_without_password` | `src/memory/secret.rs` | `https://deploy@example.com:8443` | `None` | same |
| `parses_iso_and_epoch` | `src/memory/time.rs` | `2026-11-12` and `@1794441600` | equal epochs | `cargo test --locked --lib memory::time` |
| `rejects_invalid_bounds` | `src/memory/time.rs` | `20261111`, `2026-02-30`, `2023-02-29`, `1969-12-31`, `10000-01-01`, `60` seconds, `@` overflow | exit-2 usage text | same |
| `accepts_leap_day` | `src/memory/time.rs` | `2024-02-29` | valid | same |
| `unicode_tokens_preserve_words` | `src/memory/tokens.rs` | `Bộ nhớ dự án`; `đường` | `["bộ","nhớ","dự","án"]`; `["dường"]` | `cargo test --locked --lib memory::tokens` |
| `legacy_ascii_equivalent` | `src/memory/tokens.rs` | ASCII corpus; `Bộ nhớ` | legacy == Unicode for ASCII; `["b","nh"]` for Vietnamese | same |
| `v2_mirrors_legacy_rows` | `src/db/memory_tests.rs` | write, import, supersede, delete, GC | both tables one row per current note; texts equal their tokenizers | `cargo test --locked --lib db::memory_tests` |
| `partial_v2_falls_back_to_legacy` | `src/db/memory_tests.rs` | mark `fts_v2_complete = 0` with half indexed | read-only query uses legacy; all notes visible | same |
| `epoch_change_forces_rebuild` | `src/db/memory_tests.rs` | set `fts_epoch = 1` | full rebuild; v2 text refreshed | same |
| `repairs_old_binary_row` | `src/db/memory_tests.rs` | legacy-only row, `fts_version = 0` | one write open indexes it and sets 2 | same |
| `old_binary_supersede_does_not_starve_cap` | `src/db/memory_tests.rs` | 300 stale v2 rows + 1 current | current note returned | same |
| `vietnamese_consolidation_hint_fires` | `src/db/memory_tests.rs` | Vietnamese near-duplicate | guard returns a match | same |
| `migrates_anchored_store_adds_fts_version_and_index` | `src/db/memory_tests.rs` | store with `anchored_at`, no `fts_version` | migration adds both; open succeeds | same |
| `old_store_without_fts_version_opens` | `src/db/memory_tests.rs` | emulated 1.0.4 store | read and write open succeed | same |
| `fresh_and_migrated_schema_match` | `src/db/memory_tests.rs` | fresh vs migrated | `table_info(memories)` names+types equal for required columns; `sqlite_master` has `memories_created` and `memories_pending`; v2 SQL contains `remove_diacritics 2`; `memory_meta` keys present; `user_version = 2` | same |
| `window_predicate_matches_rust_filter` | `src/db/memory_tests.rs` | absent, one-sided, both, equal bounds, boundary row | SQL and Rust agree | same |
| `window_before_cap` | `src/db/memory_tests.rs` | 300 out-of-window, 1 in-window | in-window note returned | same |
| `candidate_sql_prepares` | `src/db/memory_tests.rs` | both tables | both exact SQL strings prepare and return the expected rowid | same |
| `vietnamese_unaccented_query_matches` | `tests/cli/memory_parity/mod.rs` | `mem write` Vietnamese | exit 0, key shown | `cargo test --locked --test cli memory_parity` |
| `consonant_query_does_not_match` | same | `mem query nh` | exit 1 | same |
| `time_window_boundaries` | `tests/cli/memory_parity/time.rs` | backdate via SQL | `[since, until)` | same |
| `explain_labels_no_scores` | `tests/cli/memory_parity/explain.rs` | `--explain`; ref-only note | one `why:` line; no scores; ref-only has no `lexical match` | same |
| `explain_requires_terms` | same | `--explain --all` | exit 2 | same |
| `local_excludes_global` | same | both stores | no `[global]` | same |
| `local_missing_store` | same | no local store | exit 1 | same |
| `local_global_conflict_cli` | `tests/cli/memory_parity/explain.rs` | `--local --global` | exit 2, exact message | `cargo test --locked --test cli memory_parity` |
| `scan_reports_legacy_secret` | `tests/cli/memory_parity/scan.rs` | SQL-seeded `sk-` + 24 | stderr names row/field, exit 0 | same |
| `scan_key_redacted` | same | secret in key | `row <id>: key` | same |
| `scan_zero_and_multi` | same | 0 and 2 matching notes | exact stderr wording | same |
| `legacy_flag_forces_legacy` | same | `--legacy` | same results as 1.0.4 | same |
| `mcp_query_window_parity` | `tests/mcp/memory_window.rs` | CLI and MCP | byte-identical | `cargo test --locked --test mcp memory_window` |
| `mcp_numeric_since_rejected` | same | `since: 1794441600` | usage error | same |
| `mcp_bool_strictness` | same | `"local": "true"` | usage error | same |
| `mcp_reindex` | same | `mem_reindex` | rows reindexed; same text as CLI | same |
| `parses_rfc3339_and_rejects_offsets` | `src/memory/time.rs` | `2026-11-12T00:00:00Z`; `2026-11-12T00:00:00+07:00`; `.5Z` | first valid, others usage | `cargo test --locked --lib memory::time` |
| `rejects_reversed_and_negative` | `src/memory/time.rs` | `since > until`; `@-1` | usage text | same |
| `civil_date_round_trip` | `src/time.rs` | 1970-01-01, 2024-02-29, 9999-12-31 | `days_from_civil(civil_from_days(d)) == d` | `cargo test --locked --lib time` |
| `no_why_without_explain` | `tests/cli/memory_parity/explain.rs` | default query | stdout contains no `why:` | `cargo test --locked --test cli memory_parity` |
| `explain_all_label_families` | same | notes with FTS/ref/graph/age | labels appear in fixed order | same |
| `explain_redacts_secret_ref` | same | ref containing a provider key | `ref <redacted>` | same |
| `export_window_boundaries` | `tests/cli/memory_parity/time.rs` | export with since/until | exported set matches the window | same |
| `scan_all_fields_and_stdout_unchanged` | `tests/cli/memory_parity/scan.rs` | secrets in body/tags/refs | field list comma-separated; stdout equal to non-scan export | same |
| `reindex_cli_journal_and_global` | `tests/cli/memory_parity/scan.rs` | local and global | `reindexed <n> rows`; journal fields; global store rebuilt | same |
| `reindex_missing_store` | same | no store | exit 1 | same |
| `v2_insert_failure_rolls_back` | `src/db/memory_tests.rs` | drop v2 mid-transaction | note absent, exit 6 | `cargo test --locked --lib db::memory_tests` |
| `migration_keeps_user_version` | `src/db/memory_tests.rs` | migrate v1 store | `PRAGMA user_version = 2` | same |
| `mixed_ref_legacy_parity` | `src/db/memory_tests.rs` | non-ASCII ref/tag in legacy mode | legacy token sets used | same |
| `old_binary_compat_script` | `tests/scripts/old_binary_compat.sh` | v1.0.4 worktree binary | old read/write succeed; new query sees the old write | `bash tests/scripts/old_binary_compat.sh --old ../eg-baseline/target/release/eg --new target/release/eg` |

### Verification Plan

Prerequisites: Rust 1.90, Cargo, Bash, Python 3, `git`, the `sqlite3` CLI, SQLite 3.30+ (the scale script uses `DROP TABLE`, not `DROP COLUMN`), and a clean tree.

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
bash tests/scripts/allowlist_check.sh Cargo.lock
bash tests/scripts/write_path_check.sh
bash tests/scripts/hygiene_check.sh $(git ls-files) \
  src/time.rs src/memory/time.rs src/memory/secret.rs \
  src/service/request.rs src/service/from_cli/memory.rs src/service/tools/memory.rs \
  src/service/mem_export.rs src/service/mem_reindex.rs src/service/scope.rs src/service/explain.rs \
  tests/cli/memory_parity/mod.rs tests/cli/memory_parity/time.rs \
  tests/cli/memory_parity/explain.rs tests/cli/memory_parity/scan.rs \
  tests/mcp/memory_window.rs tests/scripts/mem_scale.py tests/scripts/old_binary_compat.sh \
  tests/fixtures/memory/oracle-v3-vietnamese.json
```

Baselines:

```sh
cargo build --locked --release
mkdir -p docs/baselines
# Pre-change write baseline from the tagged release (offline worktree)
git worktree add ../eg-baseline v1.0.4
(cd ../eg-baseline && cargo build --locked --release)
python3 -I tests/scripts/mem_scale.py --eg ../eg-baseline/target/release/eg \
  --writes 100 --writes-only --json docs/baselines/memory-parity-write-baseline.json
python3 -I tests/scripts/mem_recall.py --eg target/release/eg \
  --fixture tests/fixtures/memory/oracle-v2.json --threshold 1.0 \
  --json docs/baselines/memory-parity-oracle-v2.json
python3 -I tests/scripts/mem_recall.py --eg target/release/eg \
  --fixture tests/fixtures/memory/oracle-v3-vietnamese.json --threshold 1.0 \
  --json docs/baselines/memory-parity-oracle-v3.json
python3 -I tests/scripts/mem_scale.py --eg target/release/eg --notes 5000 \
  --seed 20261112 --writes 100 \
  --baseline docs/baselines/memory-parity-write-baseline.json \
  --json docs/baselines/memory-parity-scale.json
bash tests/scripts/old_binary_compat.sh \
  --old ../eg-baseline/target/release/eg --new target/release/eg
```

The oracle-v3 run also records `fixture_sha256`; it must match the hash frozen in Phase 0.

Pass conditions:

- Both oracles: `hits == queries` and negative-control recall below the threshold.
- `mem_scale.py`: `import_seconds < 1`, `repair_seconds < 1`, `write_p95_ms` no more than 1 ms above the 1.0.4 baseline. The scale gate runs on this workstation, not in CI; the JSON is committed.
- No `why:` line without `--explain`; exactly one per shown ranked note with it.
- A partial-v2 store serves all notes through legacy; an emulated 1.0.4 row is repaired by one write open and `eg mem reindex`.
- `tests/scripts/old_binary_compat.sh` proves the v1.0.4 binary reads and writes a migrated store, and the new binary sees the old write. The SQL emulation test covers the same path without the binary.
