# Contributing to kg

kg is a small, measurement-driven tool. Contributions are welcome under the
project's dual MIT/Apache-2.0 license.

## Ground rules

1. **Read the spec first.** `docs/designs/2026-10-31_kg-rust-code-index.md`
   is normative: component layout, schema (additive-only), resolution
   algorithm, response contract, exit codes, safety invariants. Code that
   deviates needs an amendment entry first.
2. **Safety invariants are non-negotiable** (CI-enforced): zero network, no
   long-lived child processes, read-only outside `.kg/`, no agent-config
   writes. A pull request that weakens any enforcement test is rejected.
3. **Honesty rule**: every gate number is reported as measured, with the
   command attached, run against the binary being shipped. Label historical
   or reported numbers as such.
4. **No new dependencies** beyond the pinned set; the lockfile allowlist
   check must stay green. No `unsafe`.
5. **Contract is byte-stable**: changes to stdout/stderr formats or exit
   codes require a contract-bump note in the spec.

## Workflow

```
cargo build --locked --release
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
bash tests/scripts/allowlist_check.sh Cargo.lock
bash tests/scripts/write_path_check.sh
```

Keep diffs small and focused; every resolution-behavior change ships with a
fixture in `tests/fixtures.rs` that asserts the exact edge outcome (including
what must NOT resolve). Commits: `type(scope): summary`
(feat/fix/docs/test/chore).

## Review

Changes to the spec or to a gate end with an independent review against the
deliverable paths and acceptance criteria only (no author reasoning).
Verdicts are APPROVE / NEEDS FIXES with severity-tagged findings; fix bound is
two cycles, then escalate. Review records live in `docs/baselines/reviews/`
in the private evidence vault.

## Security

See `SECURITY.md` for private vulnerability reporting.
