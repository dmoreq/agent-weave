# Goldfish Review Report

- Target: `docs/archive/designs/2026-11-09_agent-weave-egm-memory-amendment.md` (Full Document: Lines 1–689)
- Date: 2026-11-09
- Lenses run: 5 (Zoom-Out, Comprehension, Design Critic, Implementation Readiness, Codebase Details)
- Disposition: Conditional Rejection / Revision Required (rev 5 → rev 6)

## Findings

### Critical Findings

#### F-01: Length-Delimited Framing (`body_len`) Breaks Git Versioning, CRLF, and Human Edits
- Severity: Critical
- Lens: Zoom-Out, Design Critic
- Scope: §7.4 (lines 147–171), §8.4 (lines 247–249), Goal G-1 (lines 77–82)
- Description: Goal G-1 establishes the Markdown path for Git versioning and human diffing/editing of memory without SQLite. However, §7.4 mandates a strict binary byte-length delimiter (`body_len` in `@@ aw-note slug body_len @@`). Any line ending normalization (`\r\n` vs `\n` via `core.autocrlf`), manual text edit, or merge conflict resolution alters byte offsets, immediately causing `aw mem import` to abort with exit 2 ("bad body_len"). This turns the document into a fragile binary container rather than readable, diffable Markdown.
- Recommendation: Replace byte-length delimiters with standard Markdown fence markers (`---` or `@@` tokens) and line normalization, or compute lengths dynamically upon parsing. If byte-exact binary serialization is required, use existing JSONL.

#### F-02: JSONL vs Markdown Split-Brain and Asymmetric Importer
- Severity: Critical
- Lens: Zoom-Out
- Scope: §7.1 (lines 121–127), §8.1 (lines 207–217), §8.6 (lines 270–286)
- Description: The proposal introduces `aw mem import` exclusively for Markdown, leaving JSONL export (`aw mem export`) without an import counterpart despite `src/db/memory_store.rs#L14` citing JSONL export as the sole disaster-recovery backup. The design also creates split query predicates (`memory_current_rows` for Markdown vs `memory_export_rows` for JSONL) and conflicting expiration semantics (absolute deadlines in Markdown vs relative TTL drift in JSONL).
- Recommendation: Provide symmetric format support in `aw mem import` (`--format <jsonl|markdown>`) through a shared row-ingestion pipeline, and unify export row selection on `memory_current_rows`.

#### F-03: Silent Overwrite and Recency Invalidation on Re-Import
- Severity: Critical
- Lens: Design Critic
- Scope: §8.5 (line 263), §8.8 (lines 311–313), §18 (line 599)
- Description: `aw mem import` creates new revisions for existing keys whenever content differs, re-stamping `created_at = now` and incrementing `rev`. Importing an older Git export over newer local edits silently supersedes local knowledge and resets recency scoring. Stating "review the diff" in §18 provides no protection for automated MCP agents calling `mem_import`.
- Recommendation: Reject importing keys that have newer or differing current local revisions unless an explicit `--overwrite` or `--force` flag is passed.

#### F-04: Missing Standard Implementation Boundary Header (Semantic Split Violation)
- Severity: Critical
- Lens: Implementation Readiness
- Scope: Section 13 (`docs/archive/designs/2026-11-09_agent-weave-egm-memory-amendment.md#L487`)
- Description: No standard boundary header (`## Detailed Implementation Plan`, `## Implementation Plan`, `## Work Breakdown`, or `## Implementation Steps`) exists in the document, leaving the boundary between architecture and actionable implementation undefined and violating review standards.
- Recommendation: Insert `## Detailed Implementation Plan` before Section 13.

#### F-05: Incomplete and Unannotated File Ledger
- Severity: Critical
- Lens: Implementation Readiness, Codebase Details
- Scope: Section 13 (lines 487–506)
- Description: Section 13 lists bare file paths without operational action tags (`[Create]`, `[Modify]`, `[Delete]`). The markdown import parser module path is omitted entirely, CLI test files are wildcarded (`new tests/cli/memory_*.rs`), and test fixtures are missing from the ledger.
- Recommendation: Convert Section 13 into a formal File Ledger with `[Create]`, `[Modify]`, `[Delete]` tags, explicit parser module paths, and exact test file paths.

#### F-06: Missing JSON Matter Schema and Format Example
- Severity: Critical
- Lens: Comprehension
- Scope: Section 7.4 (lines 148–164)
- Description: The grammar defines `matter := single-line JSON (no body)` and lists excluded fields, but omits the included schema, field types (e.g., whether `tags` and `refs` are JSON arrays or whitespace strings), and concrete format examples. Implementers cannot write the serializer or parser.
- Recommendation: Specify the exact JSON schema and provide a complete example document.

