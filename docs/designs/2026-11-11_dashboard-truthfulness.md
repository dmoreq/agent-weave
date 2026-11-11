# Dashboard Truthfulness & Minimalism

- Author: quy.doan (with EGM 18-goldfish evidence)
- Status: Approved (Gate 2 resolved 2026-11-11)
- Date: 2026-11-11
- Predecessor: `docs/designs/2026-11-11_concepts-brief_dashboard-truthfulness.md`, EGM bundle `.tmp/ux-review/` (159 findings: 8 P0 · 21 P1 · 79 P2 · 51 P3)
- Revision 2: incorporates Gate 2 (5 lenses, 108 findings) and owner resolutions.

## Overview

The `eg` web dashboard (3 tabs, embedded vanilla assets) and `eg monitor` TUI report journal telemetry to a human operator and AI agents. An 18-persona EGM audit found the surfaces run but display wrong data in four places, drift from the server in client-side mirrors, and carry redundant structure. Gate 2 (Contrarian / Ergonomics / Observability / Maintenance / Security lenses) then found the first spec draft's own defects: a CLI rank regression from the overlap change, an unmigrated Touch body, index-keyed selection that can mutate the wrong note, a debounce that starves, fail-open empty payloads, and several High-severity injection/CSRF/CORS surfaces. Revision 2 resolves all of them.

## The Problem

Operators and agents read numbers that are sometimes wrong (monitor shows 0.0ms/0/100.0% for real tools; recall shows 50.0% where tagged-only truth is 0%; overlap shows 100% for D3/D4 constants), sometimes unverifiable (estimates presented as measured, no window, no schema version), and sometimes contradictory on one screen ("Duplicate Hits" 618 vs "330 candidates"). Interaction defects (blank console guard missing, filter resets, Touch mutating the wrong store, selection drifting to another note) erode trust. The HTTP surface also trusts any Host/Origin and interpolates unescaped strings into `innerHTML`, so any visited webpage can read telemetry or forge a Touch write.

## Goals

- Zero displayed falsehoods: every P0 cluster and every Gate-2 Critical eliminated and pinned by named tests.
- One classification and pricing source per concept: `ToolClass` and `ExitClass` in Rust; client renders payloads only; parity tests pin `GRAPH_TOOLS` and memory classes against `app.js`.
- Truth labeling: `estimated: true` attached only to derived fields; `window: "retained_journal"`; `generated_at`; `schema_version` on all three payloads; golden key-set tests assert both presence and absence.
- Durable mutation: `mem touch` gains `anchored_at` (memory schema v2), keeps `created_at`, requires the displayed `rev` (409 on skew), resolves `store_id` exactly, and never falls back across stores.
- Local-tool hardening: no inline handlers, CSP + defensive headers, Host/Origin/Content-Type validation, no wildcard CORS, bounded requests/streams/snippets/journal lines.
- Accessibility: keyboard-operable rows/cards with roving tabindex, complete tabs pattern, aria-live limited to state transitions, AA contrast both themes.
- Performance: `/api/reuse` served from an mtime-keyed cache computed before the read, never caching errors; bounded snippet I/O; lazy reuse fetch; bounded refresh schedule.

## Non-Goals

- Replacing the dashboard with CLI-only output.
- Changing journal write semantics, live-inventory/purge rules, or the usage-journal schema.
- Reworking D1–D4 detection algorithms beyond the D3/D4 overlap evidence label.
- Remote/multi-user dashboard operation (host validation exists to keep it loopback-only).
- Committing the EGM evidence bundle (owner chose chat + `.tmp`); the spec's appendix carries the mapping.

## Open Questions

- None. Owner resolved Gate 2 on 2026-11-11: agent tables kept + deduped renderer; `anchored_at` schema v2; full security hardening.

## Critical User Journeys

### CUJ 1: Operator opens the dashboard

