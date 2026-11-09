# aw — Memory amendment: portable memory and consolidation hints

Status: Proposal — rev 6 (post Goldfish 5-lens review + architecture reconciliation). No
implementation is authorized by this page. Each feature is an independent
option with its own owner decision and kill gate.

- Author: Quy Doan
- Date: 2026-11-09
- Storage baseline: memory schema v1 (`schema_memory.sql`), `MEMORY_SCHEMA_VERSION = 1`
- Companion design: [aw project memory](2026-11-05_agent-weave-memory-retrieval.md)
- External input: EverMind-AI/EverOS (Apache-2.0). Only its deterministic,
  offline, zero-token ideas are eligible.
- Review history: rev 1 (no review) → rev 2 (13 lenses) → rev 3 (rewrite) →
  rev 4 (round-2 fixes) → rev 5 (verification fixes) → rev 6 (Goldfish 5-lens
  audit: Dual-Format pipeline, standard fenced Markdown without `body_len`,
  reject-on-conflict with `--force`, single-file/stdin import with MCP global lock,
  `docs/memory.md` convention, Phase D deferral for F2, explicit implementation
  ledger & concrete Rust contracts).

---

## 1. Glossary

| Term | Meaning |
|---|---|
| **EGM** | Extended Goldfish Model: architectural framework partitioning responsibilities into Elephant, Goldfish, Editor, and Resolver roles. |
| **note** | One `memory.db` row: `key`, `rev`, `kind`, `scope`, `body`, `tags`, `refs`, `source`, timestamps. |
| **key** | Caller identity, `<= 200` chars, no control chars. |
| **kind** | `decision`, `gotcha`, `fact`, `task`, `procedure`. |
| **scope** | `durable` (never expires) or `session` (TTL bounded by `MAX_SESSION_TTL_SECS`). |
| **current row** | `superseded_by IS NULL` and (`expires_at = 0` or `expires_at > now`). |
| **elephant / goldfish / editor / resolver** | Long-term store / session lane + agent context / the calling agent that writes text / deterministic retrieval. |
| **local / global store** | `<repo>/.aw/memory.db` / `$AW_GLOBAL_DIR` (default `~/.aw/memory.db`). |
| **confinement** | aw writes/deletes only under `.aw/` and the global dir; primitives live in `db.rs`, `doctor.rs`, `journal.rs`. |
| **source** | Provenance: `cli`, `mcp`, or `import`. |

---

## 2. Problem statement

`aw` cannot read, diff, or Git-version durable memory without SQLite, and offers
no signal that several notes are really one. The project-memory design delegates
synthesis to the agent (Decision D3-A), which is correct for the invariants but
leaves those gaps open. This amendment proposes the smallest deterministic
features that close them without a model, daemon, network call, crate, or
resolver-ranking change.

---

## 3. Elephant/Goldfish role placement

| EGM role | aw surface | Delta |
|---|---|---|
| elephant | `.aw/memory.db` durable notes; FTS derived | F1 projects it to readable text (Markdown) and machine backup (JSONL); F3 completes its taxonomy |
| goldfish | `scope='session'` rows + agent context | unchanged |
| editor | agent authors consolidated notes | F2 hands it a deterministic candidate list; the agent still writes all text |
| resolver | FTS5 BM25 + ref-overlap + recency | unchanged |

Rule: **the core never generates or mutates prose.** Every feature reads rows,
emits identifiers/tokens, or replays caller input through the write path.

---

## 4. Design principles (binding)

1. Zero LLM, zero embeddings, zero network.
2. Single native binary; no new crate; no async runtime.
3. No daemon, no watcher, no hook; synchronous commands.
4. Confinement (writes only under `.aw/` and `$AW_GLOBAL_DIR`).
5. Deterministic: same store + binary + input → same output bytes.
6. Core is text-free.
7. **A new column requires a `MEMORY_SCHEMA_VERSION` bump** with a rebuild/
   export-import story; the shipped `MEMORY_REQUIRED` manifest and fixed
   `insert_revision` column list do not support silent column addition. Never
   migrate in place.
8. CLI and MCP success-path output is byte-identical.
9. **Reject-on-conflict by default**: imported notes must never silently overwrite
   newer local knowledge or corrupt recency scoring without explicit `--force`.

