# Contributing to kg

Private project — contributions are owner-directed. This document records the
rules any change (human or agent) must follow.

## Ground rules

1. **Read the spec first.** `docs/designs/2026-10-31_kg-rust-code-index.md`
   (v1.1 + Amendments 1-3, canonical in this repo) is normative: component layout, schema v1
   (additive-only), resolution algorithm, response contract, exit codes,
   safety invariants. Code deviating from it needs an amendment entry first.
2. **Safety invariants are non-negotiable** (CI-enforced): zero network, no
   long-lived child processes, read-only outside `.kg/`, no agent-config
   writes. A PR that weakens any test for them is rejected regardless of
   other merits.
3. **Honesty rule**: every gate number is reported as measured, with the
   command attached, run against the binary being shipped. Never report a
   measurement from a stale build (this cost us one review cycle already).
4. **No new dependencies** beyond the pinned five+grammars; the allowlist
   check (`tests/scripts/allowlist_check.sh`) must stay green. No unsafe.
5. **Contract is byte-stable**: changes to stdout/stderr formats or exit
   codes require a contract-bump entry in the spec and a wrapper update in
   the same change.

## Change workflow

```
cargo build --locked --release
cargo clippy --all-targets -- -D warnings
cargo test --locked
bash tests/scripts/allowlist_check.sh Cargo.lock
bash tests/scripts/write_path_check.sh
```

- Keep diffs small and focused; isolate refactors from features.
- Every resolution-behavior change ships with a fixture in `tests/fixtures.rs`
  asserting the exact edge outcome (see `factory_nested_constructor_types_nothing`
  for the pattern: assert what must NOT resolve, too).
- Commits: `type(scope): summary` (feat/fix/docs/test/chore), matching the
  existing history.

## Review protocol

Each phase (and any large change) ends with an independent review by a
stateless reviewer given only the deliverable paths and acceptance criteria —
never the author's reasoning. Verdicts are APPROVE / NEEDS FIXES with
severity-tagged findings; the fix bound is <= 2 cycles, then the item
escalates. Review records live in `docs/baselines/reviews/`.

## Gates

G1-G8 are defined in the spec's Gate Registry with kill actions. Re-measure
with `tests/scripts/timing.sh`, `tests/scripts/precision_recall.py`, and the
transcripts harness in `docs/baselines/harness/`. Gate claims go in
`docs/baselines/transcripts/SUMMARY.md` with the command attached.

## License note

The repo is private; `LICENSE` is an all-rights-reserved notice. Replacing it
with an open-source license is an owner decision (OQ1) and must land before
any public release.
