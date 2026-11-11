# Concepts Brief — Dashboard Truthfulness & Minimalism

- Date: 2026-11-11
- Topic: `dashboard-truthfulness`
- Source artifacts: EGM review bundle (`.tmp/ux-review/report.md`, `findings/consolidated.json` — 18 personas, 159 findings), `docs/designs/2026-11-11_dashboard-truthfulness.md` (spec)
- Gate 1 lock (owner, 2026-11-11): resolve all 159 findings; apply minimalism cuts; spec committed under `docs/designs/`.

## Anchored hypothesis

The `eg` dashboard runs and its server-side aggregation is mostly correct, but it misrepresents the tool in four P0 places (monitor tool table reads payload keys that are never emitted; a broken tab-id call blanks the console on drill-down; the Session Recall KPI contradicts its own tooltip; candidate overlap is hardcoded at 100%), and a further 21 P1s make agent-driven numbers unverifiable (client/server estimate drift, contaminated recall counts, hit-vs-candidate conflation, wrong-store mutation). Resolving all 159 findings under the repo's minimalism doctrine yields a smaller surface whose every number is live-repo-attributed, estimate-labeled, and quote-able by both humans and agents.

## Consumers

- Human operator: occasional glance at `eg dashboard`, terminal `eg monitor`.
- AI agents: primary consumers of `/api/summary|reuse|memories` payloads and journal-adjacent text.

## Hard constraints (repo invariants)

Zero dependencies; vanilla HTML/CSS/JS assets embedded via `include_str!`; ≤500 LOC per Rust source/test file; no emoji/sparkline; `Cache-Control: no-store`; live-inventory journal rule (only live-repo entries journaled/read/pruned); `eg` MSRV 1.90.

## Out of scope

- Replacing the dashboard with CLI-only output (owner: dashboard stays, gets smaller and truthful).
- Changing journal write semantics or the live-inventory/purge rules (settled Nov 10–11).
- Any new dependency, framework, or build step for the assets.

## Problem areas (159 findings deduplicated → 14 areas)

| Area | Findings (examples) | Severity peak |
|---|---|---|
| A. Monitor tool-table key drift | OK%/p50/SAVED read `reliability_pct/p50_ms/saved_tokens`; server emits `calls/errors/shown/p50` | P0 ×4 personas |
| B. Drill-down blank console | `switchTab('graph')` invalid id hides all tabs | P0 ×4 |
| C. Memory KPI semantics | Recall denominator includes untagged `default`; tooltip contradicts code; currency vacuous 100% | P0/P1 |
| D. Overlap hardcode | `overlap_pct` = 100 on all 50 candidates incl. D4 wrappers | P0 |
| E. Client/server parity | Token coefficients 10x drift; read/write class drift; empty-result pricing; UNATTRIBUTED client mirror; p50 never patched live | P1/P2 |
| F. Interaction | Filter selections reset by poll/SSE; Touch Anchor wrong store; global-note refs falsely BROKEN; SSE replay double-count after prune | P1/P2 |
| G. Performance | `/api/reuse` 2.2s uncached per request; full-file snippet reads; log re-parse per request | P1/P2 |
| H. Truth labeling | Estimates unlabeled in payloads; "N recalls" presented as exact; Duplicate-Hits vs Candidates conflation; no time window | P1/P2 |
| I. Copy & terminology | Tooltip/code mismatches; jargon; Status vs Reliability; "1 refs"; USP leak; hardcoded v0.5.3 | P1-P3 |
| J. Accessibility | Click-only rows; tablist ARIA missing; no aria-live; contrast (2.88:1 dark btn-touch); `--intent-warn` undefined; tooltip keyboard/touch gap | P1/P2 |
| K. Redundancy | 21 KB dead `recent_queries` payload; dead fields ×10; dead CSS ×3; unreachable fallbacks; duplicated agent tables | P2/P3 |
| L. Fresh/empty states | Vacuous 100.0% currency; "Multi-repository" for 0–1 repos; "Codebase is clean" on filter-zero; stuck Loading rows | P2 |
| M. Tool-frequency presentation | `build` inside graph KPI skews p95; per-tool errors/p95 hidden; 9/19 tools homeless | P2/P3 |
| N. TUI parity remainder | R:W drops mem_get; stream wrap; filter scope mismatch; silent fresh state | P2 |

## Strongest objections (owner must resolve)

1. **Client-side aggregation mirror should be retired, not fixed** (contrarian-02/03, redundancy-03): `patchSummaryState` duplicates the server fold in 180 lines, drifts on coefficients, resurrects the retired bucket, and is overwritten by its own 3s-debounced refetch. Proposed resolution: keep only immediate-recent-row insertion client-side; all aggregates come from refetch (3s debounce already exists). Risk: one-refresh latency for KPI increments.
2. **Cut agent tables on tabs 02/03, cut the "Graph Workspace Scope" KPI** (redundancy-09, component-value-07): the same agent table on three tabs triples maintenance; the Scope tile duplicates the repos table row count. Proposed resolution: cut; tab 01 remains the single agent/scope home.
3. **`/api/reuse` per-request detector scan is architectural** (performance-01): fixing it properly needs an mtime-keyed cache; the alternative (compute-on-tab-open only) trades live-ness for simplicity. Proposed resolution: mtime-keyed cache inside `data/reuse.rs`, invalidated by index/log mtimes.

## Decision log (Gate 1, owner)

- Resolve all 159 findings including P3 nitpicks and structural cuts.
- Minimalism direction accepted: retire client SSE aggregate mirror, cut duplicated tables/tiles, remove dead payload fields.
- Spec artifacts committed under `docs/designs/`.
- Dashboard is kept; replacement by CLI-only output is out of scope.