---

## 5. Goals, non-goals, store selection, Git lifecycle

### Goals

- **G-1 Symmetric Dual-Format Portability.** Provide matching export and import
  for both JSONL (lossless machine backup and disaster recovery) and Markdown
  (human-readable, diffable Git versioning). Both formats preserve content
  (`key`/`kind`/`scope`/`body`/`tags`/`refs`) and absolute session deadlines;
  `created_at`/`rev` are re-stamped on import.
- **G-2 Consolidation signal.** A deterministic list of notes sharing
  vocabulary; no merge text computed. Deferred to Phase D.
- **G-3 Taxonomy.** A reusable `procedure` kind.
- **G-4 Invariants** (§4) hold; zero LLM tokens.
- **G-5 Cost.** `aw build` unaffected; new read paths p95 < 20 ms at 1,000
  current durable rows (the stated domain).

### Non-goals

No semantic duplicate detection (F2 is candidate surfacing); no vector/LLM
search; no server/cross-repo merge/background sync; no automatic consolidation
or merged-body generation; no resolver ranking change.

### Store selection (normative)

`--global` selects the **global store exclusively**; absent, the command reads
and writes the **local store only**. These commands never inherit `mem query`'s
merged read. One sentence governs export, import, and digest.

### Git lifecycle and conventional paths (normative)

Because `.aw/` is gitignored by default, Git-tracked memory files must reside
under project-managed paths:
- **Default convention**: `docs/memory.md` for human-readable Git tracking;
  `docs/memory.jsonl` for machine-reproducible snapshots.
- **CLI flexibility**: `aw mem export` streams to stdout, allowing arbitrary
  redirection (`aw mem export --format markdown > docs/memory.md`).
- **Import flexibility**: `aw mem import` accepts either a file path or `-`
  (stdin), enabling piped restoration (`cat docs/memory.md | aw mem import -`).

---

## 6. Feature overview

| ID | Feature | EGM role | Risk | Independent | Phasing |
|---|---|---|---|---|---|
| F3 | `procedure` kind | elephant taxonomy | low | yes | Phase A |
| F1a | Dual-format export (stdout stream) | elephant portability | low | yes | Phase B |
| F1b | Dual-format import (single file / stdin) | elephant restore | medium | needs F1a | Phase C |
| F2 | `aw mem digest` candidate groups | editor assist | medium | yes | Phase D (deferred) |

MCP exposure:
- `mem_export`: gains `format` parameter (`jsonl` | `markdown`), default `jsonl`.
- `mem_import`: gains `format` parameter (`auto` | `jsonl` | `markdown`), `path` (repo-relative), `force` (bool). **`global` is strictly forbidden and rejected over MCP (confined to `false`)**.
- `mem_digest`: tool `mem_digest` (`global: bool`), `core:false`.

---

## 7. F1a — Dual-Format Export

### 7.1 Mechanism

`aw mem export [--format <jsonl|markdown>] [--limit N] [--global]` (default `jsonl`).
Both formats query **current rows** via `memory_current_rows` (which filters
`expires_at = 0 OR expires_at > now`). This aligns JSONL and Markdown export
predicates, ending the prior query divergence.

### 7.2 CLI

```
aw mem export [--format <jsonl|markdown>] [--limit N] [--global]
```

`--limit` enforces `value_parser!(usize).range(1..)` for both formats (`--limit 0`
exits 2). Output order is deterministic: `created_at DESC, key ASC, rev DESC`.

### 7.3 MCP

`mem_export` gains a `format` string parameter (`"One of: jsonl, markdown"`, default
`jsonl`); `core:false`. Validation occurs in `run_mem_export`, ensuring identical CLI
and MCP validation. Success payload equals CLI stdout.

### 7.4 Document grammar (normative for Markdown)

To eliminate the fragility of binary byte-length delimiters (`body_len`) that break
on Git merge conflicts, manual edits, and CRLF line normalization, the Markdown format
uses standard fenced delimiters with normalized line breaks:

```
stream   := { blank-line } { document { blank-line } }
document := opener "\n" matter "\n" body "\n" closer "\n"
opener   := "@@ aw-note " slug " @@"
matter   := single-line JSON
closer   := "@@ end aw-note " slug " @@"
```

