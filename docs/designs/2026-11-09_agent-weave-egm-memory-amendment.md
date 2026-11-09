# aw — Memory amendment: portable memory and consolidation hints

Status: Proposal — not approved, rev 5 (post round-2 + verification). No
implementation is authorized by this page. Each feature is an independent
option with its own owner decision and kill gate.

- Author: Quy Doan
- Date: 2026-11-09
- Storage baseline: memory schema v1 (`schema_memory.sql`), `MEMORY_SCHEMA_VERSION = 1`
- Companion design: [aw project memory](2026-11-05_agent-weave-memory-retrieval.md)
- External input: EverMind-AI/EverOS (Apache-2.0). Only its deterministic,
  offline, zero-token ideas are eligible.
- Review history: rev 1 (no review) → rev 2 (13 lenses) → rev 3 (rewrite) →
  rev 4 (round-2 fixes) → rev 5 (verification fixes, this text).

---

## 1. Glossary

| Term | Meaning |
|---|---|
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

## 3. Elephant/Goldfish role placement

| EGM role | aw surface | Delta |
|---|---|---|
| elephant | `.aw/memory.db` durable notes; FTS derived | F1 projects it to readable text; F3 completes its taxonomy |
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

---

## 5. Goals, non-goals, store selection

### Goals

- **G-1 Portability (Markdown path).** Read/diff/Git-version durable memory
  without SQLite; restore content-exactly (`key`/`kind`/`scope`/`body`/`tags`/
  `refs` and the absolute session deadline; `created_at`/`rev` re-stamped).
  Known limitation: the JSONL replay path re-derives `--ttl`, so a JSONL restore
  extends session deadlines by the replay delay; only the Markdown path keeps the
  absolute deadline.
- **G-2 Consolidation signal.** A deterministic list of notes sharing
  vocabulary; no merge text computed.
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

---

## 6. Feature overview

| ID | Feature | EGM role | Risk | Independent |
|---|---|---|---|---|
| F3 | `procedure` kind | elephant taxonomy | low | yes |
| F1a | Markdown export (stdout stream) | elephant portability | low | yes |
| F1b | Markdown import (opt-in) | elephant restore | medium | needs F1a format |
| F2 | `aw mem digest` candidate groups | editor assist | medium | yes |

MCP exposure: every new command gets a `ToolSpec` with `core:false` (behind
`--all-tools`); no silent CLI-only omission. Import over MCP accepts only a
repo-relative path.

---

## 7. F1a — Markdown export

### 7.1 Mechanism

`aw mem export --format <jsonl|markdown>` (default `jsonl`). `--format markdown`
renders **current rows** (`memory_current_rows`, which filters `expires_at = 0 OR
expires_at > now`) to **stdout only**. JSONL is unchanged and intentionally still
uses `memory_export_rows` (which does not filter expiry); this asymmetry is
documented in §8.6. aw never opens a caller-named path.

### 7.2 CLI

```
aw mem export [--format <jsonl|markdown>] [--limit N] [--global]
```

`--limit` gains `value_parser!(usize).range(1..)` for both formats (declared
change: `--limit 0` becomes exit 2). Order is the shipped export order
(`created_at desc, key asc, rev desc`).

### 7.3 MCP

`mem_export` gains a `format` string param (`"One of: jsonl, markdown"`, default
`jsonl`); `core:false`. Validation lives in the shared `run_mem_export`, so CLI
and MCP reject unknown values identically (exit 2). Success payload equals CLI
stdout.

### 7.4 Document grammar (normative)

```
stream   := { blank-line } { document { blank-line } }
document := opener "\n" matter "\n" body "\n" closer "\n"
opener   := "@@ aw-note " slug " " body_len " @@"
matter   := single-line JSON (no body)
closer   := "@@ end aw-note " slug " @@"
```

- **`body_len` is normative and bounded.** Parse as `usize` with checked
  conversion; reject if `body_len > MAX_BODY_BYTES` (8,192) or `body_len >
  remaining bytes after the matter newline`. The parser reads exactly
  `body_len` bytes, so a forged closer inside a body is ordinary content.
- **Front-matter** is a single-line JSON object **without `body`, without
  `session`** (session is not restorable and would make golden bytes
  non-hermetic), and without `content_hash`. It carries `format:"aw-note/1"`.
  Serialization is done by the JSON encoder, which escapes every control
  character (this is why the line stays single-line).
