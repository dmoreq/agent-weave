# R6f — Performance and scale (Goldfish, 2026-11-02)

- Verdict: **NEEDS FIXES** — 4 Blocker, 6 Important, 4 Suggestion.
- All numbers measured on macOS arm64 at HEAD `6691b34`; corpora under /tmp.

| # | Sev | Finding | Evidence |
|---|-----|---------|----------|
| 1 | Blocker | Deep nesting aborts (exit 134) | 2,000 parens; see R6d#1 |
| 2 | Blocker | `clones` O(k²): 700 files shared header → **2.87 GB RSS, 9,052,014 pairs** | `src/query.rs:746-803` |
| 3 | Blocker | Public perf claims stale for v4: committed 2,025-file corpus → **2.87 s / 186 MB / 58 MB index** vs claimed 0.27 s / ≤40 MB / 4.6 MB; A11 binary also fails (1.00 s / 130 MB / 26.9 MB) | `README.md:25`; `EVALUATION.md:14-15,50-60` |
| 4 | Blocker | Incremental rebuild O(N²): 4,000-file chain, one tail edit → **42.5 s** (full build 0.13 s); 250/500/1000/2000 → 0.09/0.28/1.28/6.88 s | `src/db.rs:230-261`; `src/build.rs:77` |
| 5 | Important | Query materialization unbounded: 1.05M callers → 550 ms / 109 MB; `dead` at 1M symbols 0.65 s / 99 MB; `search a` 1.02 s / 162 MB | `src/query.rs:845-854` |
| 6 | Important | MCP unbounded frame (200 MB → 205 MB) | `src/mcp.rs:71-77` |
| 7 | Important | Search quadratic in tokens: 10k tokens 8.06 s; 30k 71.1 s; 1M >180 s (killed) | `src/query.rs:662-666` |
| 8 | Important | No operating envelope documented; `doctor` target 50 MB exceeded by own small-corpus builds | `src/doctor.rs:62`; README |
| 9 | Important | `unresolved` says "full coverage" while external-import calls are dropped | `src/query.rs:543-545`; probe |
| 10-14 | Suggestion/Nit | No-op builds rewrite whole DB (1M symbols 13.07 s / 948 MB); ambiguous candidates uncapped (200k → 7.8 MB stdout); `stats` full scans; whole-buffer git calls; self-index row stale | as cited |

Independently supported at scale (positive): query latency < 10 ms on a 382 MB
index (Django 3,050 files; vscode 11,275 files, 722k edges); atomic-abort
leaves the old index usable; zero network/no daemon confirmed. Measured cost
at those scales: Django build 3.72 s / 223 MB / 58 MB DB; vscode build 24.6 s /
1.04 GB / 382 MB DB — both far above the spec's 50 MB DB / 100 MB RSS targets.
