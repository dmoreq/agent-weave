# Concepts Brief — Hindsight-Derived Memory Capabilities

- Date: 2026-11-12
- Topic: `hindsight-memory-parity`
- Revision 2: seven-lens brainstorm complete (native isolated contexts, 2 concurrent). Revision 1 capability map is superseded; its disposition is recorded at the end.
- Status: Gate 1 passed. Concept does not survive review yet. Gate 2 and spec drafting are blocked on decisions D1-D3.

## Measured probes (this session)

| Probe | Command | Result |
|---|---|---|
| Vietnamese unaccented recall | `eg mem query "bo nho"` after writing `Bộ nhớ dự án ...` | `no-results`; the note exists |
| Vietnamese accented recall | `eg mem query "bộ nhớ"` | matches by consonant skeleton (`b nh`), not by word identity |
| Secret false positive | `eg mem write --body "task-1 pending"` | exit 2, `content contains suspected secret token` |
| Secret false positive | `eg mem write --body "risk-averse reviewer"` | exit 2, same message |
| Frozen memory baseline | `docs/evaluation.md:285` | recall@5 = 20/20 = 1.000, negative control 0.050 |

The baseline is perfect. The observed defects are not retrieval-quality regressions on the frozen oracle; they are tokenizer coverage and a secret-detector false positive.

## Corrected problem areas

| Area | Revision 1 claim | Corrected finding | Evidence |
|---|---|---|---|
| A. Time axis | No query filter uses `created_at` | True for user-facing windows. `expires_at` already filters current rows | `schema/memory.sql:27`, `src/memory/guard.rs:92-97` |
| B. Corroboration | Guard result is discarded | The guard returns one maximum match, only on `mem write`, skipped for `--supersedes` and import | `src/memory/guard.rs:104-114` |
| C. Explanation | Features never reach output | True; additionally `fuse` keeps only the fused score, and min-max values are set-relative | `src/memory/rank.rs:56-72` |
| D. Accents | Tokenizer needs `remove_diacritics` | False. `mem_tokens` keeps only ASCII alphanumerics, so the FTS text is a consonant skeleton. Changing the SQL tokenizer cannot restore discarded letters | `src/memory/tokens.rs:78`, `:100-112`; measured probe |
| E. Hygiene | Audit lacks duplicate checks | True, but audit's contract forbids materializing keys or bodies | `src/audit/memory.rs:1-4` |
| F. Scope | No `--local` | True; global rows are already tagged, and merged local+global is deliberate | `docs/architecture/memory-store.md:79-80` |
| G. Export secrets | Export does no scan | True; import and write both screen, and the detector rejects ordinary `sk-` substrings | `src/memory/note.rs:80-85`; measured probe |
| H. Fuzzy fallback | Untested gap | The named hook tests code search, not memory; the BM25 trigger is wrong because ref and graph candidates exist | `tests/scripts/search_gap_recall.py:29` |
| I. Module digest | Missing capability | Unproven demand; `eg context` and `mem symbols` cover adjacent needs | `src/cli.rs:61-69` |

## Deduplicated findings

### Critical

