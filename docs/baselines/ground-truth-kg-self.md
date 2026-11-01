# Ground Truth — kg self-index (P2.5), 10 call sites (FILLED 2026-10-31)

- Filled: 2026-10-31, agent verification; kg indexes its own repository (dogfooding per spec P2.5).
- Rust module convention: file src/query.rs -> module src.query; use-decl crate::db::X resolves to src.db.X.
- M3 re-verification (2026-11-01): rows G02-G10 refreshed again — the M3
  MCP refactor (build.rs extraction, journal record blocks) shifted main.rs
  and query.rs; call sites re-confirmed against the freshly built index
  pointer lists. G01/G08 unchanged.
- A10 re-verification (2026-11-01): rows G01/G02/G07/G09/G10 refreshed —
  Amendment 10 (payload journal capture in query.rs collect_output/symbols,
  verify fn in journal.rs, Usage {verify,limit} in main.rs, payload-health
  lines in doctor.rs) shifted the call sites; all re-confirmed against the
  fresh index pointer lists (recall 10/10 after refresh; the 5 pre-refresh
  misses were line shifts, zero resolution regressions). G06 line 169 was
  already correct but its annotation mislabeled the Dead dispatch as Stats
  (M3-era error, now corrected).

| # | caller file:line | callee qualname | invocation kind | verified |
|---|------------------|-----------------|-----------------|----------|
| G01 | src/doctor.rs:85 | src.query.file_changed | import-call via crate:: path (doctor.rs:8 `use crate::query` module path call, stale_ratio) | [x] |
| G02 | src/query.rs:381 | src.query.file_changed | same-file local call (collect_output staleness loop) | [x] |
| G03 | src/main.rs:127 | src.db.open_for_query | import-call (use crate::db; Callers dispatch) | [x] |
| G04 | src/main.rs:141 | src.db.open_for_query | import-call (Callees dispatch) | [x] |
| G05 | src/main.rs:155 | src.db.open_for_query | import-call (Impact dispatch) | [x] |
| G06 | src/main.rs:169 | src.db.open_for_query | import-call (Dead dispatch) | [x] |
| G07 | src/main.rs:220 | src.db.open_for_query | import-call (Hotspots dispatch) | [x] |
| G08 | src/query.rs:80 | src.query.clamp_limit | same-file local call (callers entry) | [x] |
| G09 | src/query.rs:131 | src.query.clamp_limit | same-file local call (callees entry) | [x] |
| G10 | src/query.rs:204 | src.query.clamp_limit | same-file local call (impact entry) | [x] |
