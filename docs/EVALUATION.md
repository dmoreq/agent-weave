# kg — Evaluation

All numbers in this document are measured, with the command attached, against
the binary being shipped. Detailed per-run evidence lives in
[`docs/baselines/`](baselines/) (internal records; they use real corpus-repo
names, this document does not). Nothing here is projected or extrapolated
without being labeled as such.

**TL;DR**

| question | answer (measured) |
|----------|-------------------|
| Are the pointers real? | 777/777 emitted pointers verified against source lines across 6 corpora; ground-truth recall and EXTRACTED-tier precision 100% on 3 frozen ground-truth sets |
| Is the query loop fast? | 5-10 ms end-to-end per invocation (spawn floor ~6 ms) at any index size measured; independent of DB size after the integrity-scan removal |
| Is the build fast? | 0.06-0.46 s fresh on 27-596-file corpora; 0.13 s on the self-index; peak RSS <= 40 MB |
| Does it make a coding agent better? | navigation recall 98.6% vs 81.9% grep-only, at flat token cost |
| Does it make an agent cheaper? | only when routed: query-first policy cut input tokens **-27.3%** (1,058,613 → 769,306 per run) at unchanged recall |
| Does it answer semantic questions? | No — static resolution only. Out of scope by design. |

Corpora are referred to by neutral labels throughout:

| label | description | files | symbols | edges |
|-------|-------------|-------|---------|-------|
| corpus A | ~102k-LOC Python address-parsing monorepo | 252 | 3,291 | 9,128 |
| corpus B | TS/TSX product prototype | 135 | 488 | 1,848 |
| corpus C | multi-tenant data-collection backend (Python) | 52 | 385 | 717 |
| corpus D | Python DAG/notebook generator | 596 | 3,119 | 3,849 |
| corpus E | Python map-search service | 150 | 1,381 | 3,486 |
| corpus F | Python data-science pipeline | 27 | 103 | 166 |
| kg self | this repository (Rust) | 24 | 306 | 904 |

## 1. Methodology

- **Frozen tasks + ground truth**: 5 navigation tasks (hub callers, state-hub
  callers, signature-impact, flow trace, transitive callees) with 60
  owner-delegated, agent-verified call-site rows frozen before measurement
  (`baselines/tasks.md`, `baselines/ground-truth*.md`, `baselines/expected/`).
- **Correctness scoring**: mechanical checker (`baselines/harness/check.py`)
  compares answer JSON against frozen expected sets — precision, recall,
  ordering, grouping. No hand-picking; disagreements audited individually.
- **Agent A/B**: 42 instrumented agent runs across two hosts (a TUI coding
  agent and an MCP-connected CLI agent), two arms (grep-only vs kg-available),
  with per-call byte accounting and per-session token accounting taken from
  host-recorded stores, not self-reports (`baselines/grok-a-b-2026-11-01.md`).
- **Reproduction**: every record carries its commands; run manifests and
  session IDs are committed (`baselines/grok-a-b-2026-11-01/`).

## 2. Performance ceiling pass

A measured optimization pass (spec Amendment 11) against a 2,025-file
synthetic corpus (index 4.6 MB) and the self-index:

| surface | before | after | factor |
|---------|--------|-------|--------|
| `kg build --force` wall | 1.16 s | 0.27 s | 4.3x |
| — parse phase (single- → 8-worker parallel) | 0.66 s | 0.11 s | 6x |
| — db phase (fsync per 500-row chunk → single transaction) | 0.39 s | 0.09 s | 4.3x |
| `kg callers` end-to-end | 15.1 ms | 7.6 ms | -50%, flat in index size |
| `kg hotspots` (re-parsed the repo per call) | 409-781 ms | 31 ms | 13-25x |
| `kg report` | 560 ms | 44 ms | ~12x |

The query-path win came from removing a full-B-tree `PRAGMA quick_check` from
every open (0.7-1.1 ms/MB of index) — corruption still exits 3 via SQLite's
own corruption reports; deep integrity remains `kg doctor`. The hotspots win
came from computing cyclomatic/LOC once at build time (schema v3, additive)
instead of re-parsing every file per invocation.

## 3. Multi-repo verification

Fresh force builds on six real codebases + the self-index (post-optimization
binary):

| check | result |
|-------|--------|
| Parse errors | 0 across all corpora |
| Pointer authenticity (callers/impact/dead/symbols spot-checked against source lines) | **777/777 (100%)** |
| Fast path | second build carries 100% of files on every corpus |
| Corrupt-index contract | non-SQLite and truncated files both exit 3 with the contract message |
| Dirty closure | callee-side and caller-side edits both preserve cross-file edges; table totals invariant |
| MCP vs CLI parity | tool text byte-identical to CLI stdout (1,793-byte sample) |
| Staleness | silent on fresh indexes; header fires after a live edit |
| Extraction determinism | parallel rebuilds produce identical table content (files+symbols+edges) |

Record: `baselines/multi-repo-verification-2026-11-01-post-a11.md`.

## 4. Correctness gates

