# Concepts Brief: kg — Local Code Knowledge Index (Rust)

- Date: 2026-10-31
- Status: Concept SURVIVES review — 6 revisions required before spec
- Review method: six-lens Goldfish brainstorm. Isolation via stateless subagents (fresh context each; simulated Tier-2 isolation, stated per dispatch contract). Hypothesis: `.tmp/kg-spec/hypothesis.md`.
- Lenses run: Contrarian/YAGNI, Cost/Quota/Token, UX/Ergonomics, Migration/Compat, Observability/SRE, Maintenance/Coupling.

## 3 Strongest Objections

1. **Sequencing is inverted (Critical — Contrarian)**: the only gate proving the tool beats grep (utility A/B) was scheduled after all Rust work. If it fails, the project dies after full investment. Resolution accepted: new **Phase 0** measures the grep baseline, freezes ground truth (30 call sites, top-10 pain list), and builds the A/B harness **before any Rust code**.
2. **Scope was scaffolding-heavy (Important — Contrarian)**: 7 of 10 subcommands are covered by existing local tools (universal-ctags, ast-grep) at zero maintenance. Unique value is only `callers`/`callees`/`impact`. Resolution accepted: P1 cut to `build` + 3 graph queries + `doctor`; `symbols`/`search` deferred until ctags demonstrably fails; `report`/`hotspots`/`stats` moved to P1.5 with their own gates; `references` edges dropped from P1.
3. **Migration hazard was structural (Critical — Migration)**: no schema version, no grammar version, ambiguous build semantics — version skew could silently mix graphs extracted under different grammar versions. Resolution accepted: `PRAGMA user_version` + `grammar_version` in meta; **`kg build` is always a full atomic replacement** (build to side file, checkpoint+close, rename; resolves mixed-graph, rollback, and partial-build hazards in one rule); `flock` build lock; read window N/N-1; never migrate in place.

## Findings and Dispositions

| Severity | Count | Disposition summary |
|----------|-------|---------------------|
| Critical | 6 | All accepted; 3 structural (sequencing, scope, atomic-replacement policy), 3 spec gaps (schema_version prerequisite, silent tree-sitter parse failure observability, call-resolution algorithm must be specified — it is the core IP) |
| High | 14 | Accepted: recall gate + grep-fallback on empty EXTRACTED; concurrency gate + busy_timeout + flock; per-file parse-size cap; build manifest (kg/grammar version, source rev) stamped in DB; crash-atomic swap; cap notice duplicated as final stdout line with total ("showing 20 of 143"); per-result tier tag + `--tier` filter; exit-code taxonomy; index discovery upward to git root; ambiguous-name exit-with-candidates; `--raw` replaced by `--limit`; A/B correctness sampling; runtime rollback (`build --force`, `doctor --invalidate`); zero-network test mechanism per-OS (Seatbelt/seccomp) + dep allowlist |
| Medium | 15 | Accepted: cap N default specified; WAL checkpoint+close before rename; growth model MB/kLOC measured in P0; unit-cost metric (result bytes + CPU-s); scaling measurement on synthetic 1M LOC corpus once; rename `health`->`hotspots`; FTS5/plain error mapping (deferred with `search`); stale-check scoped to result-set files only; golden stdout/stderr snapshots = versioned response contract; grammar crates + runtime upgrade together with fixture replay; streamed chunked inserts (no full-corpus Vec); FNV semantics frozen (raw bytes, mtime+size fast path); `.kg/` excluded from walk unconditionally; A/B transcripts committed; additive-only schema changes |
| Low/Nit | 8 | Accepted: forbid(unsafe) wording corrected ("no unsafe Rust in this crate; C parser runtime is an accepted pinned dependency"); INFERRED tier dropped until a producer exists (P1 ships EXTRACTED + AMBIGUOUS); rayon dropped from P1 deps (measure single-threaded first); error taxonomy enum; safe-wording for safety claims; stats invocation counter; docs/tests notes |

Rejected / not adopted:
- Transparent rebuild inside query process (Contrarian #2 alternative): rejected — violates consultative principle (no hidden writes on query) and complicates invariant 3. Mitigated instead by fast builds (<5s) + explicit "run kg build" remedy line.
- Presented-as-measured gate labels: accepted correction — all gate numbers are **targets** until P0/P1 measurement runs exist; codegraph "+80% residual context" relabeled vendor-reported, unverified.

## Concept Status After Revisions

P1 scope: `build`, `callers`, `callees`, `impact`, `doctor` — single-threaded Rust, tree-sitter-python, 5-table SQLite schema + FTS5, EXTRACTED/AMBIGUOUS tiers, full-atomic-replacement builds, versioned response contract. Phase 0 (no Rust) must complete first and produces the measured baseline that G6 compares against. Full detail: spec `docs/designs/2026-10-31_kg-rust-code-index.md`.
