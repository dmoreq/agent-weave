# Prevent-Reinvention Phases 2–5 — Implementation Contract

- Status: Implemented (owner directive 2026-11-13)
- Scope: proposal + numeric confidence, allowlist, coverage, judge
- Predecessor: `2026-11-12_reuse-prevention.md` (Phase 1 approved)

## Phase 2 — Proposal + numeric confidence

- CLI/MCP `--propose "<signature>"` with optional `--callees a,b`.
- Proposal uses indexed production symbols only; it never writes a source file.
- Paths combined with `--propose`, or `--callees` without `--propose`, exit `2`.
- `low|provisional|medium|high` are public tiers; `--min-confidence` filters.
- Proposal lines journal `mode=propose` and retain candidate IDs for uptake.
- Oracle ledger arithmetic remains 160 labels: 50 tune, 60 confirmation
  positives (30 eligible + 15 D4 + 15 D3), 30 confirmation negatives, 20
  holdout. `tests/scripts/reuse_oracle.py` validates labels and can compute
  precision, recall, TP/FP/FN/TN, and Wilson precision upper bound.

## Phase 3 — Allowlist

- Local memory key: `reuse-allow/<target>-><canonical>`.
- High-confidence pairs are never suppressed.
- Notes with refs are ignored when any referenced file changed since build.
- `--show-suppressed` keeps allowlisted candidates visible and reports
  `N suppressed` in the summary and journal.
- MCP exposes the same `show_suppressed` parameter.

## Phase 4 — Coverage

- `--coverage class,unresolved,tests`, off by default.
- D5: same-name class with method overlap across files.
- D1L: unresolved bare-name call matching an indexed production symbol.
- DT: test helper name-token overlap with a production symbol.
- Coverage results are capped at Medium confidence.

## Phase 5 — Judge

- `--judge` and MCP `judge` are restored as opt-in.
- Child runs only for gray-zone candidates (45–75% overlap), at most five.
- Deadline 15 s, stdout cap 64 KiB, cleared environment, process-group
  termination, repository-contained source spans, and verdict whitelist
  (`duplicate|distinct`, probability 0..1).
- Any failure leaves static output unchanged and emits one
  `eg: judge unavailable:` stderr note.

## Verification

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked --test cli reuse_phase2
cargo test --locked --test cli reuse_phase3
cargo test --locked --test cli reuse_phase4
cargo test --locked --test cli reuse_phase5
cargo test --locked --test mcp parity
cargo test --locked
python3 -I tests/scripts/reuse_oracle.py --help
```
