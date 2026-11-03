# kg Call-Chain Traversal Amendment — `kg path <from> <to>` (P5)

## Reading status

This approved, implemented design companion preserves the call-path rationale and original gate definitions.
Use Amendment 13 in the [canonical specification](2026-10-31_kg-rust-code-index.md) for the implemented traversal contract.

**Task and impact:** find a shortest route between two known symbols without reconstructing hops from separate queries.
Only resolved forward call edges participate; missing paths do not prove that runtime reachability is absent.

Read [usage](../usage.md) for navigation workflows and [evaluation](../EVALUATION.md) for public evidence limitations.
Original internal review records and frozen ground truth are unavailable publicly; retained results below are historical reports.


- Status: **Approved (rev 3)** — OQ5 approved by owner 2026-11-02
  (forward-only semantics). Six-reviewer Goldfish round (L1-L6, all NEEDS
  FIXES; 73 raw findings consolidated to 20, all dispositioned in
  internal review R5-P5, unavailable publicly), 2 fix cycles, round-2 disposition +
  amend-delta verification verdict APPROVE. Implementation landed with G17
  evidence retained internally (original G17 evidence unavailable publicly); canonical entry is
  Amendment 13 in `docs/designs/2026-10-31_kg-rust-code-index.md`.
- Date: 2026-11-01 (rev 3 2026-11-02)
- Golden references: RepoGraph (arXiv 2410.14684, **reported** numbers,
  not locally reproduced) as demand signal; dmoreq/pi-scope (GitHub, fetched
  2026-11-01) as the rejected-alternative reference. No golden-reference clone
  was checked out for this phase; all local evidence below was measured on
  existing kg indexes.
- Measured evidence: session 2026-11-01/02, binary `kg 0.1.0`
  (`target/release/kg`), tree clean at `dd2eafb` (P4/F4 landed); the tool
  flagged the index stale during E1 ("run kg build") — disclosed per row.
  Units rule: no kg token claims anywhere (byte/pointer counts only);
  external figures are quoted in their sources' own units and labeled
  reported. E7 is session-temp and not re-runnable from this document
  (probe never committed); its method is pinned below and the claim is
  stated with its method-sensitivity.

## 1. Evidence base

| # | Type | Measurement | Result | Command / source |
|---|------|-------------|--------|------------------|
| E1 | measured | Closure does not answer reachability-with-route: `callees --depth 3` output is sorted by `path:line` but carries no hop structure or chain identity | 20 bare pointers for `walk_children` (plus the staleness header); an agent cannot extract "does A reach B, via which call sites" without re-deriving hops by hand; `callers walk_node` returns 2 bare pointers, equally route-less | `KG_NO_JOURNAL=1 ./target/release/kg callees walk_children --depth 3` (20 pointer lines, header `kg: 1 of 1 result files changed since build — run kg build`); `KG_NO_JOURNAL=1 ./target/release/kg callers walk_node` -> `src/extract.rs:377`, `src/extract.rs:463` |
| E2 | measured | Per-file graph sizes (raw call edges deduped to unique pairs) | kg: 662 raw -> 415 unique over 231 edge-participating nodes; be-parser-geocoder: 527 -> 347 / 321; food-dispatch: 87 -> 81 / 99; be-mercury: 12 -> 11 / 13 | `sqlite3 -readonly <repo>/.kg/index.db "SELECT COUNT(*) FROM edges WHERE kind='calls' AND dst_id IS NOT NULL;"` and `SELECT COUNT(*) FROM (SELECT DISTINCT src_id, dst_id FROM edges WHERE kind='calls' AND dst_id IS NOT NULL);` and `SELECT COUNT(*) FROM (SELECT src_id FROM edges WHERE kind='calls' AND dst_id IS NOT NULL UNION SELECT dst_id FROM edges WHERE kind='calls' AND dst_id IS NOT NULL);` |
| E3 | measured | Cycles exist in real repos (visited-set is mandatory; a separate SCC feature is not) | kg: 6 self-loop edges over 3 symbols + 1 SCC of 6 symbols (`src/extract.rs:374` cluster: walk_children, walk_node, define_function, define_class_ts, define_method_ts, record_ts_arrow_function); 0 SCCs and 0 self-loops in the 3 other repos | self-loops: `sqlite3 -readonly <db> "SELECT COUNT(*) FROM edges WHERE kind='calls' AND src_id=dst_id;"` (-> 6); SCC: session-temp iterative Tarjan over the same indexes, re-verified independently by reviewers on the current indexes (not re-runnable from this doc) |
| E4 | reported | RepoGraph plug-in lifts agentic resolution and cuts per-instance cost across 4 agent frameworks. Correct attribution (fixed in rev 2): SWE-bench Lite GPT-4o — Agentless 27.33 -> 29.67, SWE-agent 18.33 -> 20.33; CrossCodeEval EM — code 10.8 -> 28.5, identifier 16.7 -> 36.1 | Demand signal: modest lift on SWE-bench Lite, large lift on repository-completion-style tasks — route-style navigation helps most where the agent must reach distant definitions | arXiv 2410.14684 (HTML v1, fetched 2026-11-02) |
| E5 | reported | CodeRAG-Bench: bare grep reaches recall 1.00 but the worst token-efficiency score in the paper's own unit; retrieval with dependency expansion improves that axis | kg's capped pointer contract already sits on this axis; `path` extends it to multi-hop | arXiv 2406.14497 |
| E6 | reported | aider's repo map ranks a def/ref multigraph with PageRank into a ~1k-token map — production adoption, no published A/B of PageRank vs trivial ranking at small scale | Context for the rejected alternative below | aider docs + HN 41485744 |
| E7 | measured (session-temp, not re-runnable) | PageRank ranking vs trivial fan-in diverges; magnitude is method-sensitive | Session probe (deduped resolved edges, damping 0.85, 50 iterations, weighted): Spearman 0.51-0.68, top-10 overlap 6-9/10 across 4 indexes; an independent reviewer probe (plain power iteration, id tie-break) on the kg index: 0.861, 5/10. Combined honest claim: divergence exists, magnitude depends on probe definition (0.51-0.86), overlap 5-9/10; no task-level evidence either way at < 600-file corpora -> analytics rejected on evidence absence, not measured inferiority | session Python probes over the same 4 indexes (never committed; see preamble) |
| E8 | reported | pi-scope (dmoreq) has no reachability/route tool; its navigation is LSP-dependent (lazily-started long-lived servers) and its injection layer violates the no-hijack invariant | Confirms the gap is not covered by the reference that prompted this phase | GitHub README fetch, 2026-11-01 |

