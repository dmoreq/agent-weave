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

## Post-remediation status (2026-11-02, after commits 7748bfa..HEAD)

All R6 code blockers and Important items are fixed or explicitly deferred;
every gate below was re-run on this tree:

| gate | result |
|---|---|
| cargo test (schema v5) | 105/105 |
| fmt / clippy | clean |
| allowlist (exact, 70 packages) / write-path | PASS |
| self GT (refreshed) | 10/10 recall, 44/44 precision |
| path GT | 8/8 (one pair re-frozen 5->6 after the build refactor) |
| search recall | 0.800 (23/28), bar 0.70 |
| seatbelt + negative control | PASS (network probe blocked) |
| install smoke / audit | PASS / 0 vulnerabilities |
| public export (57 files) | PASS — private names/symbols/paths/email/secrets = 0; gate script excluded; no dead paths |

Blocker-to-fix map: deep nesting -> MAX_AST_DEPTH + 64 MB stacks; clones O(k^2)
-> region/pair caps; incremental O(N^2) -> index-backed work-queue (4000-chain
edit 9.4 s -> 0.20 s); MCP frame memory -> chunked drain (207 MB -> 13.8 MB);
gap healing -> schema v5 `unresolved_calls` + qualname matching; generic impls
-> in-order type-field traversal; non-ASCII paths -> NUL-delimited git set;
license/privacy/CI/claims -> as listed in the R6 records.

Deferred (recorded, non-blocking): PEP 420 namespace-package resolution; empty
`dead`/`clones` caveat; MCP protocol negotiation; two known precision failures
(`tests/test_marshal_kb.py:76`, `vihap/validators/gso_loader.py:703`).

**Remaining owner actions before a public push**: (1) written confirmation of
the right to publish/relicense the code and evidence-derived docs; (2) create
the GitHub repository and run `git push` of the exported tree with a public
git identity; (3) tag `v0.1.0` and publish the release artifacts; (4) enable
GitHub private vulnerability reporting; (5) confirm the GitHub handle used in
metadata (`quy-doan/kg`).

**Daily-use recommendation (unchanged)**: host LSP + Serena + ast-grep + rg as
the primary stack; keep kg for `path`/file-impact on repos <= ~3k files.
