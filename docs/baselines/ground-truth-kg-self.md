# Ground Truth — kg self-index (P2.5), 10 call sites (FILLED 2026-10-31)

- Filled: 2026-10-31, agent verification; kg indexes its own repository (dogfooding per spec P2.5).
- Rust module convention: file src/query.rs -> module src.query; use-decl crate::db::X resolves to src.db.X.
- M2 re-verification (2026-11-01): rows G02-G10 refreshed against the current
  tree — M1's journal wiring shifted main.rs and query.rs; direct code read
  re-confirmed each call site at its new line. G01 unchanged (doctor.rs).

| # | caller file:line | callee qualname | invocation kind | verified |
|---|------------------|-----------------|-----------------|----------|
| G01 | src/doctor.rs:76 | src.query.file_changed | import-call via crate:: path (doctor.rs:8 `use crate::query` module path call) | [x] |
| G02 | src/query.rs:365 | src.query.file_changed | same-file local call (collect_output staleness loop) | [x] |
| G03 | src/main.rs:116 | src.db.open_for_query | import-call (use crate::db; Callers dispatch) | [x] |
| G04 | src/main.rs:130 | src.db.open_for_query | import-call (Callees dispatch) | [x] |
| G05 | src/main.rs:144 | src.db.open_for_query | import-call (Impact dispatch) | [x] |
| G06 | src/main.rs:158 | src.db.open_for_query | import-call (Stats dispatch) | [x] |
| G07 | src/main.rs:179 | src.db.open_for_query | import-call (Hotspots dispatch) | [x] |
| G08 | src/query.rs:80 | src.query.clamp_limit | same-file local call (callers entry) | [x] |
| G09 | src/query.rs:132 | src.query.clamp_limit | same-file local call (callees entry) | [x] |
| G10 | src/query.rs:208 | src.query.clamp_limit | same-file local call (impact entry) | [x] |