- Given: journal entries from ≥1 live repository; dashboard built from this spec
- When: the operator opens http://127.0.0.1:7890
- Then: every KPI/table number matches journal semantics (tagged-only recall, estimate-marked savings, real per-tool p50/p95/errors, `generated_at` age visible), in both themes, and a stderr line records each request.

### CUJ 2: Drill-down and filtering (failure journey repaired)

- Given: any tab with repo rows and the FILTERS state object
- When: the operator clicks a repo row (mouse or Enter/Space) and a live event arrives
- Then: the tab narrows in place, the filter survives the SSE refetch, and `switchTab` ignores any unknown key with a console warning — no code path can blank all tabs.

### CUJ 3: Touch a memory note

- Given: a note rendered in the dossier with `ref_status: modified`
- When: the operator clicks "Verify" (formerly Touch Anchor)
- Then: the note's own store is resolved by `store_id`, the request carries the displayed `rev`, `anchored_at` becomes now while `created_at` is unchanged, the response names store and rev, and a 409/404/503 is rendered inline with the store name — never a silent cross-store write.

### CUJ 4: Live event stream

- Given: an open tab with SSE connected
- When: a new invocation is journaled, then the log shrinks (prune) or rotates
- Then: one row is inserted; the refetch scheduler fires at most once per 3 s and at least once per 10 s; shrink/rotation emits `event: reset`, the client refetches immediately and announces "Live stream reset", and totals never double-count.

### CUJ 5: Monitor TUI

- Given: the same journal state
- When: `eg monitor` runs with or without `--repo/--errors-only`
- Then: the TOOL table shows real OK%/p50/SAVED per tool from canonical payload keys, the header version equals the binary, sessions with no tag show "—" for recall, R:W includes `mem_get`, and no internal spec labels appear.

## Technical Plan

Stdlib thread-per-connection server (unchanged architecture) with: request logging + startup provenance to stderr; Host allowlist; `POST /api/memory/touch` hardened (Content-Type, Origin/Host, `{key, store_id, rev}`, body cap 4 KiB); three payload builders emitting `schema_version`, `degraded`, `errors[]`, `generated_at`, `window:"retained_journal"`, `estimated` on derived fields only; `/api/reuse` mtime cache (key computed before read, swap under lock, errors never cached, `ENGRAM_DASHBOARD_NO_CACHE=1` kill switch, `cache:{hit,age_ms}` in payload); one shared journal-tail helper with `(dev,ino,len)` identity and a named SSE `reset` event; client renders payloads only with a single `FILTERS` object, id-keyed selection, `addEventListener` handlers (no inline `onclick`), CSP-compatible assets, and a bounded refresh scheduler (3 s debounce + 10 s max wait). Memory store: additive `anchored_at` column (MEMORY_SCHEMA_VERSION 2, `ALTER TABLE` on write-path open; readers `COALESCE(anchored_at, created_at)`), touch sets `anchored_at` only.

## Alternatives Considered

### Alternative A: Apply minimalism cuts + Gate-2 hardening (CHOSEN)

- Pros: resolves all 159 EGM findings and all Gate-2 Critical/High; shrinks payload (~30 KB) and client code; closes XSS/CSRF/CORS/exfil paths; keeps every surface truthful at the source.
- Cons: touches db schema (additive), adds security plumbing (Host/Origin checks, CSP), and expands test surface.
- Reason for acceptance: owner approved all three Gate-2 resolutions; the security findings are High severity on a loopback tool every visited webpage can reach.

### Alternative B: Truth-only patch, defer hardening and cuts

- Pros: smallest diff (~200 lines + tests).
- Cons: leaves 79 P2 findings, the XSS/CSRF/CORS holes, and the fail-open payloads; contradicts the owner's "resolve all" directive.
- Reason for rejection: owner selected full resolution; security lens rated the holes High.

### Gate-2 resolutions applied