---

### Important Findings

#### F-07: Undefined Repository and Git Lifecycle
- Severity: Important
- Lens: Zoom-Out
- Scope: Section 2 (lines 34–42), Section 5 (lines 73–102)
- Description: `.aw/` is gitignored by default. The design does not state where Git-tracked memory files reside, when export and import run in the developer/agent lifecycle, or how concurrent branch merges in Git are reconciled upon re-import into SQLite.
- Recommendation: Specify conventional repository file locations (e.g., `docs/memory/`), agent lifecycle triggers, and merge resolution rules.

#### F-08: Unconstrained Union-Find Clustering Percolation and Hub Collapse
- Severity: Important
- Lens: Design Critic, Codebase Details
- Scope: §9.3 (lines 376–386), §9.5 (line 420), §18 (line 600)
- Description: The union-find digest merges notes on any single shared kept token (`union(first, id)`). Transitive chaining across distinct low-frequency tokens collapses unrelated notes into giant connected components sharing zero global vocabulary. Additionally, omitting `df >= MIN_GROUP_SIZE` allows single-note tokens (`df = 1`) to dominate group labels.
- Recommendation: Require >= 2 shared tokens or minimum Jaccard similarity before unioning, and filter group labels to tokens with `df >= MIN_GROUP_SIZE`.

#### F-09: Speculative `aw mem digest` Lacks Empirical Need (YAGNI)
- Severity: Important
- Lens: Zoom-Out
- Scope: Section 9 (lines 347–434), Section 17 (lines 583–588), Section 19.C (lines 614–615)
- Description: Section 17 acknowledges `mem digest` may see zero calls and could be dropped to a docs recipe, and §19.C notes shipping nothing is legitimate because the gap is unmeasured. Agents working in a file area already retrieve co-referenced notes via `mem query --ref <path>`.
- Recommendation: Defer F2 until usage data proves the bottleneck, or provide candidate grouping as an offline query recipe using existing `mem query --all`.

#### F-10: Over-Engineered Multi-File Directory Importer for Single-Stream Exporter
- Severity: Important
- Lens: Zoom-Out
- Scope: §8.1–8.3 (lines 207–243), §20 OQ-2 (lines 623–625)
- Description: Multi-file directory scanning, depth guards, and running byte caps are designed for an export structure that `aw` cannot generate (directory export was rejected in OQ-2).
- Recommendation: Restrict `aw mem import` to a single file path or stdin (`-`).

#### F-11: MCP `mem_import` Global Store Injection Hazard
- Severity: Important
- Lens: Design Critic
- Scope: §8.2 (lines 220–222), §22 item 6 (lines 658–661)
- Description: Exposing `global: bool` on MCP `mem_import` allows an agent in an untrusted repository to inject up to 5,000 notes into `~/.aw/memory.db`, polluting global memory across all local repositories without human authorization.
- Recommendation: Restrict MCP `mem_import` to `global = false` only; confine global imports to human CLI execution.

#### F-12: Missing Rust Structs and Function Signatures (Zero-Decision Violation)
- Severity: Important
- Lens: Implementation Readiness
- Scope: §13, §17 (lines 494–498, 575–577)
- Description: The plan omits concrete Rust type definitions and function signatures for `memory_write_batch`, `write_revision_in_tx`, `memory_digest_rows`, `normalize_import_note`, and the frontmatter DTO.
- Recommendation: Provide concrete Rust struct definitions and function signatures.

#### F-13: Missing SQL LIMIT and Scale Discrepancy in `memory_current_rows` and `memory_digest_rows`
- Severity: Important
- Lens: Design Critic, Codebase Details
- Scope: §7.1 (lines 123–137), §9.3 (lines 367–371), §13, §16
- Description: `memory_current_rows` has no `limit` argument or SQL `LIMIT` clause in `src/db/memory_query.rs#L51`, loading all rows into memory for Rust-side truncation. Similarly, `memory_digest_rows` has no LIMIT, yet `mem import` accepts 5,000 notes while digest is budgeted only for 1,000 notes.
- Recommendation: Add `LIMIT` clauses and parameters to both queries.

#### F-14: Unindexed Full Table Scan on Supersedes Cycle Guard
- Severity: Important
- Lens: Codebase Details
- Scope: §11 (lines 463–473), §12 (line 483)
- Description: Checking `A→B→A` requires reverse traversal (`WHERE superseded_by = ?`), but `schema_memory.sql` contains no index on `superseded_by` (only `WHERE superseded_by IS NULL`). Section 12 explicitly forbids adding new indexes, forcing full table scans per hop.
- Recommendation: Add a secondary index on `superseded_by` in `schema_memory.sql` or bound cycle traversal depth to 1 hop.

