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

## Round-2 remediation (2026-11-02, commit dcc8b5a + Stage 4)

The R6 round-2 review found 2 export blockers and several partials; all code
and export findings are now closed and verified:

| round-2 finding | fix | evidence |
|---|---|---|
| Export test suite red | scrub now also rewrites expected strings; export runs `cargo test` as a gate | export tests PASS (57 files) |
| Export leaked `marshal_io`/`default_artifacts_dir` | replacement list + gate vocabulary extended | export gate counts 0 |
| 19 dead `baselines/...` refs + mangled spec commands | doc-only path flattening; spec G18 command matches the script; G15 marked external | export gate PASS; spec updated |
| Write-path gate blind to `walk.rs` | python line-scanner: `walk.rs` allowed only after `#[cfg(test)]`; two negative controls | gate PASS with controls |
| `callees --depth>1` unbounded | COUNT + LIMIT on the closure query | 20k distinct callees at limit 50 bounded |
| Vacuous forward-import test | positive two-file resolution assertion | test `import_x_as_y_binds_module_root` |
| G12 parallel claim untested | table-hash identity test (fresh, fast-path, force) | `parallel_builds_produce_identical_tables` |
| Missing contract tests | `doctor --invalidate`, truncated index exit 3, MCP malformed JSON (-32700) | tests added |
| MSRV unverified | rust 1.90 installed; `cargo +1.90.0 build --locked` | PASS |
| Self-index numbers stale | re-measured at the final commit | 37/380/1171 |

Deferred with reasons (non-blocking): SHA-pinned GitHub Actions (supply-chain
hardening; tag pins currently), dependabot/SPDX headers, PEP 420 namespace
packages, empty-`dead`/`clones` caveat, MCP protocol negotiation, exit-6
panic-path contract test.

## Value evidence at scale (Stage 4, 2026-11-02)

`kg path` at 2,201 files / 4,401 symbols: 24/24 correct; Grok Build tool
calls −59% and kg calls −87%; OpenCode tool calls −61% and kg calls −77%.
The call bar clears on BOTH hosts at this scale (Stage 1 path also cleared
it: −57%/−86%; the effect is stable across scales, not monotonically growing). This is the largest measured
value result in the evidence set and the basis for the narrow positioning:
route/reachability questions on small-to-medium repos are where kg is
strongest; search/clones remain host-dependent.

## Round-3 closure (2026-11-02, commit 7537b40)

The round-3 verification's remaining findings are closed:

- Export scrub no longer substitutes prose for paths: no `the private evidence
  vault` tokens, no empty markdown links, Cargo exclude intact, exported CI
  guard syntactically valid, exported test suite PASS (57 files).
- Write-path gate is brace-aware: `#[cfg(test)]` modules are stripped before
  scanning, `File::options`/`File::create_new` are in the primitive set, and
  three planted controls (evil.rs, post-test walk.rs write, File::options) are
  caught.
- Self GT re-frozen at the final commit: 10/10 recall, 44/44 precision.
- Stage-4 wording corrected: the effect is stable across scales, not
  monotonically growing; Stage-1 numbers restated accurately (tool calls
  −57%/−86%, kg calls −91%/−71%).
- Full battery green: 109/109 tests, fmt, clippy, allowlist, write-path,
  self GT, path GT 8/8, search recall 0.800, seatbelt with negative control,
  install smoke, public export.

**Status: publish-ready. Remaining work is owner-only** (right-to-publish
confirmation, repository creation/push with a public identity, tag/release,
vulnerability-reporting toggle, handle confirmation).

## Redesign attempt (2026-11-02, owner-directed before publish)

Attempted the strongest available redesign for the weak feature (search):
schema v6 docstring indexing (one-line docs/comments in the FTS haystack).
Results: G20(i) G14 recall unchanged 0.800; G20(ii) prose-gap retrieval
5/6 improved; G20(iii) Stage 2c utility A/B FAILED — Grok tool calls +12%
(treatment 6.17 vs control 5.5), OpenCode −9% (5.33 vs 5.83); kg calls and
tokens up. Root cause stands: grep already matches docstrings as source text,
so symbol-level lexical search adds a step without replacing one.

Per gate G20's kill rule the feature was **reverted** (commit `28c346b`,
schema back to v5); the negative result and GT stay as evidence
(`stage2c-prose-2026-11-02.md`, `search-prose-gt.jsonl`).

**Final value position (evidence-based):**
- `kg path`: large, host-independent value at scale — 2,201-file corpus,
  24/24 correct, tool calls −59%/−61%, kg calls −87%/−77% (Grok/OpenCode);
  Stage 1 smaller repos −57%/−86% tool calls. This is the "large value" proof.
- `callers/callees/impact`: correctness advantage where grep is blind
  (98.6% vs 81.9% recall).
- `search`/`clones`: host-dependent; search redesign attempted and reverted
  for lack of economics; both remain quality-gated, honestly labeled.

The repository is publish-ready pending the owner-only actions listed above.

## Final feature completion (2026-11-02, commits a4db60d + 6fd19c5)

All in-flight features are now complete:

- **`kg context` (Amendment 17/P8)**: implementation, exact-byte fixtures,
  MCP parity, spec + G21, skill routing, Stage 5 A/B (23/24 correct; kg calls
  −84% Grok, tool calls −46% OpenCode, Grok −19%) — kept by owner re-decision,
  recorded as host-dependent.
- **Residual closures**: PEP 420 namespace resolution; empty-result caveat for
  `dead`/`clones`; MCP protocol-version negotiation; serial-vs-parallel build
  determinism test (`KG_WORKERS`); GitHub Actions SHA-pinned.
- Remaining recorded deferrals (not features): G15 skill-drift script lives in
  the companion-skills repo; exit-6 panic-path contract test (panic boundary
  covered by code review); dependabot/SPDX headers.
- Full battery green at the final commit: 112 tests, fmt, clippy, allowlist,
  write-path (negative controls), self GT 10/10, path GT 8/8, search 0.800,
  seatbelt with blocked probe, install smoke, cargo audit, MSRV 1.90, export
  57 files PASS.
