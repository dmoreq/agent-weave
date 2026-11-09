# aw — Memory amendment: features worth learning, in Elephant/Goldfish terms

Status: Proposal — not approved, **post-review**. No implementation is
authorized by this page. Each feature is an independent option with its own
owner decision.

- Author: Quy Doan (compiled after the EverOS comparison review, 2026-11-09)
- Date: 2026-11-09 (rev 2, after the Goldfish review pass)
- Storage baseline: memory schema v1 (`schema_memory.sql`); `MEMORY_SCHEMA_VERSION = 1`;
  `MEMORY_REQUIRED` is a fixed column manifest (`src/db/memory_store.rs`);
  `insert_revision` binds a fixed column list (`src/db/memory_write.rs`)
- Companion design:
  [aw project memory](2026-11-05_agent-weave-memory-retrieval.md)
- External input: EverMind-AI/EverOS (Apache-2.0). Only its deterministic,
  offline, zero-token subset is eligible here.
- Review disposition: see the final section — 13 independent Goldfish lenses,
  all Critical and Important findings resolved in this revision.

## Problem statement

The EverOS comparison isolated two structural advantages `aw` lacks: (a) a
human-readable, diffable, Git-versioned source of truth for durable memory, and
(b) any mechanism that helps an agent consolidate many small notes into fewer
durable ones. The project-memory design deliberately left both out: notes are
agent-authored prose in SQLite, and synthesis is the calling agent's job. That
split is correct for the invariants but leaves the agent no friction-reducing
help at the two moments that decide whether memory stays useful. This amendment
proposes the smallest deterministic features that close those gaps without a
model, a daemon, a network call, or a crate.

## The Elephant/Goldfish model — role placement

Per the companion design, the "elephant → goldfish → editor → resolver" narrative
is a *placement on aw*, not a claim about zeromem; the vocabulary belongs to
`google/elephant-goldfish-model`. Every candidate is judged by which role it
strengthens and whether the core stays text-free.

| EGM role | aw surface today | This proposal's delta |
|---|---|---|
| elephant (long-term store) | `.aw/memory.db` durable notes; FTS derived | F1 makes it **portable/auditable** (read-only Markdown projection); F3 completes its **taxonomy** (`procedure`) |
| goldfish (stateless short-term) | `scope='session'` rows + `expires_at` | unchanged — never grown into a subsystem |
| editor (synthesis) | agent reads `mem query` and authors a superseding note | F2 gives the editor a **deterministic candidate list**; the agent still writes all text |
| resolver (retrieval) | FTS5 BM25 + ref-overlap + recency fusion | unchanged |

Design rule carried from the companion design: **the core never generates or
mutates prose.** Every feature reads rows, emits identifiers/tokens, or replays
a caller-supplied file through the existing write path.

## Goals

- G-1: a human can read, diff, and Git-version a repo's durable memory without
  opening SQLite, and **content-restore** it through `aw` (content-exact, not a
  bit-exact replica — `created_at`/`rev` are re-stamped, per the companion's
  documented restore semantics).
- G-2: the agent can be handed a deterministic list of note groups that **share
  vocabulary** (candidate duplicates), with `aw` computing no merge text.
- G-3: the note taxonomy can express a reusable procedure.
- G-4: every aw invariant holds unchanged — no new crate, no `unsafe`, no
  network/daemon/watcher/hook, **all writes confined to `.aw/`/`~/.aw/`**,
  additive memory schema or an explicit version bump, full cross-surface sync,
  zero LLM tokens.
- G-5: `aw build` wall time unaffected; each new read path stays in the same
  latency class as `aw mem query` (p95 < 20 ms) at the measured store size, with
  the size stated.

## Non-goals

- No LLM, embeddings, vector search, or model-based extraction; no semantic
  duplicate *detection* (deterministic cross-key duplicate detection is
  impossible without a model — F2 is heuristic candidate surfacing only).
- No file watcher or cascade sync; Markdown is read/written only when the
  caller invokes a command (pull, not push).
- No automatic consolidation: `aw` never decides two notes are one or writes a
  merged body.
- No server, no long-lived process; no cross-repo merge.
- No change to the resolver's ranking.

## Proposed features

House style: mechanisms + trade-offs + failure conditions, no ranking. The
owner chooses which, if any, to authorize. **Any new command defaults to the
local store only (`<repo>/.aw/memory.db`); a `--global` opt-in mirrors
`aw mem export` and `aw mem query`.** The merged-store default of `mem query` is
NOT inherited by these commands (a merged read that then writes is ambiguous);
this is stated per feature below.