| Gate-2 finding | Resolution |
|---|---|
| Contrarian 2 / Maint 14: `Option` sort drops D4 from CLI top-10 | Sort keeps D4 at rank 100 via a helper; payload emits `overlap_pct: null` + `overlap_applicable: false`; CLI hint unchanged; new CLI test proves a D4 fixture survives `--limit 10` |
| Maint 2: D3 hardcodes 90 | D3 emits `None` + `overlap_applicable:false` with an evidence tag; card/dossier/tests updated |
| Contrarian 10: monitor fix direction | CONTESTED with evidence: monitor reads the canonical `p50/errors/saved_tokens` keys (no duplicate `p50_ms` aliases); typed key-set test pins every monitor-read field |
| Contrarian 9 / Ergo 17: agent tables are not duplicates | Owner: KEEP tabs 02/03 agent tables, dedupe via one parameterized renderer; Scope KPI tile cut remains |
| Ergo 1 / Maint 17 / Security 21: touch destroys created_at | Owner: add `anchored_at` (schema v2); relabel button "Verify"; 409 on rev skew |
| Ergo 3 / Maint 24: index-keyed selection | Selection keyed by `store_id:key:rev` (notes) and `symbol+canonical` (reuse); re-resolved after rebuild; cleared with announcement |
| Contrarian 7 / Obs 10 / Maint 7: debounce starvation | Scheduler: leading+trailing, max wait 10 s; test with sub-debounce event cadence |
| Contrarian 8 / Ergo 13 / Maint 25: switchTab guard | `TAB_KEYS` allowlist inside `switchTab`; agent-table call sites deleted same phase; static test forbids `switchTab('graph')` |
| Contrarian 4 / Maint 11: false window/estimate labels | `estimated` only on derived fields; `window: "retained_journal"`; tests assert flag placement |
| Maint 12 / Obs 12: no schema version | `schema_version: 2` on all payloads + golden key-set tests (presence and absence) |
| Obs 2 / Security 12: fail-open empty payloads | `degraded: true` + `errors:[{store,stage,message}]` + UI banner; corrupt-index test asserts non-clean |
| Obs 3/4/5 / Security 11: cache correctness | Key computed before read; compute outside lock, swap under lock; errors never cached; mtime+len of index and both log generations; `ENGRAM_DASHBOARD_NO_CACHE=1`; `cache:{hit,age_ms}` payload |
| Obs 7 / Maint 8: triplicated tail | One `tail` helper for SSE + monitor TTY + linear; `(dev,ino)` identity; named `event: reset`; monitor shrink no longer replays |
| Maint 9 / Contrarian 14: classification copies | One `ToolClass` in Rust (summary + monitor consume); `summary.rs` classification aligned (`mem_export` read; `mem_delete`/`mem_gc` write); parity test enumerates `GRAPH_TOOLS` and mem lists from `app.js` |
| Maint 3: per-tool saved mismatch | ToolAgg accumulates `ok_calls/ok_shown/ok_results`; test `sum(tool saved) == saved_tokens` |
| Maint 10 / Contrarian 17 / Ergo 18: recall sets/units | Both sets fed by the tagged predicate; `tagged_session_count` distinct; monitor "—" when untagged; invariant test |
| Security 1/17/18: XSS + CSP | Remove all inline handlers; `escapeHtml` adds `'`; CSP `script-src 'self'`, nosniff, frame-ancestors none, referrer no-referrer; hostile-key render test |
| Security 2/4/16: CSRF, rebinding, --local | Content-Type required (415), Host allowlist (403), Origin check, `--local` resolves root at startup; rejection tests |
| Security 3: wildcard CORS | Remove `Access-Control-Allow-Origin: *` from `/events`; stream is same-origin only |
| Security 8/9/10: unbounded inputs | memories LIMIT 200 current notes + body truncation flag; header/request-line caps (8 KiB/64); SSE cap 8 + 503; journal per-line cap 256 KiB + skipped counters |
| Security 13/19/20: snippet/ref/git | Snippet containment (canonical prefix, reject absolute/`..`); LIKE ESCAPE; touch-scoped git hardened flags |
| Security 15 / Maint 1: store identity | `store_id` = canonical `memory.db` path; exact-match lookup; basename only as display label; collision test |
| Ergo 14 / Maint 15: ARIA tabs static | `switchTab` owns `aria-selected` + roving tabindex; arrows/Home/End; static checks |
| Ergo 15: aria-live flood | Announce only state transitions (connection, reset, first error, verify), coalesced ≤1/s |
| Ergo 16 / Maint 6 / Security 22: FILTERS rules | FILTERS sole authority; validate restored values against options; invalid → ALL + dismissible notice; `textContent` only; memory-only |
| Ergo 19 / Obs 9: swallowed fetch errors | Per-panel error state with Retry + last-success age; `r.ok` checked; payload-age chip |
| Ergo 6/8/9/23 / Obs 6: Touch UX | Inline error (no `alert`), store+rev in response, "Verify all N" bulk action with confirmation |
| Maint 16: removed fields still read | Delete recall banner + `prev_*` reads; static test asserts removed field names absent from app.js |
| Maint 23: dead element writes / id contract | Remove `metric-active-*` writes with the Scope tile; test every `getElementById` literal exists in index.html or is created dynamically |
| Maint 26: CLI vs dashboard reliability | `journal/render.rs` fold routes through `ExitClass`; CHANGELOG notes the corrected numbers |
| Maint 19 / Contrarian 11: traceability | Appendix finding-ID → phase → test matrix; P0/P1 IDs embedded in test names |
| Maint 28: undefined "Stage 5" | Verification section names concrete commands and the EGM spot re-audit |