| gate | what it measures | result |
|------|------------------|--------|
| G2 precision/recall | `kg callers` vs 60 frozen ground-truth call sites on 3 GT sets | recall 100% (29/29, 20/20, 10/10); EXTRACTED-tier precision 100% — all PASS |
| G3 query latency | p95 over repeated pointer queries | 9.8-10.1 ms (limit 20 ms) |
| G11 payload conformance | 1 payload line per successful pointer call, byte-equal pointers, verify FAIL=0 | 594 pointers verified, FAIL=0 |
| G12 perf-pass conformance | corrupt exit 3, v3 carry-forward, hotspots from index, parallel == serial | PASS (91/91 tests) |
| G13 outline utility | `kg symbols --outline` replacing file reads in the task battery | 41.7% tool-call reduction vs no-outline control, zero correctness delta |
| G14 symbol search | `kg search` recall vs 20 frozen NL queries / 28 expected symbols | 0.850 recall — PASS |

## 5. Agent A/B — the measured case for kg

42 instrumented runs total; the headline experiment is 20 runs on corpus A
(5 frozen tasks x 2 arms x 2 runs, one agent host), replicated with a
query-first routing policy (+10 runs) and cross-checked on a second host.

### 5.1 Correctness first

| | grep-only arm | kg arm |
|--|---------------|--------|
| Gating-task recall (T1-T3, mean of 6 runs) | 81.9% | **98.6%** |
| Re-export/indirect callers (T2) | 50% / 50% — missed them | **100% / 100%** |
| Flow-trace task (T4, recall-only) | 0/2 runs hit the stage classes | 2/2 hits in the routed arm |
| Transitive callees (T5, recall-only) | 0/2 runs | 100% (cross-host routed run) |

The grep arm's misses are structural: re-export chains and indirect call
sites that name-only tools cannot follow. They are exactly the resolution
algorithm's design target.

### 5.2 The economics surprise (honest)

The original scripted-harness anchor predicted -50% tool calls / -67% bytes.
**It did not replicate with a free-running agent.** Left to its own habits,
the agent used kg as a supplement (3.8 kg calls inside 35.7 total) and spent
MORE tool calls (+73%) exploring and verifying — tokens came out flat.

What kg bought at flat cost was correctness (5.1). To also capture the token
win, the agent must be **routed**: a binding prompt clause — run kg first for
who-calls/impact/callees/flow questions; grep only for what kg cannot answer.
Measured effect of routing (n=10 vs n=10, same host):

| metric (mean/run) | grep-only | kg, unrouted | kg, query-first routed |
|-------------------|-----------|--------------|------------------------|
| input tokens | 1,058,613 | 1,028,665 | **769,306 (-27.3%)** |
| output tokens | 13,773 | 13,403 | 10,850 (-21.2%) |
| model calls | 20.2 | 19.9 | 17.2 (-14.9%) |
| kg calls | 0 | 3.8 | 8.8 |
| gating recall | 81.9% | 98.6% | 98.6% |

The saving is NOT from kg's output being smaller (tool outputs are ~2-3% of
input tokens). It compounds from a shorter trajectory: one kg call replaces a
grep-noise loop, the transcript stays short, and every subsequent model call
re-sends less. This is why routing must be enforced by prompt/skill, not
hoped for.

### 5.3 Host dominance

The same routed policy on a second host/model (12 runs) kept compliance
(4.4 kg calls/run) and the structural wins (T2 100%, T5 100%), but gating
recall fell to 85.9% — vs 98.6% on the first host with identical tool
access. kg supplies correct data; the ceiling on final answers is the
agent's own capability. Skill files standardize tool behavior; they do not
raise model capability.

### 5.4 Where kg does not help

- Semantic questions ("where is X behavior implemented") — out of scope.
- Dynamic dispatch and untyped attribute calls: 10-48% of raw call sites per
  corpus remain unresolved by design; the tool says so and recommends grep.
- Notebook-heavy corpora (corpus D/F class): resolution coverage ~10-19%,
  leverage concentrates in `hotspots`/`report` instead.

## 6. Limitations and threats to validity

- **n=2 per task cell** in the A/B; T4/T5 split 1/2 within the routed arm —
  per-task numbers are directional, the aggregate recall delta is the robust
  signal.
- **Answer-contract artifacts** independent of arms: T1 `grouping` and T2
  `ordering` keys failed system-wide because task prompts lacked JSON-shape
  examples; those two sub-scores under-report all arms equally.
- **Cross-host token counters differ in scope** (what each host counts as
  input) — token comparisons are made only within one host.
- **Single-repo A/B anchor**: the deep instrumentation ran on corpus A; the
  other corpora have pointer-authenticity and gate coverage, not full A/B.
- **Warm-cache timings** on a local SSD; cold-cache numbers will be worse
  but preserve the ratios.

## 7. Evidence map

| claim | record |
|-------|--------|
| perf ceiling pass | `baselines/` CHANGELOG section (Amendment 11) |
| multi-repo verification | `baselines/multi-repo-verification-2026-11-01-post-a11.md` |
| G2/G11/G12 gates | `baselines/ground-truth*.md`, gate rows in the spec |
| G13 outline | `baselines/g13-outline-2026-11-01.md` |
| G14 search | `baselines/g14-search-2026-11-01.md` |
| agent A/B + routing + replication | `baselines/grok-a-b-2026-11-01.md` (+ `baselines/grok-a-b-2026-11-01/` manifests and audits) |
| tasks, ground truth, harness | `baselines/tasks.md`, `baselines/ground-truth*.md`, `baselines/harness/` |

Internal records use real corpus-repo names; this document intentionally does
not.