## 2. Proposed feature — `kg path <from> <to>` (F5, Amendment 13)

One new read-only query. Question answered: *does `from` reach `to` through
resolved call edges, and via which call sites?* Nothing else changes.

### 2.1 Semantics

- Direction: forward only — `from` calls ... calls `to`. A reverse question is
  the same query with swapped arguments; no flag is added (**OQ5**).
- Edge set: BFS over the resolved call-edge set only —
  `edges WHERE kind = 'calls' AND dst_id IS NOT NULL` (rows of kind
  `imports`/`contains`/`inherits` are excluded even where `dst_id` is
  non-NULL). Raw multi-edges between the same `(src_id, dst_id)` pair are
  deduped; the representative edge is the one with the smallest `site_line`.
- Frontier discipline (pinned): FIFO queue, visited-on-enqueue, single seed
  (`from`), neighbors of a dequeued node expanded in
  `ORDER BY ds.qualname, df.path, e.site_line`; the node that first
  discovers (enqueues) a child becomes its parent. Output is deterministic
  across runs and across parallel/serial builds given an identical index.
- Depth cap: `--depth N`, clamp 1-10, default 10 (same clamp as `callees`).
  A BFS that exhausts the reachable set below the cap is a definitive
  negative; a frontier stopped by the cap is reported as such (2.2).
- No `--limit` argument (deviation, pinned): the output is a single shortest
  chain of at most 10 hops, so a result cap is dead machinery. See 2.2.