## Non-Functional Requirements

- Security and privacy: loopback-only bind; Host/Origin/Content-Type validation; CSP without `unsafe-inline`; no cross-origin reads; store path containment on every read; memory bodies remain data (provenance note in `agent-integration.md`).
- Performance and latency: `/api/summary|memories` ≤ 10 ms; `/api/reuse` ≤ 50 ms cached / ≤ 2.5 s cold; refresh bounded (≤3 s debounce, ≥10 s max wait); payload caps enforced.
- Rollout and rollback: single binary; `scripts/install.sh` to update; memory schema v2 is additive (old DBs readable; readers COALESCE); cache kill switch `ENGRAM_DASHBOARD_NO_CACHE=1`; payload cuts documented with `schema_version` so consumers detect them.

## Detailed Implementation Plan

Phases are sequential; each phase ends with `cargo test --locked` green and lists its own exit criteria.

### Phase 1 — Truth P0 (payload core)

1. `src/dashboard/data/summary.rs`: ToolAgg gains `ok_calls/ok_shown/ok_results` + `results`; `format_tool_aggs` emits `calls/errors/shown/p50/p95/saved_tokens` (success-only); recall tracked by tagged sets only; `tagged_session_count`; payload: drop `warning_count/reuse_hits/recent_queries/recent_errors/graph_recent_errors`, add `schema_version:2`, `generated_at`, `window:"retained_journal"`, `estimated:true` on `saved_tokens/saved_dollars/graph_saved_tokens/graph_saved_dollars`.
2. `src/query/reuse.rs` + `src/query/reuse/detectors.rs`: `overlap_pct: Option<u64>`; D4 → `None` + rank helper `sort_rank()` returning 100; D3 → `None` (evidence tag); judge band: `None` not gray; tests: D4 survives CLI `--limit 10`, D3/D4 hint lines carry no percentage.
3. `src/dashboard/data/reuse.rs`: emit `overlap_pct: null` + `overlap_applicable:false`; drop `detector_title/detector_summary`; `estimated:true` on `preventable_loc`, `avg_callers`, `detector_counts`; `schema_version:2`.
4. `src/dashboard/monitor/render.rs`: TOOL rows read `calls/errors/p50/saved_tokens`; version `env!("CARGO_PKG_VERSION")`; USP labels → plain banners.
5. `src/dashboard/assets/app.js`: `switchTab` allowlist guard + call-site fix; D4/D3 badge "WRAPPER/SHARED GUARD (N lines)"; remove replaced-field fallbacks; KPI reads new fields.
6. Tests in same phase: `tests/cli/dashboard.rs` payload updates; summary recall/savings tests; monitor tool-row test with real payload; CLI overlap-survival test.

