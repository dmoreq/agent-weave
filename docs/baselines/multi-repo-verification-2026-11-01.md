# Multi-repo verification — kg 0.1.0 @ HEAD `dcfffa1` (2026-11-01)

Scope: usability + correctness verification of the shipped binary on the
three chosen repos (vihap Python ~102k LOC, finguard TS/TSX/JSX, kg self
Rust) — beyond the standing G2 gate: a 26-command query battery, an
independent 164-pointer spot-verification against source lines (GT-free),
and an MCP smoke with byte-parity. Full command outputs:
`/tmp/kg-verify/` (session-local; the tables below are the durable record).

## 1. Fresh force builds (G1 shape)

Command: `/usr/bin/time -l kg build --force`, 1 run per repo (timing harness fallback
per `tests/scripts/timing.sh`; hyperfine not installed).

| repo | files | symbols | edges | parse_errors | wall | peak RSS |
|------|-------|---------|-------|--------------|------|----------|
| vihap | 252 | 3291 | 9128 | 0 | 0.54s | 22.5MB |
| finguard | 135 | 488 | 1848 | 0 | 0.24s | 7.5MB |
| kg | 24 | 280 | 781 | 0 | 0.12s | 6.3MB |

## 2. G2 precision/recall (fresh indexes, all PASS)

Command: `python3 tests/scripts/precision_recall.py <repo> target/release/kg docs/baselines/ground-truth{,-finguard,-kg-self}.md`.

| repo | recall | precision |
|------|--------|-----------|
| vihap | 29/29 = 100% | 493/495 = 100% (rounded from 99.6%) |
| finguard | 20/20 = 100% | 57/57 = 100% |
| kg self | 10/10 = 100% | 31/31 = 100% |

## 3. Query battery (26 commands; per-command latency incl. process spawn)

| repo | commands | latency (except noted) | notes |
|------|----------|------------------------|-------|
| vihap | callers/callees/impact/dead/unresolved/stats/symbols/usage | 6.9-13.7ms | hotspots 415.9ms, report 372.3ms (by design: per-file re-parse + git log) |
| finguard | callers/callees/impact/dead/stats/hotspots | 6.1-7.1ms | hotspots 156.9ms (same design cost) |
| kg | callers/callees/impact/dead/unresolved/stats/usage | 5.6-7.0ms | |

All queries returned contract-shaped output; zero staleness headers (fresh
builds); exit codes as designed.

## 4. Independent spot-verification (GT-free, 164 pointers)

Pointers from live outputs were checked against the actual source line:

| check | pointers | pass |
|-------|----------|------|
| kg callers open_for_query — line contains `open_for_query(` | 22 | 22 |
| kg callers clamp_limit — line contains `clamp_limit(` | 6 | 6 |
| finguard callers formatMoney (qualname form) — `formatMoney(` | 3 | 3 |
| kg callees resolve_call --depth 2 — lines are definitions | 13 | 13 |
| vihap impact hamlet.py — lines are call sites | 50 | 50 |
| vihap impact street_parser.py — call sites | 50 | 50 |

Note: impact pointers include intra-file call sites (the target file appears
among its own impacting files when it calls its own symbols) — by design; an
agent consumer should expect the self-file in the list.
| vihap dead — definition lines | 10 | 10 |
| finguard dead — definition lines | 10 | 10 |
| **total** | **164** | **164 (100%)** |

## 5. Investigated anomalies (all correct behavior, verified by direct read)

1. `vihap callers validate_street` -> exit 1 `symbol 'validate_street' not in
   index` — no exact-name symbol exists (bad query guess by the tester,
   honest empty result by kg; lookup is exact-name, no fuzzy match — nearest
   real symbol is `_validate_street_candidate` at
   `vihap/parsers/street_parser.py:381`).
2. `vihap callers normalize_ascii` -> exit 2 with 2 candidates (path:line
   qualname, the disambiguation form):
   `vihap/cleaners/unicode.py:47 vihap.cleaners.unicode.normalize_ascii`
   (the GT-canonical def) AND
   `scripts/build_knowledge_base.py:64 scripts.build_knowledge_base.normalize_ascii`
   — two real same-name defs; CUJ4 ambiguity is the correct contract answer.
3. `finguard callers formatMoney` -> exit 2 with 2 candidates:
   `apps/prototype/src/lib/format.ts:3 apps.prototype.src.lib.format.formatMoney`
   and `prototype/i18n.jsx:261 prototype.i18n.formatMoney` — a real duplicate
   def in a .jsx file; correct ambiguity.

## 6. MCP smoke (vihap)

initialize 4.8ms -> tools/list 11 tools 0.1ms -> tools/call impact
51 lines **byte-IDENTICAL** to CLI stdout, 4.3ms -> stats 3.4ms.
Journal received surface=mcp entries (Amendment 6/8 wiring live).

## 7. Verdict

kg is correct and usable across all three language families on real repos:
contract-shaped outputs, sub-15ms queries (hotspots/report design cost
excepted), honest empty/ambiguous results with candidate lists, 100%
pointer spot-verification, and MCP surface parity. Recorded by agent
verification; spot-verification predicates are stated above and re-runnable.
