<!--
  Keep diffs small and focused. Isolate refactors from features.
  Every resolution-behavior change ships with a fixture asserting the exact
  edge outcome — including what must NOT resolve.
-->

## What

<!-- One paragraph: the change and why it is needed. -->

## Spec conformance

- [ ] No spec deviation, or an Amendment entry exists in
      `docs/designs/2026-10-31_kg-rust-code-index.md` (in-repo, canonical)
- [ ] Response contract unchanged, or contract-bump note + wrapper update
      included
- [ ] Safety invariants untouched (network / processes / read-only / no
      hijack)

## Verification

<!-- Paste actual command output. Numbers only from the binary being shipped. -->

```
cargo clippy --all-targets -- -D warnings   # result
cargo test --locked                          # result
bash tests/scripts/allowlist_check.sh Cargo.lock
bash tests/scripts/write_path_check.sh
```

- [ ] Gate numbers re-measured if gates are touched (command attached)
- [ ] No new dependencies (allowlist green)

## Review

- [ ] Independent review verdict recorded in `docs/baselines/reviews/`
      (large/phase changes; <= 2 fix cycles)