### Phase 2 — Parity & classification single source

1. `src/dashboard/data/summary.rs`: `pub(crate) enum ToolClass` + `classify(tool)`; `is_graph_tool`/mem classes derived; include `mem_export` read, `mem_delete`/`mem_gc` write.
2. `src/dashboard/monitor/state.rs` + `render.rs`: consume `ToolClass`; R:W sums reads (`mem_query|mem_get|mem_export`) and writes (`mem_write|mem_touch|mem_delete|mem_gc`); recall "—" when no tagged session.
3. `src/journal/render.rs`: global fold reliability routes through `ExitClass` (empty/ambiguous not errors); CHANGELOG note.
4. `src/dashboard/assets/app.js`: delete `estimateEventTokens` + all aggregate patching; `GRAPH_TOOLS`/mem lists aligned; refresh scheduler 3 s debounce + 10 s max wait.
5. `src/dashboard.rs` tests: parity test parsing `GRAPH_TOOLS` + mem lists from `app.js` (include_str) and asserting equality with `ToolClass`; scheduler static check; no-coefficient-literal test.

### Phase 3 — Durable interaction

1. `schema/memory.sql` + `src/db/memory_store.rs` + `src/db/memory_write.rs` + `src/memory.rs`: add `anchored_at INTEGER NOT NULL DEFAULT 0`; `MEMORY_SCHEMA_VERSION = 2`; write-path open runs `ALTER TABLE` when needed; touch sets `anchored_at = now`, preserves `created_at`; export/import tolerate the column.
2. `src/dashboard/data/memories.rs`: `store_id` = canonical `memory.db` path; `anchored_at` used for ref staleness (COALESCE); `index_conn.is_none()` → `unverified` for every store; drop `session/content_hash/expires_at/superseded_by/superseded_at/prev_*`; recall_count exact-key only; delete `estimate_fallback_recall` + `kind_prevention_card`; LIKE `ESCAPE`; `schema_version` + `degraded/errors[]`.
3. `src/dashboard/server.rs`: Touch v2 — Content-Type 415, Origin/Host check, `{key, store_id, rev}`, exact store lookup (400 missing/404 unknown/409 rev/503 open), audit line, 4 KiB cap; `--local` resolves root at startup; named `event: reset` on shrink/rotation via shared tail helper; request-log line + startup provenance.
4. `src/dashboard/monitor/mod.rs`: shared tail helper; no replay on shrink.
5. `src/dashboard/assets/app.js`: FILTERS sole authority + validation/reset notice; selection keyed by `store_id:key:rev` / symbol+canonical; Touch "Verify" sends `{key, store_id, rev}`; inline error + bulk verify; `onopen` refetch; reset announcement; error states with Retry + age chip.
6. Tests: schema migration (v1 db gains column, notes preserved), touch store/rev/status matrix, SSE reset frame name, selection stability, corrupt store → degraded.

### Phase 4 — Security hardening

1. `src/dashboard/server.rs`: Host allowlist (403), remove wildcard CORS, CSP/nosniff/referrer/frame-ancestors headers, header/request caps, SSE cap 8, refuse non-loopback bind without explicit flag.
2. `src/dashboard/assets/app.js` + `index.html`: remove every inline `onclick/oninput`; `addEventListener` with data-attributes; `escapeHtml` adds `'`; escape all DB-derived interpolations; live region `textContent` only.
3. `src/dashboard/data.rs`: `read_snippet` containment (reject absolute/`..`, canonical prefix check) + I/O bound; journal per-line 256 KiB cap + skipped counters.
4. `src/dashboard/data/reuse.rs` + `memories.rs`: canonical containment on snippet/memory paths; `reject_symlink` on read opens.
5. Tests: hostile memory key (`');fetch(...)`) renders inert; cross-origin POST rejected; wildcard-CORS gone; traversal ref/snippet rejected; oversized header/line rejected; corrupt store non-200/degraded.