#### Line Normalization Rule
All line endings (`\r\n`) are normalized to standard Unix newlines (`\n`) before parsing.

#### Matter JSON Schema
The `matter` line must be a valid, single-line JSON object matching this schema:
```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "NoteFrontmatter",
  "type": "object",
  "required": ["format", "key", "kind", "scope", "tags", "refs", "expires_at"],
  "properties": {
    "format": { "type": "string", "enum": ["aw-note/1"] },
    "key": { "type": "string", "maxLength": 200 },
    "kind": { "type": "string", "enum": ["decision", "gotcha", "fact", "task", "procedure"] },
    "scope": { "type": "string", "enum": ["durable", "session"] },
    "tags": { "type": "array", "items": { "type": "string" } },
    "refs": { "type": "array", "items": { "type": "string" } },
    "expires_at": { "type": "integer", "minimum": 0 }
  },
  "additionalProperties": false
}
```

#### Slug Construction
`slug` is display-only: ASCII-lowercase the key, keep `[a-z0-9]` runs joined by `-`,
truncate to 64 bytes, append `-` + 16-char lowercase zero-padded `fnv1a64(key)` hex;
empty prefix → `note-<hash>`.

#### Concrete Document Example
```markdown
@@ aw-note auth-jwt-token-a1b2c3d4e5f60718 @@
{"format":"aw-note/1","key":"auth:jwt:token","kind":"decision","scope":"durable","tags":["auth","jwt"],"refs":["src/auth.rs"],"expires_at":0}
Use Ed25519 asymmetric signatures for all internal microservice JWT verification.
Shared HMAC secrets are strictly deprecated.
@@ end aw-note auth-jwt-token-a1b2c3d4e5f60718 @@
```

---

## 8. F1b — Dual-Format Import

### 8.1 Mechanism

```
aw mem import <path> [--format <auto|jsonl|markdown>] [--force] [--global]
```

- `<path>` must be a **single file** or `-` (standard input).
- Multi-file directory scanning and recursive walking are rejected (YAGNI).
- `--format` defaults to `auto` (infers from file extension `.jsonl` / `.md`, or
  sniffs first line for `{"key":` vs `@@ aw-note`).
- When reading from stdin (`-`), `--format` defaults to `markdown` if not specified.

### 8.2 Conflict handling & recency protection (normative)

1. **Reject-on-conflict by default**:
   - For each imported note, if `key` already exists in the store:
     - If content hash matches current row: treated as a **no-op** (skipped, counted).
     - If content hash differs and `--force` is **absent**: import rejects the note
       as a conflict (skipped and counted). If zero notes are successfully imported,
       exits 2 naming the conflicting keys on stderr.
     - If content hash differs and `--force` is **present**: a new revision is
       created with `rev = current_rev + 1`, `created_at = now`, superseding the
       current row.
2. **Provenance**: Imported rows receive `source = import` and display `[imported]`
   in `mem query` and `mem get`. Content-duplicate no-ops do not flip existing provenance.

### 8.3 Security & MCP Confinement

- **MCP Global Lock**: Tool `mem_import` exposed over MCP hardcodes `global = false`.
  Any invocation attempting to write to `~/.aw/memory.db` via MCP is rejected with exit 2.
  Global imports are confined exclusively to human CLI execution (`aw mem import --global`).
- **Path confinement**: MCP `<path>` must be repo-relative; path traversal (`..`),
  absolute paths, and symlinks at any path component are rejected (exit 2).

### 8.4 Ingestion Pipeline & Transaction Atomicity

0. Parse the single file or stdin stream into validated in-memory `MemNoteInput` structs.
1. Enforce guards: file size `<= 16 MiB`, total notes `<= 5,000`, body `<= 8,192` bytes.
2. Deduplicate keys within the input stream (duplicate keys in the same batch exit 2).
3. Check conflicts against the target store.
4. If valid, open store and execute `memory_write_batch(conn, &notes, force, now)`
   under a single `BEGIN IMMEDIATE` transaction.

### 8.5 Exit codes