| ID | Finding | Evidence | Disposition |
|---|---|---|---|
| C1 | M4 is inert as specified. `mem_tokens` strips non-ASCII before indexing; the real fix is the Rust tokenizer, shared by FTS text, query terms, ref tokens, and guard Jaccard | `src/memory/tokens.rs:78`, `:100-125`; `src/memory/guard.rs:41` | Accepted. M4 is re-scoped as a tokenizer change with a reindex plan |
| C2 | No migration mechanism exists. `create_memory_schema` runs only when the manifest is incomplete; `migrate_memory_schema` only adds `anchored_at`; read-only opens never migrate; `BEGIN`/`COMMIT` errors are discarded | `src/db/memory_store.rs:265-273`, `:311-330`, `:332-344` | Accepted. Migration runner is a prerequisite |
| C3 | Adding a new table to the required manifest makes every older store exit 5, and the create path then fails on "already exists" before the gate | `src/db/memory_store.rs:23-39`, `:283-300` | Accepted. Do not extend the manifest before the runner exists |
| C4 | `created_at` is mutable. `mem write --touch` overwrites it; `mem touch` updates `anchored_at`. Any window on `created_at` is not an authorship window | `src/db/memory_write.rs:224-233`, `:389-403` | Accepted. Immutability fix precedes M1 |
| C5 | M2 cannot produce a corroboration count. The guard returns one maximum, is write-only, is skipped for supersedes, and import never calls it. Jaccard measures duplication, so paraphrases and imports inflate trust | `src/memory/guard.rs:104-114`; `src/service/mem_write.rs:44-52` | Accepted. M2 re-scoped or cut (D2) |
| C6 | M5 contradicts the audit privacy contract. Duplicate clusters need keys or bodies; JSON may not carry them | `src/audit/memory.rs:1-4` | Accepted. Move to a separate surface or drop (D3) |
| C7 | The M1 test plan cannot work. `mem_recall.py` seeds through `mem write`, which stamps `now`; the frozen oracle is not mutable | `tests/scripts/mem_recall.py:54-77`, `:94-104` | Accepted. New store fixture with controlled timestamps |
| C8 | M7 blocks the only documented backup. Export is lossy already (no `anchored_at`, superseded chain, or revision numbering) and a mid-stream block leaves truncated JSONL | `src/db/memory_store.rs:14-15`; `src/service/mem_ops.rs:167-182` | Accepted. Advisory scan only; store snapshot is the real backup |
| C9 | The secret detector rejects `task-`, `risk-`, `desk-` because it substring-matches `sk-`; it also misses AWS secrets, bearer tokens, and DSNs | `src/memory/note.rs:80-85`; measured probe | Accepted. Fix before M7 |
| C10 | Time filters change the searched row set with no journal marker, so `mem_query_empty` will read filter-induced zeros as recall failure | `src/audit/collect.rs:152-159` | Accepted. Journal filter state; split empty rate |

### Important

| ID | Finding | Evidence | Disposition |
|---|---|---|---|
| I1 | `--local` reusing the store selector falls through to global when the local store is absent; it would return the rows it was asked to exclude | `src/service/mem_query.rs:45-49` | Accepted. Absent local store is exit 1 |
| I2 | `--explain` has no wire contract. `mem query` emits text only; `fuse` discards per-signal values; weights sum to 1.25 with the graph signal | `src/memory/rank.rs:7-14`, `:56-72` | Accepted. Text-only causal labels, no raw scores |
| I3 | M2 edges have no lifecycle: delete, GC, supersede, and import do not maintain them; foreign keys are off | `src/service/mem_ops.rs:50-73`; `src/memory.rs:140-141` | Accepted. Reject persisted edges unless D2 chooses them with cascade rules |
| I4 | `MemFilter` is shared by write-path currency checks, ambient retrieval, query, and export; time bounds must not leak into the guard | `src/memory/guard.rs:92-97`, `:164-169` | Accepted. Separate `TimeWindow` type |
| I5 | Audit truncates stores at 50, rows at 500, refs at 20; only store truncation is flagged | `src/audit/memory.rs:81`, `:138`, `:157` | Accepted. Truncation must force `Insufficient` |
| I6 | Filter state, invocation id, and stage timings are absent from the journal; a failing call before root resolution writes nothing | `src/journal.rs:148-168`, `:141-144` | Accepted for filter state; invocation id deferred |
| I7 | M7's override is unnamed and could be exposed over MCP; the journal parser rejects exit codes outside `0..=7` | `src/audit/collect.rs:352-357` | Accepted. Reuse exit 2; CLI-only override |
| I8 | Export and `mem query --all` materialize the full store before display limits; MCP export has no hard cap | `src/service/mem_query.rs:221-223`; `src/service/mem_ops.rs:129-144` | Accepted. Push LIMIT into SQL; MCP export cap |
| I9 | Expense asymmetry: `mem touch` help says it refreshes `created_at`; the code updates `anchored_at` | `src/cli.rs:319-320`; `src/db/memory_write.rs:389-403` | Accepted. Help/docs fix |
| I10 | MCP import accepts `..` and follows a symlinked parent; parse errors echo file content | `src/service/mem_import.rs:157-174`, `:238-240` | Accepted as a pre-existing security defect; fix before M10 or any import work |
| I11 | Memory notes are untrusted agent output, but `eg context` guard output drops `source`, so imported or poisoned notes look like owner notes | `src/memory/guard.rs:219-228` | Accepted. Provenance in every consumption path |
| I12 | `ENGRAM_GLOBAL_DIR` skips the tracked-directory check applied to the local store | `src/db/memory_store.rs:174-178` vs `:231-241` | Accepted as a pre-existing defect |

