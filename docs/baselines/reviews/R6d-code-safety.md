# R6d — Code safety and correctness (Goldfish, 2026-11-02)

- Verdict: **NEEDS FIXES** — 2 Blocker, 12 Important, 12 Suggestion, 5 Nit;
  22 code-vs-spec divergences.
- Verified green: `cargo test --locked` 98 pass; clippy clean; both invariant
  scripts pass; corrupt/truncated indexes exit 3; SIGKILL build recovery OK;
  600 malformed sources build without panic.

| # | Sev | Finding | Evidence |
|---|-----|---------|----------|
| 1 | Blocker | Deeply nested source aborts the build (SIGABRT, exit 134) — ~2,000 nesting in a 4 KB file; recursive tree walks on 2 MB worker stacks | `src/parse.rs:68-78,116-126`; `src/extract.rs:332-383`; `src/build.rs:108-147` |
| 2 | Blocker | `kg clones` materializes every pair pre-cap: 700 files with a shared 5-line header → 3,180,438 pairs, 845 MB RSS, 1.66 s | `src/query.rs:727-798` |
| 3 | Important | `callees --depth N` returns N+1 levels (CTE stops at `< depth`, SELECT emits one more) | `src/query.rs:189-204` |
| 4 | Important | Incremental build keeps stale unresolved calls and misses new symbols (dirty closure ignores unresolved/plain-import edges) | `src/db.rs:230-261`; `src/build.rs:71-88` |
| 5 | Important | Rust `impl` method qualnames attach to the wrong type (first `type_identifier` in the body) | `src/extract.rs:1099-1112` |
| 6 | Important | Constructor calls redirect to `__init__`: `callers Class` → 0, `dead` lists instantiated class | `src/extract.rs:1464-1480` |
| 7 | Important | MCP stdin unbounded: 32 MB frame → 72 MB RSS | `src/mcp.rs:72-76` |
| 8 | Important | Build rewrites `.git/index` outside `.kg/` (invariant 3 violation) | `src/db.rs:543-549` |
| 9 | Important | `kg mcp --no-journal` still journals (flag not propagated) | `src/main.rs:31-35`; `src/mcp.rs:185-188` |
| 10 | Important | `GRAMMAR_VERSION` hand-maintained and wrong (`rs-0.24.2` vs lock `0.23.3`) | `src/db.rs:25` vs `Cargo.lock` |
| 11 | Important | Broken pipe panics → exit 6 with false "no output written" | `src/main.rs:37-65` |
| 12 | Important | Tracked-but-gitignored files silently skipped | `src/walk.rs:64-110` |
| 13 | Important | PEP 420 namespace packages unresolved | `src/extract.rs:1388-1406,1806-1833` |
| 14 | Important | Linux G5 network-deny job cannot pass as written | `.github/workflows/ci.yml:56` |
| 15-31 | Suggestion/Nit | Search tie order non-deterministic fresh vs carried; corrupt OLD index exits 0 (spec says 3); zero-length index exits 5; empty `dead`/`clones` omit caveat; duplicate pointers in callers/impact; `doctor --invalidate` deletes journals without lock; carried `inherits` unfiltered; unresolved undercounts; MRO 6 / re-export 4 hard caps; allowlist is a denylist (69 crates); search loads all matches; `.mjs/.mts` module path bugs; MCP protocol gaps (no negotiation, no panic isolation, no watchdog, missing newline); build memory 307 MB at 12k files; journal quote-unescape; contract drift | as cited |
