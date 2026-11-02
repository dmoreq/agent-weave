# R6 Synthesis — public-readiness decision (7 Goldfish lenses, 2026-11-02)

Inputs: R6a-R6g records (this directory). All reviews stateless, read-only,
each with its own acceptance criteria; findings carry file:line/command
evidence. This synthesis is main-thread work, not a reviewer verdict.

## Decision 1 — Public repo/release: **NO-GO** (at HEAD `6691b34`)

Blocking, in dependency order:

1. **Legal/ownership** (R6a): all-rights-reserved LICENSE; commits authored
   with the employer email; `@finguard/*` references. Right to relicense must
   be confirmed before any public push.
2. **Privacy** (R6b): `docs/baselines/**` (581 files) cannot ship; 68 privacy
   lines in the 12 included non-baselines files; a fresh repo must use a
   public git identity; A-F labels alone re-identify (exact counts + domain).
3. **Correctness/safety code** (R6d, R6f): build aborts (SIGABRT) on deeply
   nested source; `clones` O(k²) OOM (2.87 GB / 9M pairs at 700 files);
   incremental rebuild O(N²) (42 s for one edit on a 4,000-file chain).
4. **Invariant enforcement** (R6e): macOS seatbelt job is vacuous; Linux
   bwrap job cannot pass; cargo allowlist is a broken denylist; G2 gate stale
   and non-gating. Invariant 1 has no working enforcement anywhere.
5. **Claim integrity** (R6c, R6f): "10-48% unresolved" inverted (actual
   51.8-90.4%); G14 0.850 → 0.800 on the shipped P6 index (invalidating the
   G18(i) premise of the keep-P6 decision); perf claims stale for schema v4
   (2.87 s / 186 MB vs 0.27 s / ≤40 MB at the committed 2,025-file corpus);
   G2 self GT 1/10; G13 -64.8% → -62.7%; 91/91 → 98/98; "five deps"/"static
   binary"/"MCP-connected" unsupported; README omits the negative utility
   results.
6. **Distribution identity** (R6a): crates.io `kg`/`kg-cli` taken; choose
   binaries-only + `publish=false`, or rename. Metadata/install/CHANGELOG/
   NOTICE/community files still private-framed.

GO conditions: all six closed, the sanitized 51-file export passes the R6b
zero-assert gate, CI green including a real network-deny (with negative
control) and a fresh-GT self gate, claims re-measured on the shipped binary,
and a tagged release with install smoke.

## Decision 2 — Daily use: alternatives first, kg as optional supplement

R6g independent runs: query latency <10 ms even on a 382 MB index (Django,
3,050 files; vscode 11,275 files) — the query engine is genuinely fast — but
build cost (24.6 s / 1.04 GB RSS on vscode) and unresolved share (62.9-71.7%)
define a narrow envelope. Alternatives win 5 of 6 workload classes (see R6g).

Keep kg only if all hold: copyright holder; repo ≤ ~3,000 files / ≤50 MB
index; questions are `path`/file-impact; kg-first skill installed; outputs
verified with grep/LSP. Otherwise: host LSP plugin + Serena + ast-grep + rg;
specialists for audits (Vulture/Knip, jscpd, lizard/radon).

## Phase-2 fix program (owner re-scope required)

| bucket | items | effort |
|---|---|---|
| A. Code blockers | deep-nesting guard/iterative walks; clones pair cap/streaming; callees depth off-by-one; incremental dirty closure; Rust impl qualname; constructor attribution; MCP frame cap + `--no-journal`; `.git/index` write; GRAMMAR_VERSION derivation; broken pipe; gitignored tracked files; namespace packages; search tie-break | ~3-5 days + tests |
| B. Perf envelope | query materialization bounds; MCP/search input caps; incremental work-queue; no-op build cost; publish measured envelope; doctor warning | ~2-3 days |
| C. Evidence/claims | F1 inversion; F2 re-measure/re-rank (P6 keep decision re-evaluated); self GT refresh + CI gate; G13/denominator/test-count corrections; missing G11/A11 records; precision fraction; README negative results | ~1-2 days |
| D. Public export/legal | ownership confirmation; dual license + NOTICE; 51-file export + zero-assert gate; git identity; metadata; community files; install docs | ~1 day |
| E. CI | seatbelt apply + negative control; bwrap writable; exact allowlist; fmt/MSRV/install/audit/release jobs; GT gates wired | ~1-2 days |

Not in scope without a new decision: history rewrite (fresh repo chosen),
feature work, publishing/pushing (owner-executed).
