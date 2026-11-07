# Graft-learning decisions

Status: B1 superseded by the project-memory design (see its section);
B2 and B3 remain proposals, not approved. No implementation is authorized by
this page for them.

Research of a public external memory tool (Graft) identified three options that
`aw` does not implement today. They are recorded here so the owner can approve
or reject each one separately. The accepted changes — public benchmark, concepts
documentation, `--explain`, and `usage --quality` — do not depend on these
decisions.

## Decision B1: symbol annotation layer

**Mechanism.** Add `aw note add <symbol> --text ...` and show notes in
`aw context`. Store notes in `.aw/` (a separate file or a new table) keyed by
the symbol's `qualname` and a content fingerprint. Notes survive `aw build`;
a note whose symbol disappeared becomes `orphaned` instead of being deleted.

**Trade-off.** `aw` stops being a purely derived index. Notes need a lifecycle
(stale, superseded, orphaned, retention), backup semantics, a schema, and a
migration policy. `aw doctor --invalidate` would discard primary data, not only
a rebuildable index. The benefit is that decisions and gotchas can live next to
the code structure instead of in a separate tool.

**Failure conditions.** A renamed symbol silently reattaches a note to the
wrong definition; a rebuild loses notes; an agent treats a note as proof of
current behavior. Any of these makes the feature unsafe without an explicit
orphan/audit design.

**Status:** replaced (owner decision OQ6, 2026-11-05) by
`docs/designs/2026-11-05_agent-weave-memory-retrieval.md` — the `aw mem`
note store supersedes `aw note add`; its `key` + `refs` model expresses the
symbol anchoring B1 wanted (refs hold qualnames), and the store's revision
chain, `aw mem export` backup, and `doctor --invalidate` disposition answer
the trade-offs recorded above. One note store lands per repo.

## Decision B2: hybrid symbol search

**Mechanism.** Embed symbol names, qualified names, signatures, and paths with a
local model, store vectors in SQLite, and fuse the vector ranking with FTS5
BM25 using reciprocal rank fusion.

**Trade-off.** This conflicts with the single-binary, no-daemon, no-network
model. Loading a model per invocation adds cold-start cost; keeping it warm
requires a daemon or a long-lived process. A model adds hundreds of megabytes,
a new dependency, calibration work, and non-deterministic ranking. The benefit
is natural-language symbol lookup when the exact identifier is unknown.

**Failure conditions.** Cold-start cost makes every query slower; the model is
missing or incompatible; ranking changes break exact-name expectations; the
index grows beyond the current size budget. A separate opt-in tool that reads
the `aw` index avoids changing these invariants, but it is a different product
surface.

**Status:** proposed — not approved.

## Decision B3: packaged distribution

**Mechanism.** Ship prebuilt binaries with checksums and a Homebrew formula;
keep `aw doctor` as the post-install check. This does not change the Rust
binary or the index contract.

**Trade-off.** Release infrastructure, platform builds, signing or notarization,
and formula maintenance. The failure mode is packaging drift, not runtime
correctness. Adoption may justify the cost; the core tool does not require it.

**Failure conditions.** An unsigned binary is blocked by the host; the formula
points at a stale release; a prebuilt binary does not match the documented
version. Checksums and an install smoke test cover the last two.

**Status:** proposed — not approved.