### Phase 5 — Accessibility

1. `index.html` + `app.js`: tabs pattern (roving tabindex, arrows/Home/End, `aria-selected`/`aria-controls`), row/card activation via real buttons or `role="row"`+tabindex+Enter/Space with accessible names, aria-live state-transition policy.
2. `style.css`: token fixes (`--intent-warning`), contrast (light primary pairs, agent-tag-user, dark Verify button), `:focus-visible`, reduced-motion guards, truncation classes, KPI grid `auto-fit`, page-scroll layout (no 820 px traps).
3. Tests: static ARIA/keyboard assertions in `src/dashboard.rs`; no `<tr role="button">`.

### Phase 6 — Copy, cuts, monitor remainder

1. `app.js` + `index.html`: TIP rewrites; empty-state split (filter-zero vs empty vs degraded); terminology (Repository/Store/Note scope, Reliability header, "rev", pluralization); "Verify" naming; agent tables kept but rendered by one parameterized `renderAgentTable(spec)`; Scope KPI tile + its writes removed; dead CSS/`liveEventCount`/unreachable fallbacks removed; recall badge omitted at 0.
2. `monitor/render.rs` + `state.rs`: stream clamp to 80 cols, fresh-state path line, filter-scope copy, "no refs"/"unverified" states, plain labels.
3. Docs: `agent-integration.md` payload schema (fields, null semantics, units, `schema_version`, SSE event names, untrusted-body note), `cli-reference.md`, `usage.md`, `CHANGELOG.md`.
4. Tests: static test that app.js contains no removed field names; snapshot of empty-state copy.

### Phase 7 — Performance

1. `src/dashboard/data/reuse.rs`: cache `(CacheKey, Payload)` — key computed before read (index + both log generations, mtime+len; missing-file sentinel), compute outside lock, swap under lock, never cache `Err`/degraded, `cache:{hit,age_ms}` in payload, `ENGRAM_DASHBOARD_NO_CACHE=1`.
2. `src/dashboard/data.rs`: `read_usage_entries` mtime+len cache (incl. `usage.log.1`), snippet window read.
3. `app.js`: lazy `/api/reuse` (tab active or build event only); remove `DOMContentLoaded` eager call.
4. Tests: cache hit/miss/age, mtime bump recompute, no-cache env, snippet window, lazy-fetch static check.

### File Ledger

#### `schema/memory.sql` (Modify) — add `anchored_at`; version 2 comment.
#### `src/db/memory_store.rs`, `src/db/memory_write.rs`, `src/memory.rs` (Modify) — schema v2 migration on write open; touch sets `anchored_at`; exports include it.
#### `src/query/reuse.rs`, `src/query/reuse/detectors.rs` (Modify) — `Option<u64>` overlap, D3/D4 `None`, rank helper, judge/sort/hint adjustments.
#### `src/journal/render.rs` (Modify) — reliability via `ExitClass`.
#### `src/dashboard/data/summary.rs` (Modify) — recall sets, ToolAgg success counters, ToolClass, payload shape/markers, per-tool metrics.
#### `src/dashboard/data/reuse.rs` (Modify) — null overlap, drop metadata, estimate flags, cache, containment.
#### `src/dashboard/data/memories.rs` (Modify) — store_id, anchored_at, unverified, field cuts, LIKE escape, degraded.
#### `src/dashboard/data.rs` (Modify) — log cache, snippet containment + window, per-line cap, skipped counters.
#### `src/dashboard/server.rs` (Modify) — touch v2, Host/Origin/CSP/CORS, SSE reset + tail helper + cap, request log, startup provenance, status map.
#### `src/dashboard/monitor/mod.rs` (Modify) — shared tail helper.
#### `src/dashboard/monitor/render.rs`, `state.rs` (Modify) — canonical keys, version, labels, R:W, clamp, fresh state, recall dash.
#### `src/dashboard/assets/app.js` (Modify) — all client changes above.
#### `src/dashboard/assets/index.html` (Modify) — ARIA, cuts, copy.
#### `src/dashboard/assets/style.css` (Modify) — tokens, contrast, focus, motion, truncation, grid/scroll.
#### `src/dashboard.rs` (Modify) — parity/static/ARIA/key-set/id-existence tests.
#### `docs/agent-integration.md`, `docs/cli-reference.md`, `docs/usage.md`, `CHANGELOG.md` (Modify) — schema, behavior, changelog.
#### `docs/designs/2026-11-11_concepts-brief_dashboard-truthfulness.md`, `docs/designs/2026-11-11_dashboard-truthfulness.md` (Create) — workflow artifacts.
#### Deleted files — None.

