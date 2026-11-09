<!--
  Keep diffs small and focused. Isolate refactors from features.
  Every resolution-behavior change ships with a fixture asserting the exact
  edge outcome — including what must NOT resolve.
-->

## Change and motivation

<!-- One paragraph: the change and why it is needed. -->

## Specification and compatibility

- [ ] Matches architectural constraints defined in `docs/architecture/code-index.md`
      and `docs/architecture/memory-store.md`
- [ ] Output formats and exit codes unchanged, or specification contract-bump
      note and affected integration updates included
- [ ] Runtime safety preserved: no network, long-lived child processes, writes
      outside `.engram/`, or agent-configuration writes; enforcement tests not weakened
- [ ] Schema changes are additive, if applicable

## Verification

<!-- Record commands and actual results, including failures and checks not run.
     For prose-only changes, check links/examples and git diff --check; skip runtime tests.
     The block below lists the main CI checks for runtime changes, not executed results. -->

```sh
cargo fmt --all -- --check
cargo build --locked --release
cargo clippy --all-targets -- -D warnings
cargo test --locked
bash tests/scripts/allowlist_check.sh Cargo.lock
bash tests/scripts/write_path_check.sh
```

- [ ] Resolution changes include fixtures for exact outcomes, including what must not resolve (or not applicable)
- [ ] Changed accuracy/performance claims include commands, revision, binary, inputs, environment, and limitations (or not applicable)
- [ ] No new dependencies beyond the pinned set; allowlist check passes for dependency changes
- [ ] No `unsafe` code introduced

Results and checks not run:

## Review

<!-- Required for specification changes and changes to acceptance checks or thresholds.
     Give the independent reviewer changed files and acceptance criteria, not author reasoning.
     Record APPROVE or NEEDS FIXES here with severity and readable evidence for findings.
     Resolve Critical and Important findings. After at most two fix-and-review cycles,
     escalate unresolved findings to the maintainer. No private review archive is required. -->

- Review required or not applicable (explain):
- Independent reviewer and verdict:
- Findings, resolutions, and unresolved blockers:
