# Public readiness — kg @ 2026-11-02 (`6691b34`)

**Answer: NO-GO.** kg is not ready for a public repo/release; and for generic
daily use the alternatives are stronger. It remains a competent private tool
with a fast query engine and a narrow unique bundle (`path`, file `impact`,
`usage --verify`, `unresolved`).

## Why NO-GO (ordered)

1. Legal: proprietary LICENSE; employer-authored commit identity; ownership
   of corpus-derived content unconfirmed.
2. Privacy: 581 baselines files with private names/source/paths; 68 lines in
   the 51-file candidate export; exact corpus fingerprints re-identify.
3. Code blockers: build SIGABRT on deep nesting; `clones` O(k²) OOM;
   4,000-file incremental rebuild 42 s; `callees --depth` N+1; Rust impl
   qualnames wrong; incremental staleness; MCP unbounded input.
4. CI: both network-deny jobs are broken/vacuous; allowlist is an open
   denylist; G2 stale; no release/install/fmt jobs.
5. Claims: unresolved-rate inverted (10-48% vs actual 51.8-90.4); G14 0.850
   → 0.800 on the shipped index; perf table stale for schema v4; several
   denominators/records wrong; README omits negative utility results.

## Daily-use answer

Primary stack: host LSP plugin + Serena (symbols/refs/rename) + ast-grep +
ripgrep; specialists for audits (Vulture/Knip, jscpd, lizard/radon);
Sourcegraph only if cross-repo/enterprise is worth $16K/y. Keep kg only for
`path`/file-impact on repos ≤ ~3,000 files with the kg-first skill installed
and outputs verified.

## GO conditions (all required)

- Ownership/right-to-publish confirmed in writing; dual MIT/Apache-2.0 +
  NOTICE; public git identity.
- 51-file sanitized export, zero-assert gate green (R6b manifest).
- Code blockers A closed with regression tests; perf envelope published and
  enforced.
- CI: real seatbelt/bwrap with negative controls, exact allowlist, fmt/MSRV/
  install/audit/release jobs, fresh-GT self gate.
- Claims corrected and re-measured on the shipped binary; README carries the
  negative results and host dependence.
- Tagged release with install smoke on macOS/Linux.

Full evidence: `docs/baselines/reviews/R6a..R6g` + `R6-synthesis.md`.
