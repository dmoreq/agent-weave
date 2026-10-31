# Ground Truth — kg self-index (P2.5), 10 call sites (FILLED 2026-10-31)

- Filled: 2026-10-31, agent verification; kg indexes its own repository (dogfooding per spec P2.5).
- Rust module convention: file src/query.rs -> module src.query; use-decl crate::db::X resolves to src.db.X.

| # | caller file:line | callee qualname | invocation kind | verified |
|---|------------------|-----------------|-----------------|----------|
| G01 | src/doctor.rs:76 | src.query.file_changed | import-call via crate:: path (doctor.rs:8 `use crate::query` module path call) | [x] |
| G02 | src/query.rs:254 | src.query.file_changed | same-file local call (collect_output staleness loop) | [x] |
| G03 | src/main.rs:48 | src.db.open_for_query | import-call (use crate::db; Callers dispatch) | [x] |
| G04 | src/main.rs:55 | src.db.open_for_query | import-call (Callees dispatch) | [x] |
| G05 | src/main.rs:62 | src.db.open_for_query | import-call (Impact dispatch) | [x] |
| G06 | src/main.rs:69 | src.db.open_for_query | import-call (Stats dispatch) | [x] |
| G07 | src/main.rs:75 | src.db.open_for_query | import-call (Hotspots dispatch) | [x] |
| G08 | src/query.rs:74 | src.query.clamp_limit | same-file local call (callers entry) | [x] |
| G09 | src/query.rs:126 | src.query.clamp_limit | same-file local call (callees entry) | [x] |
| G10 | src/query.rs:202 | src.query.clamp_limit | same-file local call (impact entry) | [x] |