| Condition | Exit |
|---|---|
| Malformed document / bad JSON matter / bad format version | 2 (names error; nothing written) |
| Invalid kind/scope, cap violation, secret detected, duplicate key in input | 2 (nothing written) |
| Conflict encountered when `--force` is absent (and 0 notes imported) | 2 (names conflicting keys) |
| File size > 16 MiB or notes > 5,000 | 2 (nothing written) |
| Symlink at path component / unreadable path | 2 |
| Missing input file | 1 |
| Store I/O error | 6 |
| Success with >= 1 notes imported | 0 (`ok: imported N, skipped M (K conflicts, L expired)`) |

---

## 9. F2 — Candidate-group surfacing (`aw mem digest`) [Phase D Deferral]

### 9.1 Deferral Status
F2 is explicitly **deferred to Phase D**. Implementation will proceed only if
telemetry from `.aw/usage.log` demonstrates measurable duplicate note accumulation.

### 9.2 Upgraded Algorithm Specification

To prevent single-token bridge percolation where unrelated notes collapse into giant
connected components, the clustering algorithm requires multi-token support:

```
digest(store, now):
  rows = memory_digest_rows(store, now, LIMIT 1000)   # bounded query
  for r in rows:
      toks = mem_ref_tag_tokens(r.refs, r.tags)
      if r.key.contains(':'):
          if let Some(p) = r.key.split(':').next():
              toks += mem_ref_tokens(p)
  inv = inverted map token -> [row ids]
  df_min = MIN_GROUP_SIZE (2)
  df_max = max(df_min, rows.len() / 2)
  keep tokens with df_min <= df <= df_max

  # Pairwise intersection with minimum overlap threshold
  for pair (u, v) sharing tokens:
      if shared_tokens(u, v).len() >= 2:    # anti-percolation guard
          union(u, v)

  label_map = {}
  for (token, ids) in kept_inv:
      if ids.len() >= df_min:
          label_map[find(ids[0])].insert(token)
```

---

## 10. F3 — `procedure` kind

### 10.1 Mechanism
Add `procedure` to the closed kind set (`decision`, `gotcha`, `fact`, `task`, `procedure`).
Procedures represent repeatable operational how-tos and troubleshooting workflows.
Advisory claim: never auto-executed by `aw`.

### 10.2 Surfaces
- Rust: `MemKind::Procedure` variant in `src/memory/note.rs`.
- Dashboard: CSS badge `badge status-neutral` and `<option value="procedure">Procedure</option>`
  in `src/dashboard/assets/index.html` and `app.js`.

---

## 11. Cross-cutting: supersedes cycle guard

A cycle `A→B→A` hides keys from active views.
- `--supersedes <self>` exits 2.
- Traversal depth for cycle detection is bounded to 1 hop without requiring a secondary
  index on `superseded_by`.
- If target row `B` has `supersedes == new_key`, the write is rejected with exit 2.

---

## 12. Schema and version policy

- F1a, F1b, F3, and `source=import` require **no schema version bump** (`MEMORY_SCHEMA_VERSION = 1`).
- `MEMORY_RESTORE_HINT` is parameterized by the resolved store path:
  `"corrupt or skewed memory store at {path}; run 'aw doctor --invalidate' or restore from 'docs/memory.md'"`.

---

## Detailed Implementation Plan

## 13. File Ledger

| Action | Path | Rationale |
|---|---|---|
| `[Modify]` | `src/memory/note.rs` | Add `MemKind::Procedure`, define `NoteFrontmatter`, implement `normalize_import_note`. |
| `[Modify]` | `src/cli.rs` | Add `MemAction::Import` and update `Export` flags (`--format`, `--force`). |
| `[Modify]` | `src/service/args.rs` | Add `MemImportArgs` and update `MemExportArgs`. |
| `[Modify]` | `src/service/from_cli.rs` | Map CLI import/export arguments into `CommandRequest`. |
| `[Create]` | `src/service/mem_import.rs` | Markdown and JSONL parsing, line normalization, conflict validation. |
| `[Modify]` | `src/service/mem_ops.rs` | Dispatch import/export service actions; update `mem get` tag rendering for `[imported]`. |
| `[Modify]` | `src/service/mem_query.rs` | Update `store_tags` to accept `row.source` and render `[imported]`. |
| `[Modify]` | `src/service/tools.rs` | Register `mem_import` MCP tool (`global: false`, `core: false`). |
| `[Modify]` | `src/db/memory_query.rs` | Add `limit` parameter and SQL `LIMIT` clause to `memory_current_rows`. |
| `[Modify]` | `src/db/memory_write.rs` | Implement `memory_write_batch` and `write_revision_in_tx`. |
| `[Modify]` | `src/dashboard/assets/index.html` | Add `procedure` to kind filter dropdown. |
| `[Modify]` | `src/dashboard/assets/app.js` | Add `procedure` badge CSS rendering. |
| `[Create]` | `tests/cli/memory_export.rs` | CLI integration tests for JSONL and Markdown export. |
| `[Create]` | `tests/cli/memory_import.rs` | CLI integration tests for single file and stdin import, conflicts, and `--force`. |
| `[Modify]` | `tests/mcp/parity.rs` | Incrementally add `mem_import` to tool lists during Phase C. |