- Endpoint input contract (pinned): each endpoint is matched by `target_ids` exactly — the bare name (`s.name = ?1`) or the full printed qualname (`s.qualname = ?1`). Partial qualnames (`query.target_ids`) and pasted `path:line` pointers are rejected with `kg: no-results: symbol '<name>' not in index`; recovery paths (`kg search`, `kg symbols <file>`) are enumerated in the skill edits. Resolution order: resolve `from` then resolve `to`, each via `target_ids` unchanged (zero matches -> exit 1; multiple matches -> exit 2 with the candidate list and the failing endpoint's role named in the detail, e.g. `... as the from endpoint`). Only after both resolve unambiguously: if both resolved ids are equal -> exit 1 (same-symbol case, 2.2). `kg path <x> <x>` with an ambiguous `<x>` therefore exits 2, not 1.

### 2.2 Output contract (contract-bump entry required)

stdout byte sequence, in order:

1. header line (new, byte-pinned): `kg: path — resolved static edges only; dynamic dispatch may hide paths`; when the displayed files are stale, the existing staleness header is appended to the same first line after a newline, exactly as `dead` renders caveat + staleness (`src/query.rs:268-272`).
2. data lines: hop pointers in path order — bare `repo-relative/path:line` where path:line is the **call site** (`site_line`) of each hop; path is always the caller's file. Hop pointers never carry ` [ambiguous]` (the edge set excludes NULL-dst rows by construction). There is no result cap and no `showing N of M` line: chain length is bounded by the depth clamp (<= 10). **Deviation recorded**: `path` is the first query with no limit surface; rationale above.
3. final stdout line (new, byte-pinned): `kg: chain <from> -> <h1> -> ... -> <to> (K hops)` where every name is the **resolved full qualname** (not the bare segment, not the user-typed argument) — same-bare-name hops are real (`callers build` matches `src.build` and `tests.cli.build`), and resolved qualnames let the agent re-query any hop directly.

stderr: unchanged — error lines only, under the stable `kg: <class>: <detail>` taxonomy; no new class. Empty/negative outcomes (exit 1, class `no-results`, `<q1>`/`<q2>` rendered as the resolved full qualnames, `D` as the clamped depth):

- `kg: no-results: '<q1>' and '<q2>' resolve to the same symbol`
- `kg: no-results: no call path from '<q1>' to '<q2>' at any depth <= D (resolved edges only)` — frontier exhausted below the cap: definitive negative.
- `kg: no-results: no call path from '<q1>' to '<q2>' within depth D; a longer path may exist (resolved edges only)` — frontier stopped by the cap.

Ambiguous endpoint (exit 2, existing class `usage` — there is no `ambiguous` class in `src/error.rs`; live shape `kg: usage: ambiguous name 'build' — 2 candidates`): existing candidate rendering, detail gains the failing endpoint's role, byte-pinned in the contract bump. Exit codes 0/1/2/3/4/5/6/7 keep their existing meanings; no new codes.

### 2.3 Wiring obligations (per precedent, each 1:1)

- MCP (G10 parity): new tool `path`, `inputSchema {"from": string, "to": string, "depth": integer min 1 max 10 default 10}`, required `["from","to"]`; the MCP depth default equals the CLI default (10). Tool result byte-identical to CLI stdout.
- Journal (Amendment 6): `path` journaled; `args` gains the `to` key (`name` carries the `from` argument) and records `depth` unconditionally — same unconditional style as `callees`' `depth`. **Deviation recorded**: no `limit` key (no limit surface exists for `path`).
- Payload (Amendment 10): `path` joins the payload list. The payload line's `name` field is the `from` argument (same value as the usage.log `name` key); `to` travels only in usage.log `args`. `pointers` is the rendered hop set, byte-identical to the stdout data lines; `shown` = `total` = K. `usage --verify` needs zero verifier changes: the proposed rule is the existing bounds-only default branch for non-`callers` tools (`src/journal.rs:497` gates the name-segment check to `callers` only). Out-of-bounds on an unchanged file = FAIL; CHANGED/OK semantics untouched.
- Skill (`skills/kg-first/SKILL.md`) — exact edits, enumerated: (a) one routing-table row: `path` | reachability or route between two known symbols | instead of composing `callees` + `callers` by hand; (b) behavior-section entries for the four exit-1 shapes and the exit-2 role detail (swap-arguments guidance for the reverse question, `kg search`/`kg symbols <file>` recovery for endpoint misses, and compounding calibration: a K-hop route fails if any single hop edge is unresolved — with 10-48% of raw call sites unresolved per the evaluation corpora, repeated no-path results likely reflect that compounding, so fall back to composing `callees`/`callers` or grep rather than re-querying `path`); (c) frontmatter description gains `path`. Enforcement honesty: the kg repo has NO automated drift coverage for this file; the agent-skills sync stays an owner pre-commit step per the F3 limitation (G15 lives there and is `KG_BIN`-gated).
- Schema: **no change** — the feature reads existing `edges` rows only; v4 stays v4.

### 2.4 Gate G17 (kill-able)

- Correctness: `cargo test path_seed` — exactly 10 fixtures, each asserting exact stdout bytes: (1) 3-hop chain; (2) diamond tie-break (pinned parent rule output); (3) cycle A->B->C->A (A reaches C in 2 hops, terminates); (4) disconnected pair -> exit 1 exhaustive-negative bytes; (5) depth-cap miss (real path at 11) -> exit 1 cap-truncated bytes; (6) from == to -> exit 1 same-symbol bytes; (7) ambiguous `from` endpoint -> exit 2 with role bytes; (8) ambiguous `to` endpoint -> exit 2 with role bytes; (9) multi-edge representative (two call sites, one pair -> min-`site_line` pointer); (10) stale-index header bytes on `path` output. Bar: 10/10.
- Frozen GT: internal path ground truth (unavailable publicly) — hand-verified reachable pairs (with expected hop counts) plus at least one no-path pair per repo, on the GT repos (kg + vihap worktrees, as G14). Construction protocol: every hop edge of every GT pair verified `EXTRACTED` in the index at freeze time. Historical pass command: `python3 tests/scripts/path_recall.py --pairs <internal-path-ground-truth> <kg-binary> --kg-root <kg-repo> --vihap-root <vihap-worktree>` (requires unavailable internal inputs). Bar: 100% agreement. Mismatch routing (pinned): a hop edge missing or moved at re-run = resolution-attributable -> routed to G2 retune, NOT a path kill; a BFS-attributable failure (wrong parent assignment, non-determinism, wrong byte shape) = path kill.
- Latency: `tests/scripts/timing.sh` gains a `path` case; budget < 20 ms best-of-7 on the GT repos (existing queries measure 5-10 ms; BFS adds one pass). The 2,025-file synthetic corpus from Amendment 11 has no committed generator; P5 commits one (`tests/scripts/make_synth_corpus.py`) and the < 20 ms budget is re-stated on it at gate day (Amendment 11's 4.6 MB index is the scale reference; the "<= 1M edges" headroom claim is est., not measured).
- Kill action (full residue): code removed; the contract-bump entry withdrawn; MCP tool, journal `to` key, payload-list entry, and skill paragraph withdrawn; `path-gt.jsonl` + `path_recall.py` + `make_synth_corpus.py` archived internally (archive unavailable publicly); no schema or data migration (v4 stays v4).

## 3. Rejected alternatives

| alternative | why rejected |
|---|---|
| pi-scope LSP navigation (definition/references/hover) | needs long-lived language servers: violates invariant 2; static pointers + grep fallback already cover the slice honestly |
| PageRank-weighted traversal order (aider-style god nodes first) | E7: divergence from fan-in exists but its magnitude is method-sensitive (Spearman 0.51-0.86 across probe definitions) and no task-level evidence at this scale; adds an unmeasured choice into a determinism-sensitive path |
| `kg chain <sym>` (parent-tree closure instead of endpoint pair) | answers a different question (full ancestry), output explodes; `callees`/`callers` already compose it; YAGNI |
| Bidirectional BFS | unmeasured benefit at the measured index sizes (E2: <= 415 nodes); plain BFS keeps the implementation minimal |
| `--all-paths` enumeration | combinatorial output, no agent demand signal; YAGNI |

## 4. Open question for the owner

- **OQ5**: confirm forward-only semantics (reverse query = swap arguments,
  no flag). If the owner wants a `--dir` flag instead, the change is BFS edge
  direction + output text only; gates are unaffected.

## 5. Implementation plan (post-approval)

1. Pre-step, own commit (pre-existing, not caused by P5): repair
   `tests/scripts/write_path_check.sh` — its exclusion pattern
   (`fs::rename(db::db_new_path`) no longer matches the qualified call
   (`std::fs::rename(crate::db::db_new_path(...)`), so G5 is red on the
   committed tree (`src/journal.rs:10,172,208,564`, `src/build.rs:313`
   flagged; all writes are `.kg/`-scoped and the invariant itself holds —
   the enforcement script is stale).
2. `src/query.rs`: `pub fn path(...)` reusing `target_ids`, `clamp_limit`
   removal not needed (no limit), `QueryOutput` (~150-220 lines with BFS +
   pinned frontier discipline + rendering).
3. `src/cli.rs` + `src/mcp.rs`: `path` surface, 1:1 (no `--limit`).
4. `src/journal.rs`: `to` arg key; payload-list membership (no verifier change).
5. `tests/fixtures.rs`: `path_seed` (the 10 G17 fixtures);
   internal path ground truth (unavailable publicly) + `tests/scripts/path_recall.py` +
   `tests/scripts/make_synth_corpus.py`.
6. Docs: canonical Amendment 13 + G17 rows, README usage block, skill edits
   per 2.3.
7. Gates re-run per the change workflow; commit `feat(p5): ...`.