### F1a — Markdown projection (`aw mem export --format markdown`)

**Mechanism.** Optional `--format` on the existing `mem export` (default stays
`jsonl`, today's stdout contract unchanged). Renders current revisions to
**stdout only**: a single concatenated stream of documents, each an envelope
`@@ aw-note <slug> @@` line, a **JSON front-matter block** (one line: the same
field set as the JSONL export), then the verbatim body, then a terminator. The
caller performs any redirection to files; **aw never opens a caller-named path**
(confinement invariant, AGENTS.md §1). JSON front-matter is chosen over YAML to
reuse the existing JSONL scalar set and add zero parse surface.

**EGM role.** elephant — a portable, diffable projection of the durable store.

**Trade-off.** A second serialization to maintain; a document-envelope grammar
to test. Deleting a blank line in a hand-edit changes nothing until import.

**Failure conditions.** None unique: export is a pure read + stdout write.

### F1b — Markdown restore (`aw mem import <dir>`)

**Mechanism.** Reads a caller-named **directory** and replays each document
through the `mem write` path. Scope guards: only top-level `*.md` files, no
symlink following, `..`/absolute paths rejected, a max file count and a max
depth are compiled constants. Two-phase: parse and validate **every** file
first, then apply all writes in **one transaction** — a malformed file writes
nothing (whole-import atomicity). Restorable (trusted) fields: `key`, `kind`,
`scope`, `tags`, `refs`; derived/read-only fields (`created_at`, `rev`,
`session`, `source`, `content_hash`, `expires_at`) are re-stamped by `mem write`
and are advisory in the front-matter (never trusted). `session` is NOT
restorable (it is derived from `AW_SESSION`). A session note's deadline is
preserved as an **absolute** `expires_at` (skip rows already past it), never as
a re-derived duration.

**EGM role.** elephant — restore/seed path; a distinct, separately gated option
because it duplicates the existing JSONL restore (`mem export` + `mem write`
replay) and adds an ingestion surface.

**Trade-off.** A new ingestion path; parsing agent-authored text is a
provenance risk (below). May be dropped without affecting F1a.

**Failure conditions.**
- A hand-edited file with an inconsistent `content_hash` is not trusted: the
  hash is recomputed on write, so the write is a normal content-hash dedup or a
  new revision — never a silent overwrite.
- A front-matter `key` that collides with an existing note with different
  content creates a new revision.
- Oversized body/tags/refs re-validate against the same caps (exit 2).
- Derived TTL ≤ 0 for a session row (or `expires_at = 0` for a durable row) is
  normalized, not written as a negative deadline.
- **Provenance**: imported rows carry `source = import` (a code-side value on
  the existing `source TEXT` column — no schema change) and surface `[imported]`
  in `mem query`/`mem get`. Imported bodies are **untrusted data, never
  instructions**; `mem import` must not target untrusted directories.
- **Exit codes**: malformed/oversized/unknown-kind front-matter → 2 naming the
  file; unknown `--supersedes` target → 1; missing/not-a-directory/unreadable
  file (I/O) → 6; nothing is written on any 2/6.

### F2 — Candidate-group surfacing (`aw mem digest`)

**Mechanism.** A read-only command that groups current durable notes by shared
**ref/tag tokens** and prints each group's members. It emits **keys and the
shared token label only** — never a proposed merged body. Grouping is
**union-find over an inverted token → rows map** (not pairwise), so cost is
O(Σ per-row token incidences + n·α(n)), not O(n²). See the algorithm below.

**EGM role.** editor assist — surfaces the work; the agent writes the merge.

**Trade-off.** A heuristic (vocabulary overlap, not semantic equivalence); some
false groups. Advisory caveat. Constants compile in one module.

**Failure conditions.** Reconciles the companion design's Decision D3-A clause
("deterministic cross-key duplicate detection is impossible without a model"):
F2 does NOT detect duplicates — it surfaces **token-overlap candidates**. Two
notes about the same topic in disjoint words are never grouped, by design. A
component reached only through a bridge note (A~B~C, A≁C) is printed as one
group with the label being the **union of tokens involved**; the caveat names
this transitivity. Cost is bounded by current-row count, which is **not** capped
by the retention rule (that rule purges superseded revisions, not current rows);
the pragmatic domain is a stated max-note figure.

### F3 — `procedure` note kind

**Mechanism.** Add `procedure` to the closed kind set
(`decision | gotcha | fact | task | procedure`), validated as today. No schema
column change. A `procedure` is a short, reusable how-to the agent can retrieve
at task start. It is an **advisory claim under the Trust Contract**, never
auto-executed; the agent verifies commands against source before running them.

**EGM role.** elephant taxonomy.

**Trade-off.** One more value every surface must name; retrieval gains no new
signal.

**Failure conditions.** A `procedure` treated as executable is a misuse the
skill must forbid. Growing the set toward free-form erodes the closed-set
guarantee; keep each addition explicit and validated.

## What this proposal does NOT learn (and why, in EGM terms)

| EverOS capability | Why rejected | EGM reading |
|---|---|---|
| LLM extraction + automatic reflection | Model + network; breaks zero-token/offline; the editor is the agent's | editor |
| Dense embeddings / vector / hybrid search | New crate + model + non-deterministic ranking; rejected in the graft-learning Decision B2 | resolver |
| HTTP memory server | A daemon; `aw` has none by invariant | elephant I/O |
| Background indexer / watcher / cascade sync | No-daemon, no-watcher invariant | goldfish I/O |
| Multimodal ingestion (PDF/audio/office + LLM) | Python + subprocess + network | elephant ingestion |

## Storage & schema impact

- **F2 and F3 require no `memory.db` version bump**: F2 adds no column (only
  reads `tags`/`refs`); F3 is a value-only addition and the shipped manifest
  checks columns, not kind values.
- **F1b's `source = import` adds no column** (`source TEXT` exists) and no bump.
- **Correction to the first draft's additive claim.** The shipped store does NOT
  support graceful additive columns: `MEMORY_REQUIRED` is a fixed column
  manifest (`src/db/memory_store.rs`) and `insert_revision` binds a fixed column
  list (`src/db/memory_write.rs`); the companion's `PRAGMA table_info`-derived
  INSERT is not implemented. Therefore **any future front-matter field stored on
  the row is an explicit `MEMORY_SCHEMA_VERSION` bump with a rebuild/export-import
  story** — not silent additive evolution. State this, or implement the derived
  INSERT first.
- `schema_memory.sql` line 17's closed-set comment joins the F3 commit
  (`decision | gotcha | fact | task | procedure`).
- The two exit-5 messages are value-unchanged; note the global-store recovery
  hint (`MEMORY_RESTORE_HINT`) currently names `.aw/memory.db` even when it fires
  for `~/.aw/memory.db` — parameterize it with the resolved store path as a
  small companion fix, or record it as a known follow-up.

## `aw mem digest` algorithm (deterministic, text-free)

```
digest(store = local by default; --global opts in):
  rows   = current_rev_rows(store)                 # superseded_by IS NULL, not expired
  for r in rows:
      toks = mem_ref_tag_tokens(r.refs, r.tags)    # the refs/tags tokenizer
                                                     # (strips a trailing extension; NOT mem_tokens)
      if r.key contains ':': toks += prefix(r.key)  # prefix ONLY when present; never ''
  inv = inverted map token -> [row ids]             # single pass
  uf  = union-find over rows; for each token, union all its rows   # O(Σ incidences)
  groups = components with size >= MIN_GROUP_SIZE   # explicit filter (no 1-member groups)
  for g in groups sorted by (size desc, min(key) asc):
      label = sorted(union of tokens that touch g)  # defined even for bridge chains
      print: `group <n> — <size> notes: [<label joined by space>]`
      for m in members sorted by (key asc, rev desc):
          print: `  <key> <rev> <kind> <created_at>`
  header: `aw: digest — advisory token-overlap groups; aw never merges notes`
  exit 0 with groups; exit 1 (caveat printed) when no group meets the threshold
```

Properties: same store + binary → identical bytes (group order total because
current rows are one-per-key; member order and label order pinned); cost
O(Σ incidences); no model; never writes. `MIN_SHARED` from the first draft is
dropped (union-find shares ≥1 token); `MIN_GROUP_SIZE` and the prefix rule
compile as constants. Constants live in one module.

## Surfaces & cross-surface gate (AGENTS.md §5)

New surfaces: `mem export --format markdown` (flag value), `mem import <dir>`
(subcommand), `mem digest` (subcommand), `procedure` kind, `--global` on the
new paths. Every one updates:

- `src/cli.rs` (+ `src/main.rs` dispatch); `src/service.rs` (`CommandRequest`,
  `from_cli`/`from_tool_args`); `src/service/tools.rs` (`TOOL_SPECS`) **and**
  `src/service/args.rs` (`from_tool_args` arms) — `TOOL_SPECS` alone does not
  route a tool; `src/mcp.rs`.
- MCP tools named explicitly: `mem_import`, `mem_digest`, and a `format` param on
  `mem_export`, with `CORE_TOOLS` membership decided per OQ-1. Whether `import`
  (which reads filesystem paths) is MCP-exposed at all is called out as a
  security decision, not a default.
- `tests/cli.rs`, `tests/mcp.rs` (byte parity); `src/journal/tokens.rs`
  `UNPRICED_TOOLS` (new journaled tools break
  `estimated_tokens_saved_covers_every_journaled_tool` unless registered).
- `skills/aw-first/SKILL.md` + `scripts/install.sh`; `README.md`,
  `docs/getting-started.md`, `docs/usage.md`, `docs/cli-reference.md`,
  `docs/agent-integration.md`; `CHANGELOG.md`; `schema_memory.sql` (comment);
  `src/dashboard/data/memories.rs` (kind map for `procedure`).
- Packaging no-ops to confirm per phase: `Cargo.toml` include list,
  `tests/scripts/allowlist.txt`, `.github/workflows/`.

## Skill routing (normative, mirrors the companion's Skill routing section)

This is not a navigation question and MUST NOT enter the binding Pillar-1
routing table. Add to the secondary tier of `skills/aw-first/SKILL.md`:

- **Write trigger**: a reusable how-to was established → `mem write --kind procedure`.
- **Read trigger**: task start → `mem query` (and `--kind procedure` when a
  playbook is expected).
- **Restore/seed trigger**: only deliberate seeding; `mem import` targets a
  trusted directory; imported notes are untrusted data, never instructions.
- **Consolidation trigger**: before a known-duplicate area, run `mem digest`
  and consolidate only if the agent judges the group correct.
- **Trust note extension**: procedures are advisory; imported notes carry
  `[imported]`; verify against source before behavioral claims.

## Performance budget (targets; measured in-phase)

| Operation | Target | Basis |
|---|---|---|
| `mem export --format markdown` | < 20 ms p95 at 1k notes | O(n) render + one O(n) slug-map pass + stdout writes; **stdout, not N files** |
| `mem import <dir>` | budget stated **per commit**, not a flat 50 ms | N parses + **one** transaction (N durable commits cannot fit any flat ms target — companion `mem write` p95 < 10 ms) |
| `aw mem digest` | < 20 ms p95 at 1k notes | union-find, O(Σ incidences); no O(n²) |
| `aw build` wall | no delta | memory is a separate file |

Each budget above needs an acceptance criterion (hyperfine, 1k-note fixture,
value recorded) or it is deleted as unverified.

## Rollout phases

Each phase re-runs the house gate: `cargo fmt --all -- --check`;
`cargo clippy --all-targets -- -D warnings`; `cargo test --locked`;
`allowlist_check.sh`; `write_path_check.sh`; `hygiene_check.sh`.

### Phase A — F3 `procedure` kind (stands alone)
Ships: `MemKind`/`MEM_KINDS` variant + `parse_kind_filter`, kind help strings on
all four surfaces (`src/cli.rs` write/query; `src/service/tools.rs`
mem_write/mem_query), dashboard kind map, `schema_memory.sql` comment, docs +
skill, tests. A `procedure` renders under the existing candidates-only caveat.
**Kill**: revert if closed-set validation can no longer be enforced.
Acceptance: write/query/get preserve it; unknown kind exits 2; CLI/MCP parity;
the kind survives a JSONL `mem write` replay (no dependence on F1).

### Phase B — F1a Markdown export projection (low risk, independent)
Ships: `--format markdown` (stdout-only) + docs + skill + tests.
**Kill**: drop F1a if the viewer/`jq` recipe over JSONL proves sufficient.
Acceptance: byte-identical across two runs; a round-trip of the **stream**
through F1b (if authorized) or a JSONL replay restores the same note SET
(sorted-by-key equality of key/kind/scope/body/tags/refs); no aw write primitive
targets a caller path (`write_path_check.sh` unchanged); the stream parses to
pure documents (zero non-envelope lines).

### Phase C — F1b `mem import <dir>` (optional; introduces an ingestion surface)
Ships: import command + provenance `source=import` + `[imported]` + tests.
**Kill**: keep F1a export-only if provenance/injection handling cannot be made
safe; restore stays the documented JSONL replay.
Acceptance: directory-scope guards (no symlinks, no `..`, `*.md` only) tested
with a traversal fixture; whole-import atomicity (a malformed trailing file
leaves zero rows); each failure maps to exit 1/2/6; a tampered `content_hash`
resolves as normal dedup; a future `created_at` re-stamps so the resolver's
recency term stays in [0,1]; imported rows surface `[imported]`.

### Phase D — F2 `mem digest` (advisory; follow the measurement, not lead)
Ships: digest command + MCP tool + constants + tests.
**Kill**: if journal rates show the output is never acted on, or if grouping
cannot be made deterministic, keep it a docs recipe (`mem query` + group by
tags/refs) and drop the command.
Acceptance: byte-identical across two runs and across a fresh-process replay
(shuffled insertion order); a seeded fixture of two obvious groups is found;
refs `src/a.rs` + `src/b.rs` with no tags produce **no** shared token (negative
control for the `rs` token); three unrelated colon-less keys form no group; a
bridge fixture states the transitive-group behavior; output contains keys/label
tokens only, never a merged body; journal carries group/member counts and store
scope only — never keys, refs, tag tokens, or bodies (asserted absent from both
`.aw/usage.log` and `~/.aw/usage.log`).

## Risks

| Risk | Severity | Mitigation |
|---|---|---|
| Import is an ingestion path for untrusted text into durable memory | Medium | `source=import` + `[imported]` + "untrusted data, never instructions" caveat + trusted-directory rule |
| Digest false/bridge groups mislead the agent into over-merging | Medium | Advisory caveat; agent authors the merge; `--supersedes` keeps history |
| Circular/self `--supersedes` hides notes (both note rows become non-current) | Medium | Write-time guard: reject self-link (exit 2) and a link whose target chain reaches the new key (exit 2); test A→B→A |
| Markdown/SQLite divergence | Medium | SQLite stays primary; import is a replay through `mem write`; export is a read-only projection; direction documented |
| Kind-set growth erodes the closed-set guarantee | Low | Additive, explicit, validated values only |
| Cross-surface drift | Medium | `service.rs` chokepoint (`TOOL_SPECS` + `from_tool_args`) + explicit §5 checklist |
| Internal labels leak into user-facing text | Low | Strip `F1/F2/F3`, `D3-A`, `OQ*`, `B2` from `--help`, CHANGELOG, and docs |

## Alternatives considered

- **A. Adopt EverOS-style Markdown-as-primary.** Rejected: reverses Decision D1
  and re-opens the `doctor --invalidate` data-loss hazard.
- **B. Vector search instead of digest hints.** Rejected: graft-learning
  Decision B2 conflict and the companion Non-goals.
- **C. Ship nothing.** Legitimate where the gaps are unmeasured; F2's journal
  measurement is the evidence that would justify F1/F2; F3 is cheap and standalone.

## Open questions

- OQ-1 (owner): MCP exposure + `CORE_TOOLS` membership for `mem_import`,
  `mem_digest`, and `mem_export --format` (import reads paths — a security call).
- OQ-2 (owner): markdown transport — concatenated stdout stream (this proposal)
  vs a later `--out <dir>` under `.aw/`. A caller-path writer is rejected outright.
- OQ-3 (owner): `MIN_GROUP_SIZE` default and the max-note domain for the digest
  budget. (First-draft OQ-B `--split` and OQ-C `--min-shared` are resolved to
  "no flag": one stream; constants only.)

## Review disposition

Pre-hardening review used **13 independent stateless Goldfish lenses** —
retrieval/determinism, safety/confinement, output-contract/CLI-MCP parity,
algorithm/correctness, cross-surface gate, acceptance/kill-gate design,
round-trip integrity, schema/version policy, privacy/telemetry, performance/cost,
agent-usability/adversarial, minimalism/YAGNI, documentation hygiene. All
Critical and Important findings are resolved in this rev-2 text: export is
stdout-only (confinement); digest uses union-find, `mem_ref_tag_tokens`, no empty
prefix, applied `MIN_GROUP_SIZE`, pinned ordering, and is re-scoped as
candidate-surfacing (not detection); the cross-surface checklist names
`from_tool_args`/`src/mcp.rs`/`tests/mcp.rs`/`CHANGELOG`/journal tokens/dashboard
map; per-phase kill gates added; round-trip fields, atomicity, provenance, and
exit codes specified; the additive-schema claim corrected to a version bump; the
privacy/journal fields pinned; performance budgets rebased and tied to
acceptance; supersedes cycle guard added; skill routing added. Spec/design review
bound is ≤ 3 cycles (AGENTS.md §1); this was cycle 1.