### Verification Plan

- Per phase: `cargo test --locked` (targeted first, full at phase end), `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`.
- Final gates: the two commands above, `cargo test --locked`, `bash tests/scripts/write_path_check.sh`, `bash tests/scripts/hygiene_check.sh`, `bash tests/scripts/allowlist_check.sh Cargo.lock`.
- End-to-end: rebuild release, `bash scripts/install.sh`, restart `eg dashboard`; verify CUJ 1–5 manually plus: agent-row click never blanks; monitor TOOL numbers real; recall tagged-only; D4/D3 badges label-based; filters survive SSE; Verify updates `anchored_at` only; cross-origin POST rejected; hostile key inert; `/api/summary` carries `schema_version/generated_at/window/estimated` and no dropped fields.
- EGM spot re-audit once (data-integrity, feature-correctness, tui-parity, rendering-fidelity) against the fixed surfaces; record the before/after in the final report.

## Appendix — Finding-ID traceability

P0/P1 findings and their pinning tests (EGM IDs from `.tmp/ux-review/findings/`):

| Finding IDs | Phase | Pinning test |
|---|---|---|
| tool-frequency-01, data-viz-01, tui-parity-01, contrarian-01 | 1 | `monitor_tool_row_reads_canonical_keys` |
| feature-correctness-01, consistency-01, component-value-01, a11y-02 | 1 | `switch_tab_rejects_unknown_key` (static) |
| data-integrity-01, agent-consumer-01, copy-wording-01, first-run-01 | 1 | `recall_is_tagged_only` |
| agent-consumer-05, contrarian-15, maintenance-02 | 1 | `d4_d3_overlap_is_absent_not_fabricated` |
| data-integrity-02/03/04, contrarian-02, feature-correctness-02 | 2 | `classification_parity_with_client`, `no_token_mirror_in_client` |
| edge-states-01, performance-05, maintenance-08 | 3 | `sse_emits_named_reset_on_identity_change` |
| edge-states-02/03, first-run-07, ergo-01/02/03 | 3 | `touch_resolves_store_and_rev`, `unverified_refs_*` |
| rendering-fidelity-01, feature-correctness-03 | 3 | `filters_survive_live_refresh` |
| a11y-01/03/04/05/06/07/08 | 5 | static ARIA/contrast checks |
| first-run-02/03, data-integrity-05/06, copy-wording-02 | 6 | `recall_badge_omitted_at_zero`, `hits_vs_candidates_copy` |
| performance-01, edge-states-08, obs-03 | 7 | `reuse_cache_serves_within_budget` |
| security High set | 4 | `hostile_key_inert`, `cross_origin_post_rejected`, `cors_removed`, `traversal_rejected` |

P2/P3 findings are grouped in the concepts brief's 14 problem areas; every area maps to a phase above.