- **`slug`** is display-only: ASCII-lowercase the key, keep `[a-z0-9]` runs
  joined by `-`, truncate to 64 bytes, append `-` + 16-char lowercase
  zero-padded `fnv1a64(key)` hex; empty prefix → `note`. Framing never depends on
  slug uniqueness.
- **Non-blank bytes outside a document** in an imported file are a usage error
  (exit 2); a file with zero openers is skipped and counted.

### 7.5 Determinism

Documents ordered by the shipped export order; same store + binary → identical
bytes. Bodies emitted raw.

### 7.6 Edge cases

| Case | Behavior |
|---|---|
| Empty store / all expired | exit 1; stderr `no-results` message |
| `--limit` > rows | all rows; no cap line |
| `--limit 0`/negative | exit 2 (both formats) |
| Unknown `--format` | exit 2 (CLI and MCP) |
| `--global` no global store | exit 1 |
| Body with `---`, `@@`, JSON text, forged closer | safe (length-delimited) |
| Empty-prefix key | slug `note-<hash>` |
| `body_len` > 8192 or > remaining | exit 2 (import) |

### 7.7 Security / performance

Pure read + stdout; no file, no symlink, no new write primitive. Cost O(n log n)
at 1,000 notes (no dedicated export-order index), average ~1 KiB bodies; p95 <
20 ms, output redirected.

### 7.8 Tests

`tests/fixtures/memory/egm/export/`: empty, expired-only, one, many, unicode,
body with `---`, forged closer, no trailing newline, empty-prefix key; determinism
(two runs + replay); `--limit`/`--global`/`--format` exits; MCP `format:"yaml"` →
exit 2; expired+live with `--limit 1` emits only the live doc.

---

## 8. F1b — Markdown import (opt-in)

### 8.1 Mechanism

```
aw mem import <path> [--global]
```

`<path>` is a file or directory; every `.md` file is parsed as a stream of zero
or more documents (§7.4). This makes both
`aw mem export --format markdown > memory.md && aw mem import memory.md` and a
directory of single-document files work. Import replays parsed notes through a
single transactional batch.

### 8.2 MCP

New tool `mem_import` with **one** `path` string param (repo-relative) and
`global` boolean; `core:false`. Absolute paths and `..` are rejected before
resolution. `infer_active_root` already reads `file`/`path`, so it needs no new
arm.

### 8.3 Path and byte/note guards

| Guard | Rule |
|---|---|
| `<path>` | file or directory; symlink at any component rejected |
| Directory depth | top-level only |
| Files | `*.md` only (case-sensitive); others skipped+counted |
| Symlinks | not followed; every component below the resolved root must be a non-symlink |
| MCP path | repo-relative; canonical path must stay under the canonical root |
| `MAX_IMPORT_FILES` | 500 |
| `MAX_IMPORT_FILE_BYTES` | 1 MiB |
| `MAX_IMPORT_TOTAL_BYTES` | 64 MiB (running total; abort at cap) |
| `MAX_IMPORT_NOTES` | 5,000 (enforced during parse) |