---

## 14. Concrete Rust Structs and API Signatures

### 14.1 Markdown Frontmatter DTO (`src/memory/note.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NoteFrontmatter {
    pub format: String, // must equal "aw-note/1"
    pub key: String,
    pub kind: MemKind,
    pub scope: MemScope,
    pub tags: Vec<String>,
    pub refs: Vec<String>,
    pub expires_at: i64,
}
```

### 14.2 Normalized Import Input (`src/memory/note.rs`)

```rust
#[derive(Debug, Clone)]
pub struct NormalizedImportNote {
    pub key: String,
    pub kind: MemKind,
    pub scope: MemScope,
    pub body: String,
    pub tags: String,
    pub refs: String,
    pub expires_at: i64,
    pub is_expired: bool,
}

pub fn normalize_import_note(
    frontmatter: NoteFrontmatter,
    body: String,
    now: i64,
) -> Result<NormalizedImportNote, String>;
```

### 14.3 Batch Transactional Write (`src/db/memory_write.rs`)

```rust
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BatchImportStats {
    pub imported: usize,
    pub skipped: usize,
    pub conflicts: usize,
    pub expired: usize,
}

pub fn memory_write_batch(
    conn: &mut rusqlite::Connection,
    notes: &[NormalizedImportNote],
    force: bool,
    now: i64,
) -> Result<BatchImportStats, rusqlite::Error>;
```

---

## 15. Rollout phases, acceptance, and kill gates

### Phase A — F3 `procedure` + `MEMORY_RESTORE_HINT`
- **Actions**: Add `MemKind::Procedure`, update dashboard assets, parameterize restore hint.
- **Verification**: `cargo test --locked --lib memory::note` and dashboard manual check.
- **Kill Gate**: Revert if kind validation breaks existing store queries.

### Phase B — F1a Dual Export (`jsonl` and `markdown`)
- **Actions**: Update `memory_current_rows` with `LIMIT`, implement Markdown fenced formatting in `mem_export`.
- **Verification**: `cargo test --locked --test cli memory_export`. Golden byte equality on fixtures.
- **Kill Gate**: If Markdown export cannot be parsed cleanly by standard tools, drop Markdown export.

### Phase C — F1b Dual Import (Single-file, Stdin, `--force`, MCP local lock)
- **Actions**: Implement `src/service/mem_import.rs`, batch writes, conflict detection, MCP tool.
- **Verification**: `cargo test --locked --test cli memory_import` and `cargo test --locked --test mcp`.
- **Kill Gate**: If conflict detection fails to protect local revisions, restrict import to fresh stores only.

### Phase D — F2 `mem digest` (Deferred)
- **Actions**: Evaluate usage telemetry. If justified, implement multi-token union-find with `LIMIT 1000`.
- **Kill Gate**: If zero-call threshold persists over 60 days, drop feature permanently.

---

## 16. Verification commands

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
bash tests/scripts/allowlist_check.sh Cargo.lock
bash tests/scripts/write_path_check.sh
bash tests/scripts/hygiene_check.sh
```

---

## 17. References

- Companion Design: [aw project memory](2026-11-05_agent-weave-memory-retrieval.md)
- Goldfish Audit Report: [Goldfish Review Report](../reports/2026-11-09_goldfish-review_egm-memory-amendment.md)
- Binding Rules: [`AGENTS.md`](../../../AGENTS.md)