#### F-15: Batch Transaction Abort on Constraint Violations vs Single-Write Retries
- Severity: Important
- Lens: Codebase Details
- Scope: §8.4 (lines 252–256)
- Description: Single writes in `src/db/memory_write.rs#L121` retry by dropping and re-entering the transaction on `UNIQUE(key, rev)` violations. In a batch transaction, a constraint failure aborts the entire transaction.
- Recommendation: Pre-compute non-conflicting revision numbers within the transaction or document atomic batch rollback on key collision.

#### F-16: Non-Buildable Intermediate State in MCP Parity Tests Across Phasing
- Severity: Important
- Lens: Implementation Readiness, Codebase Details
- Scope: §13 & §17 (lines 507–508, 568–588)
- Description: Phasing splits `mem_import` (Phase C) and `mem_digest` (Phase D), but §13 mandates parity array updates in the same commit. Furthermore, `tests/mcp/main.rs` contains no tool assertions (they reside in `tests/mcp/parity.rs`).
- Recommendation: Specify incremental updates to `TOOL_SPECS` and `tests/mcp/parity.rs` per phase, and correct the reference to `tests/mcp/main.rs`.

#### F-17: Tag Rendering Gap in `mem get` for `[imported]`
- Severity: Important
- Lens: Codebase Details
- Scope: §8.7 (lines 290–293), §13 (line 499)
- Description: Section 13 assigns `[imported]` tag rendering to `src/memory/rank.rs`, which contains zero tag logic. `src/service/mem_ops.rs` (`mem get`) calls `store_tags` without passing `row.source`, meaning `mem get` cannot render `[imported]` without modifying `mem_ops.rs` and `store_tags`.
- Recommendation: Update `store_tags` in `src/service/mem_query.rs` to accept `row.source` and include `src/service/mem_ops.rs` in the modification ledger.

#### F-18: Missing Alternatives Evaluation for Core Proposals
- Severity: Important
- Lens: Design Critic
- Scope: Section 19 (lines 609–617)
- Description: Section 19 evaluates past decisions (D1, B2) rather than alternatives for the proposed features (fenced Markdown vs length-delimited; union-find vs pairwise Jaccard).
- Recommendation: Add concrete evaluations of rejected design alternatives for framing and clustering.

#### F-19: Ambiguities in Root Containment, Subdirectories, and `AW_SESSION`
- Severity: Important
- Lens: Comprehension
- Scope: §8.2–8.5 (lines 221–265), §12 (line 482)
- Description: The proposal leaves several edge cases unspecified: subdirectory handling in import directories; whether root containment applies to CLI; unset `AW_SESSION` fallback behavior; and the definition of `MEMORY_RESTORE_HINT`.
- Recommendation: Define explicit failure paths, clarify CLI path boundaries, define `AW_SESSION` fallback to `""`, and provide the `MEMORY_RESTORE_HINT` template.

---

### Suggestions & Nits

#### F-20: Missing Standard Input (`-`) Support on `aw mem import`
- Severity: Nit
- Lens: Zoom-Out
- Scope: §8.1 (lines 209–217)
- Description: `aw mem export` emits to stdout, but `aw mem import <path>` cannot read from standard input (`-`), preventing shell pipeline composition (`aw mem export | aw mem import -`).
- Recommendation: Accept `-` as `<path>` to read directly from stdin.

#### F-21: Colon-Less Keys Key Prefix Tokenizer Discrepancy
- Severity: Nit
- Lens: Codebase Details
- Scope: §9.3 (lines 374–375), §9.5 (line 419)
- Description: `str::split(':').next()` returns the entire string for colon-less keys, treating keys like `auth_login` and `auth_session` as sharing prefix `auth` unless colon presence is checked.
- Recommendation: Check `key.contains(':')` before extracting key prefixes.

#### F-22: Privacy Leak Risk for Workstation Paths in Logs
- Severity: Suggestion
- Lens: Design Critic
- Scope: §15 (lines 534–536)
- Description: External CLI paths logged to `.aw/usage.log` without hashing risk leaking developer workstation directories into Git-tracked repositories.
- Recommendation: Explicitly require all paths outside repository root to be recorded strictly as `fnv1a64` hex hashes.

#### F-23: Unspecified Dashboard Integration Contract for `procedure`
- Severity: Suggestion
- Lens: Implementation Readiness
- Scope: §10.2 (lines 448–449), §13 (line 501)
- Description: The proposal omits the badge CSS class and UI filter element for `procedure` notes in `index.html` and `app.js`.
- Recommendation: Specify the badge CSS class and dropdown option in the dashboard specification.

