# Multi-repo verification — kg 0.1.0 @ post-Amendment-11 `56f448a` (2026-11-01)

Scope: verify the perf-ceiling-pass binary (Amendment 11: quick_check-free
query path, single-txn inserts, parallel parse, schema v3) on six real
Workspace codebases + kg self, with pointer-level authenticity checks.
Companion to `multi-repo-verification-2026-11-01.md` (pre-A11 binary).

## 1. Fresh force builds (`/usr/bin/time -l kg build --force`)

| repo | files (git-visible) | symbols | edges | parse_errors | wall | peak RSS |
|------|--------------------|---------|-------|--------------|------|----------|
| vihap | 252 | 3291 | 9128 | 0 | 0.27s | 29.6MB |
| finguard | 135 | 488 | 1848 | 0 | 0.18s | 16.3MB |
| be-pool-collector | 52 (of 3875 raw .py; rest untracked) | 385 | 717 | 0 | 0.08s | 10.3MB |
| ds-team-dag | 596 | 3119 | 3849 | 0 | 0.46s | 40.0MB |
| be-map-search-v2 | 150 | 1381 | 3486 | 0 | 0.16s | 26.8MB |
| be-ds-forecast | 27 | 103 | 166 | 0 | 0.06s | 8.9MB |
| kg self | 24 | 306 | 904 | 0 | 0.13s | - |

Extraction parity: vihap totals (252/3291/9128) byte-match the pre-A11
baseline record; corpus totals match to the seed. Second `kg build` carries
100% on every repo (fast path + v3 carry-forward).

## 2. Query battery (best-of-5/7, incl. process spawn)

All commands 5.0-10.5 ms across all repos — flat at spawn floor (~6 ms) +
query cost; no quick_check term, so latency does not grow with index size.
Heaviest observed: `impact` on be-ds-forecast 10.5 ms. (Pre-A11 battery:
6.9-13.7 ms on vihap-class repos.)

## 3. Pointer spot-verification (authenticity)

Rule set mirrors G2 precision: `callers` pointers must hit a source line
containing the callee bare name; `impact`/`dead`/`symbols` pointers must be
call sites / definitions / in-bounds respectively.

| repo | pointers checked | pass |
|------|-----------------|------|
| vihap | 210 | 210 |
| finguard | 138 (10 initial checker-regex false fails on TS `export function` defs; re-checked 10/10) | 138 |
| be-pool-collector | 130 | 130 |
| ds-team-dag | 85 | 85 |
| be-map-search-v2 | 181 | 181 |
| be-ds-forecast | 33 | 33 |
| **total** | **777** | **777 (100%)** |

Zero staleness headers on fresh indexes (correct silence); after a live edit
the header fires (`kg: 1 of 1 result files changed since build — run kg build`).

## 4. G2 precision/recall (frozen ground truth, post-A11 binary)

| repo | recall | EXTRACTED-tier precision | verdict |
|------|--------|--------------------------|---------|
| vihap | 29/29 = 100% | 100% | PASS |
| finguard | 20/20 = 100% | 100% | PASS |
| kg self | 10/10 = 100% | 100% | PASS |

Commands: `python3 tests/scripts/precision_recall.py <repo> target/release/kg
docs/baselines/ground-truth{,-finguard,-kg-self}.md`.

## 5. Contract checks on a real tree (be-ds-forecast copy in /tmp)

- Dirty closure, callee-side edit: `# drift` appended to `app/api/deps.py`
  -> rebuild re-extracted the closure (carried=23/27), `kg callers
  get_model_repository` still returns `tests/test_deps.py:8`; table totals
  invariant (27/103/166/254).
- Dirty closure, caller-side edit: same edge survives after editing
  `tests/test_deps.py` (carried=26).
- CUJ4: `kg callers __init__` exits 2 with the `path:line qualname` candidate
  list (ambiguous bare name correctly never merges silently).
- MCP-vs-CLI parity (G10 smoke, vihap, `callers normalize_admin_name`):
  MCP tool text byte-identical to CLI stdout (1793 bytes), isError=false.

## 6. Notes

- RSS is up ~5-7 MB vs the pre-A11 record on the same repos (parallel worker
  stacks/trees); peak remains <= 40 MB on the largest repo (ds-team-dag).
- be-pool-collector: kg indexed only the 52 git-visible files; untracked
  data/venv trees are excluded by design (git-visible-set walk).