All guard failures are a usage error (exit 2) naming the file; nothing is
written. The import boundary maps cap/regular-file failures to its own message
(it does not surface the `walk` module's internal text).

### 8.4 Parse, validate, then one transaction

0. Resolve the root only; **do not open or create either store yet**.
1. Enumerate (guards); parse every file into documents using `body_len`; reject a
   missing or non-`aw-note/1` `format`, a bad `body_len`, or duplicate keys
   across the input (exit 2).
2. Validate fields and resolve the absolute deadline via
   `normalize_import_note(input, now)` (§8.6).
3. Only if **all** documents validate: open/create the store, then call
   `memory_write_batch(conn, &[MemNoteInput], now)`.
4. `memory_write_batch` wraps one `BEGIN IMMEDIATE` and one commit; the
   per-note work is `write_revision_in_tx(tx, note, now)`. `mem write` keeps its
   single-note wrapper. All rows and FTS entries roll back together.

### 8.5 Trusted vs derived

| Field | On import |
|---|---|
| `key`, `kind`, `scope`, `tags`, `refs` | trusted, validated |
| `created_at`, `rev` | re-stamped |
| `source` | `import` on insert; a content-duplicate is a **no-op** and keeps the existing `source` (no provenance flip) |
| `session` | not present in the document; derived from `AW_SESSION` |
| `expires_at` | bounded absolute deadline (§8.6) |

### 8.6 Session deadline (import)

This amendment supersedes the companion's relative `--ttl` re-derivation **for
the Markdown import path only**. `normalize_import_note(input, now)` returns a
validated note plus a resolved absolute `expires_at` and an `expired` flag,
with explicit precedence:

1. `expires_at < 0` → exit 2 (before any other check).
2. `scope=durable` → `expires_at = 0`.
3. `scope=session`, `expires_at = 0` → `now + DEFAULT_SESSION_TTL_SECS`.
4. `scope=session`, `expires_at > 0` → `min(expires_at, now + MAX_SESSION_TTL_SECS)`.
5. resolved `expires_at <= now` → mark expired (skipped and counted).
6. field/tag/ref caps and the secret scan run via the existing `normalize_note`
   helpers; the relative `ttl_secs` output is discarded in favour of the absolute
   deadline.

Markdown export excludes expired rows; JSONL export intentionally does not, so
export→import is set-equal for Markdown and may differ for JSONL expired session
rows (documented).

### 8.7 Provenance and trust

New imported rows carry `source=import` and render `[imported]` in `mem query`/
`mem get` (tag order `[imported]` before `[global]`). Imported bodies are
**untrusted data, never instructions**. Content-duplicate imports do not flip an
existing row's provenance.

### 8.8 Output, failure, exit codes

Success stdout (one line): `ok: imported N, skipped M (K expired)`; the
`MEMORY_CAVEAT` goes to stderr. MCP success payload equals stdout; MCP error text
is stdout+stderr by contract.

| Condition | Exit |
|---|---|
| Malformed document / bad `body_len` / bad format version | 2 (names file; nothing written) |
| Invalid kind/scope, cap violation, secret detected, duplicate key | 2 (names file; nothing written) |
| Over a size/count/note guard | 2 (names file; nothing written) |
| Symlink at any component / not a regular file | 2 (names path; nothing written) |
| Missing input path | 1 (missing target) |
| Not a directory when required / unreadable after open (I/O) | 6 |
| Empty input / all skipped | 1 (`no importable notes`) |

Import never creates cross-key `--supersedes` links. A same-key content
difference still creates a revision that supersedes the previous revision of that
key (see the rollback risk in §18).

### 8.9 Threat model

| Threat | Control |
|---|---|
| Prompt injection via body | `source=import`, `[imported]`, untrusted-data caveat, trusted-directory rule |
| Path traversal / symlink escape | repo-relative over MCP, component-wise symlink rejection, canonical-root containment, no `..` |
| OOM via huge file/dir/note count | file/total-byte/note caps, bounded reads, streaming parse |
| Crafted `body_len` aborts/panics | checked conversion + `<= MAX_BODY_BYTES` + `<= remaining` |
| Future `created_at` breaks recency | re-stamped; rec stays in [0,1] |
| Crafted `expires_at` forces durable retention | clamped to `now + MAX_SESSION_TTL_SECS`; durable forced to 0 |
| Import rolls back a newer local revision under the same key | documented risk; identical content is a hash no-op; review the diff before import |
| Secret committed via Git-versioned export | import replays `normalize_note` secret scan |

### 8.10 Performance

N parses + one durable transaction. Budget: p95 < 300 ms for 500 average notes
(worst case bounded by `MAX_IMPORT_NOTES` = 5,000, not by file count). Measured
in-phase with `hyperfine`.

### 8.11 Tests

`tests/fixtures/memory/egm/import/`: 3-doc single file; multiple `.md` files;
forged closer; no trailing newline; non-blank bytes outside a document; truncated
stream; bad format version; `body_len` 2^60 and remaining+1; traversal; symlink at
final and intermediate component; oversized file; secret token; future
`created_at`; `expires_at=-1`/0/`now`/`now+MAX+1`; duplicate keys; content-duplicate
no-op (source unchanged); atomicity (0 rows and 0 FTS rows on trailing failure);
no store created on a rejected import; `[imported]` in query and get; CLI/MCP
parity; success-line bytes.

---

## 9. F2 — Candidate-group surfacing (`aw mem digest`)

### 9.1 Mechanism

Read-only grouping of **current durable** notes by shared ref/tag tokens, printing
keys and a token label only. Candidate surfacing, not detection.

### 9.2 CLI / MCP

```
aw mem digest [--global]
```

Thresholds are compiled constants (no tunable flag): `MIN_GROUP_SIZE = 2`,
`MAX_GROUPS = 64`, `MAX_TOKEN_DF = max(MIN_GROUP_SIZE, rows / 2)`. MCP tool
`mem_digest` (`global`), `core:false`; success payload equals CLI stdout.

### 9.3 Algorithm (deterministic, text-free, no body read)

```
digest(store, now):
  rows = memory_digest_rows(store, now)   # SELECT id,key,rev,kind,created_at,tags,refs
                                          # WHERE superseded_by IS NULL AND scope='durable'
                                          #   AND (expires_at = 0 OR expires_at > :now)
                                          # ORDER BY id ASC        (no body column)
  for r in rows:
      toks = mem_ref_tag_tokens(r.refs, r.tags)       # existing single helper
      if let Some(p) = r.key.split(':').next():
          if !p.is_empty(): toks += mem_ref_tokens(p)  # extension-stripped
  inv = inverted map token -> [row ids]                # dedup (token,row)
  df_max = max(MIN_GROUP_SIZE, rows.len() / 2)
  keep tokens with df <= df_max                         # drops hub tokens like `src`
  uf = union-find over rows; for each kept token: union(first, id)
  label_map = {}                                        # FINAL pass, keyed by final root
  for (token, ids) in kept_inv: label_map[find(ids[0])].insert(token)
  groups = components with size >= MIN_GROUP_SIZE
  print header first: `aw: digest — advisory token-overlap groups; aw never merges notes`
  for g in groups sorted by (size desc, min(key) asc, sorted member keys asc, min(id) asc):
      label = first 12 sorted label_map[g]; + " +N more" if truncated
      print `group <n> — <size> notes: [<label>]`
      for m in members sorted by (key asc, rev desc):
          print `  <key> <rev> <kind> <created_at>`
  exit 0 with groups; exit 1 (header printed) when none meet the threshold
```

- **Union-find**, not pairwise: O(Σ incidences + T·α(n) + T log T).
- **DF guard** `max(MIN_GROUP_SIZE, rows/2)` fixes hub collapse while keeping
  small stores (2–3 rows) functional.
- **Correct tokenizers**: `mem_ref_tag_tokens` for refs/tags; `mem_ref_tokens`
  for the key prefix (extension-stripped). No parallel normalization rule.
- **Label computed in a final pass** keyed by final roots (no mid-pass
  reparenting).
- **Deterministic**: total group order (member-key and `min(id)` tiebreaks),
  pinned member and label order.
- **Frozen clock**: `now` is a run input. The byte-determinism gate is a unit
  test on `digest(conn, frozen_now)`; CLI fixture bytes are not used for this.

### 9.4 Output / journal

stdout: header then blocks; stderr `aw: digest capped at 64 (M total)` when
capped. Journal: one line with `groups`, `total_members`, `min_group_size`,
`global` only. Never keys, refs, tag tokens, or bodies.

### 9.5 Edge cases

| Case | Behavior |
|---|---|
| Empty store / no group | header; exit 1 |
| 2–3 notes | works (df floor = MIN_GROUP_SIZE) |
| `src/a.rs`, `src/b.rs`, `src/c.rs` | no group (`src` dropped) |
| Two notes both `src/memory/tokens.rs` | one group |
| Tags `a.rs`/`b.rs` | overlap documented (tags not extension-stripped) |
| Unrelated colon-less keys | no group |
| Bridge chain | one group; caveat names transitivity |
| > 64 groups | first 64; stderr lower-bound note |

### 9.6 Security / performance

Read-only; projected query excludes `body`; no network/model. Cost bounded by
current durable rows; p95 < 20 ms at 1,000 rows; above is unmeasured.

### 9.7 Tests

`tests/fixtures/memory/egm/digest/`: two groups; 2-note and 3-note positive
controls; `src/a.rs`+`src/b.rs` negative; shared-file positive; tag overlap;
unrelated colon-less keys; bridge; >64 groups; frozen-clock unit determinism with
shuffled insertion order; journal token-free.

---

## 10. F3 — `procedure` kind

### 10.1 Mechanism

Add `procedure` to the closed kind set. No schema column change. Advisory
claim: never auto-executed.

### 10.2 Surface updates

- `MemKind`/`MEM_KINDS` variant + `parse_kind_filter`.
- Help strings: `src/cli.rs` and `src/service/tools.rs`.
- Dashboard: Rust kind map **and** UI filter (`index.html`), badge class and KPI
  sub-label (`app.js`; today unmatched kinds render the error badge).
- `schema_memory.sql`: kind comment gains `procedure`; `source` comment gains
  `import`.
- Ambient guard nudges (`src/memory/guard.rs`) stay `decision`/`gotcha`; a
  `procedure` is reachable via `mem query --kind procedure` (stated in the skill).

### 10.3 Tests

Write/query/get preserve it; unknown kind exit 2; CLI/MCP parity; JSONL round
trip; dashboard badge/filter render; guard nudges unchanged.

---

## 11. Cross-cutting: supersedes cycle guard

A cycle A→B→A hides a key: A rev1 is superseded by A rev2; A rev2 is then
cross-superseded by B rev2; A has no current row.

- `--supersedes <self>` → exit 2 (already shipped).
- A link whose target chain already reaches the new key → exit 2 (new).
- Fixture: `A→B` accepted; `B→A` rejected; both keys still current; `A→B→C`
  accepted.
- Update the `--supersedes` help/description to name the self/cycle rule.

---

## 12. Schema and version policy

- F2, F3, and `source=import` need no bump.
- A new column requires an explicit `MEMORY_SCHEMA_VERSION` bump with a
  rebuild/export-import story (the shipped manifest + fixed INSERT do not support
  silent addition). This supersedes the companion's "older store opens for write"
  bullet until the derived INSERT ships.
- `MEMORY_RESTORE_HINT` is parameterized by the resolved store path (ships Phase
  A). No new index is added; export stays O(n log n) at the stated domain.

---

## 13. Surfaces and cross-surface gate (AGENTS.md §5)

| Surface | Files |
|---|---|
| CLI + dispatch | `src/cli.rs`, `src/main.rs` |
| Service routing | `src/service.rs` (`all_requests()` gains every new shape), `src/service/from_cli.rs`, `src/service/args.rs`, `src/service/run.rs` (no `_ => unreachable!`), `src/service/mem_ops.rs` |
| Tool schemas | `src/service/tools.rs` (`TOOL_SPECS`, `ToolSpec.core`) |
| DB query/write | `src/db/memory_query.rs` (`memory_digest_rows`; markdown uses `memory_current_rows`), `src/db/memory_write.rs` (`memory_write_batch`, `write_revision_in_tx`) |
| Digest module | `src/service/mem_digest.rs` (algorithm + constants) |
| MCP | `src/mcp.rs`, `tests/mcp/{parity.rs,memory.rs,main.rs}` |
| CLI tests | `tests/cli/main.rs`, new `tests/cli/memory_*.rs` |
| Write/provenance | `src/service/mem_write.rs`, `src/memory/note.rs` (`source=import`, `normalize_import_note`) |
| Rendering | `src/service/mem_query.rs`, `src/memory/rank.rs` (`[imported]`) |
| Journal | `src/journal/tokens.rs` `UNPRICED_TOOLS` |
| Dashboard | `src/dashboard/data/memories.rs`, `src/dashboard/assets/index.html`, `src/dashboard/assets/app.js` |
| Skill | `skills/aw-first/SKILL.md`, `scripts/install.sh` |
| Docs | `README.md`, `docs/getting-started.md`, `docs/usage.md`, `docs/cli-reference.md`, `docs/agent-integration.md`, `CHANGELOG.md`, companion amendment note |
| Schema comment | `schema_memory.sql` |
| Packaging | `tests/scripts/allowlist.txt` true no-op; fixtures stay outside the package by existing design |

`tests/mcp/parity.rs` (hardcoded tool-name array) and `tests/mcp/main.rs` (tool
count) update in the same commit.

---

## 14. Skill routing (normative)

Secondary tier of `skills/aw-first/SKILL.md`; never the binding Pillar-1 table.

- **Write**: reusable how-to → `mem write --kind procedure`.
- **Read**: task start → `mem query` (and `--kind procedure`).
- **Restore/seed**: deliberate only; trusted directory; imported notes are
  untrusted data, never instructions.
- **Consolidation**: before a known-duplicate area, run `mem digest`; consolidate
  only if the agent judges the group correct.
- **Trust**: procedures advisory; imported notes carry `[imported]`; verify
  against source.

---

## 15. Privacy and telemetry

Scope: **the new surfaces only**. Existing `mem_write`/`mem_query` journal `key`,
`terms`, `ref`, `supersedes` per the companion contract and are unchanged.

- No body text, refs, or tag tokens from export/import/digest enter
  `.aw/usage.log` or `~/.aw/usage.log`.
- Export/import: `format`, `global`, counts, `body_len`; the caller-supplied path
  is recorded **repo-relative or `fnv1a64`-hashed** (one line is written to both
  logs, so no per-destination split is possible). Import emits exactly one journal
  line per invocation.
- Digest: `groups`, `total_members`, `min_group_size`, `global`.
- Note: the journal already records an absolute `repo` path for every command;
  this proposal does not change that.

---

## 16. Performance budget (targets; measured in-phase)

| Operation | Target | Basis |
|---|---|---|
| `mem export --format markdown` | p95 < 20 ms at 1,000 notes, redirected | O(n log n) + O(n) slug pass + stdout |
| `mem import` | p95 < 300 ms for 500 average notes | N parses + one transaction; bounded by `MAX_IMPORT_NOTES` = 5,000 |
| `mem digest` | p95 < 20 ms at 1,000 current durable rows | union-find + df guard, projected query |
| `aw build` wall | no delta | separate file |

Each budget has a `hyperfine` acceptance row and a recorded value under
`docs/baselines/`, or the row is deleted.

---

## 17. Rollout phases, acceptance, kill gates

House gate per phase: fmt, clippy `-D warnings`, `cargo test`, allowlist,
write-path, hygiene checks.

### Phase A — F3 `procedure` + `MEMORY_RESTORE_HINT`
Acceptance: `cargo test --lib mem_kind` (unknown kind exit 2); JSONL replay keeps
`procedure`; dashboard badge/filter; CLI/MCP parity.
**Kill**: revert if closed-set validation cannot be enforced.

### Phase B — F1a Markdown export
Acceptance: golden bytes; determinism; expired+live `--limit 1` emits only the
live doc; slug uniqueness; `--limit 0` → 2; MCP `format:"yaml"` → 2; timing row;
`write_path_check.sh` unchanged.
**Kill**: publish the exact `jq` recipe over JSONL; if it reproduces the markdown
projection on the golden fixtures, drop F1a.

### Phase C — F1b Markdown import (opt-in)
Ships: `normalize_import_note`, `memory_write_batch`, `write_revision_in_tx`,
parser, guards, provenance.
Acceptance: every §8.8 row; atomicity (0 rows + 0 FTS rows); 3-doc file
round-trip; forged closer; no store created on rejection; `[imported]`; success
line bytes; timing row.
**Kill**: keep F1a export-only if provenance/injection cannot be made safe.

### Phase D — F2 `mem digest`
Acceptance: frozen-clock unit determinism; 2–3-note positive controls; `src/a.rs`/
`src/b.rs` negative; shared-file positive; bridge; journal token-free; timing row.
**Kill**: if `mem digest` call counts are zero over the measurement window, keep
it a docs recipe and drop the command.

Spec/design review bound: **<= 3 cycles**. Round 2 and verification are complete;
this rev 5 is the disposition.

---

## 18. Risks

| Risk | Severity | Mitigation |
|---|---|---|
| Import injects untrusted text | Medium | `source=import`, `[imported]`, caveat, trusted-dir rule |
| Import rolls back a newer local revision under the same key | Medium | content-duplicate is a no-op; review the diff; documented |
| Digest hub/transitive collapse | Medium | df guard + correct tokenizers + negative controls |
| Hidden key via supersedes cycle | Medium | chain guard + A→B→A fixture |
| Markdown/SQLite divergence | Medium | SQLite primary; import replays batch write; export projection |
| Kind-set growth | Low | additive, validated values |
| Cross-surface drift | Medium | §13 + `all_requests()`/parity arrays |
| Internal labels leak | Low | strip `F1/F2/F3`, `D3-A`, `OQ*` from user-facing text |

---

## 19. Alternatives considered

- **A. Markdown as primary.** Rejected: reverses Decision D1; re-opens the
  `doctor --invalidate` data-loss hazard.
- **B. Vector search instead of digest.** Rejected: graft-learning Decision B2.
- **C. Ship nothing.** Legitimate where gaps are unmeasured; F2's evidence would
  justify it; F3 is cheap and standalone.

---

## 20. Open questions

- **OQ-1**: all new tools ship `core:false` (behind `--all-tools`); owner may
  promote later. (Default chosen so the spec is implementable.)
- **OQ-2**: concatenated stdout/file transport (this proposal); a future
  `--out <dir>` writer would need a `.aw/`-scoped primitive. Caller-path writing
  stays rejected.
- **OQ-3**: `MAX_TOKEN_DF` fraction, `MIN_GROUP_SIZE`, and `MAX_GROUPS`; compiled
  constants. No per-run flags.
- **OQ-4**: add `procedure` to the closed set (this proposal) or resolve the
  companion OQ3 first.

---

## 21. Test strategy

- Fixtures under `tests/fixtures/memory/egm/{export,import,digest}/` (created
  in-phase; the existing `tests/fixtures/memory/` holds only the recall oracle).
- Determinism: two runs + fresh-process replay; frozen clock for digest.
- Negative controls: forged closer, `src/a.rs`/`src/b.rs`, unrelated colon-less
  keys, non-overlap digest, traversal, symlink (final + intermediate), duplicate
  keys, bad `body_len`.
- Parity: CLI stdout == MCP text on success; MCP error text is stdout+stderr.
- Invariants: `write_path_check.sh`, `allowlist_check.sh` unchanged.

---

## 22. Companion design amendments (recorded)

Each ships an "amended by" note in the companion header:

1. Kind set gains `procedure` (companion OQ3 partially resolved).
2. `mem export` gains `--format markdown`.
3. Markdown import preserves the **absolute** session deadline (supersedes the
   companion's relative TTL re-derivation for this path only).
4. Opt-in batch import supersedes "no batch importer" (explicit, trusted-
   directory only; not automatic ingestion).
5. Column additions require a `MEMORY_SCHEMA_VERSION` bump (supersedes the
   companion's unimplemented "older store opens for write" bullet).
6. Opt-in global-store writes (`import --global`; `export`/`digest --global`
   exclusive) supersede the companion non-goal "no user-global memory"; global
   notes stay agent-authored and opt-in.

---

## 23. References

- Companion: [aw project memory](2026-11-05_agent-weave-memory-retrieval.md)
- Prior decisions: [graft-learning decisions](2026-11-05_graft-learning-decisions.md)
- Binding rules: [`AGENTS.md`](../../AGENTS.md)
- External study: EverMind-AI/EverOS (Apache-2.0).

---

## 24. Review disposition

- Round 1 (rev 2): 13 lenses; Critical/Important resolved.
- Round 2 (rev 3): 12 lenses; Critical/Important resolved in rev 4.
- Verification (rev 4): confirmed length-delimited framing and stream/dir import;
  remaining Important resolved in this rev 5 — markdown uses `memory_current_rows`
  (expired filtered) while JSONL is explicitly unchanged; `normalize_import_note`
  with negative-first precedence and clamp-before-validate; content-duplicate
  import is a no-op (no provenance flip); `write_revision_in_tx` +
  `memory_write_batch` named; `MAX_IMPORT_NOTES` and running-total byte cap;
  bounded `body_len`; import-owned exit-2 mapping for caps/symlinks; intermediate
  symlink and canonical-root containment; store opened only after validation;
  import success-line contract; journal path relativized/hashed and `global`
  recorded; digest `MAX_TOKEN_DF = max(MIN_GROUP_SIZE, rows/2)` for small stores;
  label final-pass; key prefix via `mem_ref_tokens`; `memory_digest_rows` with
  `ORDER BY id`; global-store amendment item 6; G-1 qualified to the Markdown
  path with the JSONL deadline-drift limitation.