### Suggestion / Nit

| ID | Finding | Disposition |
|---|---|---|
| S1 | FNV-1a collisions can fake idempotency; compare normalized bytes on hash match | Defer, record |
| S2 | Unbounded revisions per key and store growth without quota | Defer, record |
| S3 | Refs with `%`/`_` spoof LIKE-based ref verdicts | Defer, record |
| S4 | M8 trigram scan is adversary-triggerable at zero results | Defer with M8 |
| S5 | Export/import drops `source` and `session` provenance | Defer |
| N1 | `is_already_exists` substring-matches error text | Defer |
| N2 | JSON-RPC reflects client `id` and arbitrary protocol version | Defer |
| N3 | Import errors echo resolved paths | Fix with I10 |

## Revised concept

Three prerequisite fixes, one schema-free wave, one tokenizer wave. No module ships on an unproven failure.

### P0 — Prerequisites (each with a measured probe)

| ID | Change | Why it blocks | Scope |
|---|---|---|---|
| P1 | Make `created_at` insert-only; `mem write --touch` refreshes `anchored_at` | M1 window is otherwise not an authorship window | `src/db/memory_write.rs`, tests |
| P2 | Fix `mem touch` help text and architecture docs | Current help states the opposite of the code | `src/cli.rs`, `docs/` |
| P3 | Replace substring secret detection with provider patterns plus token boundaries; add `task-`, `risk-`, `desk-` regression tests | Rejects ordinary notes; M7 cannot be built on it | `src/memory/note.rs`, `src/memory.rs` |
| P4 | Store snapshot before any migration (`VACUUM INTO` under the existing write lock, into `.engram/`), plus a restore command path | Export is not a backup; no downgrade exists | `src/db.rs`, `src/doctor.rs`, docs |
| P5 | Version-keyed migration runner: idempotent steps, one immediate transaction, errors propagated, `user_version` last, v1 fixture | C2/C3 | `src/db/memory_store.rs`, new tests |
| P6 | Extend the journal exit-code contract in the same change as any new code | A new exit code would be dropped as invalid telemetry | `src/audit/collect.rs`, `src/journal.rs` |

### Wave A — Schema-free

| ID | Capability | Depends | Acceptance |
|---|---|---|---|
| A1 | `mem query` / `mem export` `--since` / `--until`: one timestamp grammar, documented bounds, SQL-level predicate applied before candidate caps and fusion | P1, P2, P6 | New store fixture with controlled timestamps; window excludes and includes exact boundary rows; filter state journaled |
| A2 | `--explain` text-only causal labels: `lexical matched 2/3 terms`, `ref <path>`, `age 2d`, `graph depth 3`, `signal unavailable`; never raw bm25, normalized scores, or weights | none | Default bytes unchanged; explain stable across `--limit`; CLI/MCP parity |
| A3 | `--local` scope with conflict against `--global`; absent local store is exit 1 naming the path; no fallback | none | Scope matrix; no global row in output |
| A4 | Read-time corroboration under `--explain` only: count distinct current keys sharing a ref target, excluding the same key and import-only origins | A2 | Count is computed from the already-fetched candidate set; no schema; no body rewrite |

### Wave B — Tokenizer and reindex

| ID | Capability | Depends | Acceptance |
|---|---|---|---|
| B1 | Unicode-aware `mem_tokens`: keep Unicode alphanumerics, fold diacritics (NFD strip), preserve camel/snake splitting; reindex existing FTS text | P4, P5 | Accented and unaccented queries match; no consonant-skeleton collisions; `mem_recall.py` and duplicate guard rebaselined |
| B2 | Export-time advisory scan: stderr only, no default block, CLI-only override, names key and field, never echoes the token | P3 | Clean export byte-identical; a legacy fixture is reported; exit stays 0 |

### Deferred

M8 trigram fallback, M9 module digest, M10 MCP resources. Re-open only with a measured memory-specific gap or a client that cannot use tools.

## Resolved decisions (2026-11-12)

Owner delegated selection to measured evidence. Measurements:

- Real stores: 12 rows / 5 current (local, 61 KB), 5 rows / 4 current (global, 41 KB).
- Synthetic 5,000-note store: import 0.11 s, DB 2.94 MB, legacy FTS 1.38 MB (47% of DB).
- FTS re-tokenization of 5,000 rows: 0.03 s, transient +0.38 MB (+13%, reclaimed by vacuum).
- SQLite `unicode61 remove_diacritics 2`: folds Vietnamese tones (`bo nho` matches `bộ nhớ`) but not `đ` (`duong` does not match `đường`).