#### F-24: Undefined Acronym "EGM" in Glossary
- Severity: Suggestion
- Lens: Comprehension
- Scope: Title and §1 (lines 1, 20–31)
- Description: The acronym "EGM" appears in the title and table headers without expansion or definition.
- Recommendation: Add "Extended Goldfish Model" to the Glossary.

---

## Reconciliation

| Finding | Decision | Evidence |
| :--- | :--- | :--- |
| F-01 (Length-delimited framing) | PROPOSE FIX | Standard Git merge conflicts and CRLF conversion modify byte counts; length delimiters break portability. Replace with standard fenced frontmatter. |
| F-02 (JSONL/Markdown split-brain) | PROPOSE FIX | `src/db/memory_store.rs#L14` cites JSONL as the disaster recovery format; lacking JSONL import leaves existing exports unrestorable. |
| F-03 (Silent overwrite on import) | PROPOSE FIX | Re-stamping `created_at = now` on re-import overwrites local edits and destroys recency ordering; require `--force`. |
| F-04 (Semantic split header) | PROPOSE FIX | Add `## Detailed Implementation Plan` before Section 13 to satisfy review standards. |
| F-05 (Unannotated file ledger) | PROPOSE FIX | Convert Section 13 table to explicit `[Create]`, `[Modify]`, `[Delete]` ledger with exact paths and rationales. |
| F-06 (Missing JSON matter schema) | PROPOSE FIX | Comprehension gap cannot be contested; provide JSON schema and format example in §7.4. |
| F-07 (Undefined Git lifecycle) | PROPOSE FIX | `.gitignore#L2` ignores `.aw/`; specify target directory outside `.aw/` for Git-tracked memory files. |
| F-08 (Union-find percolation) | PROPOSE FIX | Single-token bridges cause transitively connected giant components; require >= 2 shared tokens or Jaccard threshold. |
| F-09 (Speculative `mem digest`) | ESCALATE | §17 Phase D and §19.C acknowledge zero-call risk and unmeasured need; owner must decide whether to defer F2. |
| F-10 (Directory import YAGNI) | PROPOSE FIX | Export does not produce directories (§20 OQ-2); simplify import to single file or stdin (`-`). |
| F-11 (MCP global import hazard) | PROPOSE FIX | Untrusted workspace calling MCP `mem_import` with `global: true` pollutes user-level `~/.aw/memory.db`. Restrict MCP to local. |
| F-12 (Missing Rust types/APIs) | PROPOSE FIX | Add explicit Rust structs and signatures for batch writes, normalization, and digest rows. |
| F-13 (Missing SQL LIMIT) | PROPOSE FIX | `src/db/memory_query.rs#L51` lacks `LIMIT`; add `LIMIT` clause to both queries. |
| F-14 (Unindexed cycle traversal) | PROPOSE FIX | `schema_memory.sql#L13-L33` lacks index on `superseded_by`; bound traversal depth to 1 hop. |
| F-15 (Batch constraint abort) | PROPOSE FIX | In a multi-statement transaction, `UNIQUE(key, rev)` aborts; pre-compute non-conflicting revisions before insertion. |
| F-16 (MCP parity phasing) | PROPOSE FIX | `tests/mcp/parity.rs#L37-L60` contains hardcoded tool lists; define incremental parity updates per phase. |
| F-17 (`mem get` tag rendering) | PROPOSE FIX | `src/service/mem_ops.rs#L19` calls `store_tags` without `row.source`; update `store_tags` signature and include `mem_ops.rs`. |
| F-18 (Missing alternatives) | PROPOSE FIX | Section 19 cites past decisions; add evaluations of alternatives for framing and clustering. |
| F-19 (Edge case ambiguities) | PROPOSE FIX | Define explicit error exits for subdirectories, CLI path boundaries, and `AW_SESSION` fallback. |
| F-20 (Stdin `-` support) | PROPOSE FIX | Support `-` in CLI import for piped workflows. |
| F-21 (Colon-less key prefix) | PROPOSE FIX | Guard prefix extraction with `key.contains(':')`. |
| F-22 (Log path privacy) | PROPOSE FIX | Mandate `fnv1a64` hashing for external CLI paths in journal telemetry. |
| F-23 (Dashboard procedure badge) | PROPOSE FIX | Add `badge status-neutral` and `<option value="procedure">` to dashboard specifications. |
| F-24 (EGM acronym in glossary) | PROPOSE FIX | Add "Extended Goldfish Model" to Glossary. |
