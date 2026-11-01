# R5b — P5 Implementation Review Record

- Reviewer: independent stateless Goldfish subagent (fresh context;
  deliverable = uncommitted working tree vs `b703e47` + the approved spec
  rev 3 + acceptance criteria — no conversation history, no author
  rationale). All evidence re-run by the reviewer (71 tool calls).
- Scope: `kg path` implementation (src/query.rs, cli/main/mcp/error wiring),
  fixtures, GT layer, docs.
- Round 1 verdict: **NEEDS FIXES** — 1 Important, 3 Suggestion, 5 Nit.

## Findings and dispositions (fix cycle 1)

| # | Sev | Finding | Disposition |
|---|-----|---------|-------------|
| 1 | Important | G17 fixture 2 was not a diamond: `dtop` called only `dm1`, so no BFS tie existed and no parent rule was falsifiable; the test comment described behavior the fixture never exercised | Fixture rewritten as a true diamond (`dtop -> {dm1, dm2} -> dbot`, both 2-hop routes); expected bytes unchanged (the pinned rule still selects dm1) — the fixture now fails under any other parent rule |
| 2 | Suggestion | Skill behavior entry covered 3 of 4 pinned exit-1 shapes; recovery guidance attached only to exit 2 | Same-symbol shape + not-in-index recovery (`kg search`/`kg symbols <file>`) written into the `kg path` behavior entry |
| 3 | Suggestion | `path()` ~213 lines (2x repo norm); `level_edges` closure indented off-grid | Split into `path_bfs()` (BFS + reconstruction, `PathSearch` enum for the two negative shapes) + render tail in `path()`; closure re-indented to house style |
| 4 | Suggestion | `docs/EVALUATION.md` gate table lacks G17 | Accepted as phase-close work per the P4 convention (separate `docs(publish)` sync commit) |
| 5 | Nit | "0.63-0.86 ms" misattributed the 26-file self-index figure to the 85k-symbol scale | Reworded to the two-scale table (0.6-1.4 ms across synth corpus + self-index; reviewer re-run under load included) in CHANGELOG + canonical spec |
| 6 | Nit | Stale skill heading "the two contract behaviors" | Reworded |
| 7 | Nit | Chain render silently rendered an empty segment on a map miss while neighboring guards returned `Internal` | Aligned: map miss now returns `KgError::Internal` |
| 8 | Nit | `dist` HashMap values write-only | Replaced with a `HashSet` visited set |
| 9 | Nit | Three pinned behaviors untested: ambiguous shared name (exit 2 precedence), min-clamp boundary, MCP exit-2 role bytes | First two pinned as fixture assertions (`path dup dup` -> exit 2; `--depth 0` -> clamped cap-truncated bytes); the third is single-sourced in `KgError::Display` (theoretical gap, recorded, not pinned) |

## Round 2 (post-fix verification, this session)

- `cargo test --locked --release`: 98 passed, 0 failed (fixtures 10/10
  scenarios + the two new pins; diamond now falsifiable).
- `cargo clippy --locked --all-targets -- -D warnings`: clean.
- GT runner with the final binary: 8/8 agreement.
- Spawn-inclusive `timing.sh` on the synth corpus: real=0.00s (7/7).
- Verdict: **APPROVE** (fix bound: 1 of <= 2 cycles used).