| Decision | Choice | Evidence |
|---|---|---|
| D1 | Dual FTS tables; new binaries query v2 only when present; per-row `fts_version` repair; no `MEMORY_SCHEMA_VERSION` bump. The legacy table stays for 1.0.4. A union arm was rejected after Gate 2: BM25 is not comparable across tokenizers and the legacy arm reintroduces consonant-skeleton collisions. | No exit 5 on any store; rollback keeps 1.0.4 behavior; measured reindex is 0.03 s at the import cap. Hard cut was rejected because export cannot restore history and every older binary would fail every store. |
| D2 | Corroboration cut. | Gate 2: ref-sharing is not independent evidence and depends on cap, limit, window, and store scope. |
| D3 | Duplicate detection cut. Keep the existing write-time advisory hint. | Audit's privacy contract forbids keys; no observed duplicate failure; `--all --include-superseded` covers the backlog. |
| D4 | Keep `created_at` as "last asserted" (mutable via `mem write --touch`) and `anchored_at` as "last verified". Fix help/docs only. | Avoids a behavior change and a schema migration; M1 windows mean "asserted since", which is the useful handoff semantic. |

## Architecture options (superseded by the decisions above)

### D1 — B1 migration shape

| Option | Mechanism | Failure condition |
|---|---|---|
| D1a | Expand/contract: create a second FTS table with the new tokenizer, maintain it with triggers, keep the old table; no version bump; drop the old table in a later release | Trigger drift; double index size until contract |
| D1b | Hard cut: one `MEMORY_SCHEMA_VERSION` bump, mandatory snapshot, explicit downgrade refusal, version-keyed runner | Every older binary exits 5 on every store until upgraded; rollback means restore |
| D1c | Defer B1 this cycle; ship P0 and Wave A only | Non-English recall stays broken; no schema risk |

### D2 — Corroboration shape

| Option | Mechanism | Failure condition |
|---|---|---|
| D2a | Cut corroboration | No trust signal |
| D2b | Read-time ref-sharing count under `--explain` (A4) | Shared refs are not independent evidence |
| D2c | Persisted edges with `rule_version`, provenance, cascade rules, and a backfill | Schema and lifecycle cost; sybil risk unless source/session are weighted |

### D3 — Duplicate surface

| Option | Mechanism | Failure condition |
|---|---|---|
| D3a | Drop duplicate detection | Owner cannot find near-duplicates |
| D3b | Separate read-only `eg mem duplicates`, human output, explicit key reveal | New command surface |
| D3c | Counts-only audit check, no keys | Owner cannot act on the count |

## Out of scope

- LLM extraction, reflection, embeddings, vector search, server, multi-tenant banks, disposition traits, config files.
- Changing journal schema, rotation, or pruning beyond P6.
- Hindsight benchmark claims.
- Version bump, release, or tag in this program.
- M8-M10.

## Risks

| Risk | Mitigation |
|---|---|
| Tokenizer change alters ranking and duplicate detection | Rebaseline `mem_recall.py` and the duplicate guard; record before/after JSON |
| Migration bricks older binaries | P4 snapshot and P5 runner before B1; D1 decides whether the cut exists |
| Filtered queries distort health metrics | Journal filter state; split empty rate; test |
| Corroboration becomes a sybil signal | D2b excludes import origins and same-key revisions; D2c adds rule provenance |
| Scope creep from pre-existing defects | P0 fixes are prerequisites only; log S1-S5 and I10-I12 without fixing them here |

## Revision 1 disposition

| Rev 1 module | Outcome |
|---|---|
| M1 time filters | Kept as A1 after P1/P2; oracle plan replaced |
| M2 corroboration edges | Re-scoped; persistence rejected pending D2 |
| M3 explain | Kept as A2; causal labels only, no JSON |
| M4 diacritic tokenizer | Re-scoped as B1; change site is `mem_tokens`, not the SQL tokenizer |
| M5 audit hygiene | Moved out of `eg audit`; pending D3 |
| M6 `--local` | Kept as A3, 20-30 LOC, no module cycle |
| M7 export scan | Kept as B2, advisory only, after P3 |
| M8-M10 | Deferred |
