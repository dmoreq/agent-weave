# Contributing to aw

Contributions are welcome under the project's dual MIT/Apache-2.0 license.
Start with a focused issue or pull request that explains the user problem.

## Ground rules

1. **Read the [specification](docs/designs/2026-10-31_agent-weave-rust-code-index.md) first.**
   It defines component layout, additive-only schema changes, resolution behavior,
   output formats, exit codes, and safety requirements. Add a specification
   amendment before implementing a deviation.
2. **Preserve runtime safety:** no network access, no long-lived child processes,
   no writes outside `.aw/`, and no agent-configuration writes. Do not weaken
   safety enforcement tests.
3. **Report measurements honestly.** Attach commands, revision, environment, and
   inputs to accuracy or performance results. Identify the binary measured;
   label historical or internally reported results. Do not claim that unavailable
   evidence is publicly reproducible.
4. **Do not add dependencies** beyond the pinned set. Keep the lockfile allowlist
   check passing. Do not introduce `unsafe` code.
5. **Preserve exact output formats and exit codes.** Changes require a
   contract-bump note in the specification and updates to affected integrations.

## Development setup

Use Rust 1.90 or newer, Cargo, a C compiler, and the `rustfmt` and `clippy`
components. The shell checks also require Bash and Python 3. Run commands from
the repository root. See [.github/workflows/ci.yml](.github/workflows/ci.yml)
for the configured checks and operating-system matrix; configuration is not a
claim that a particular run passed.

### Code map

| Area | Files |
|---|---|
| Commands and output errors | `src/cli.rs`, `src/service.rs`, `src/main.rs`, `src/error.rs` |
| File discovery, parsing, and extraction | `src/walk.rs`, `src/parse.rs`, `src/extract.rs`, `src/extract/` |
| Index construction and storage | `src/build.rs`, `src/db.rs` |
| Queries and analysis | `src/query.rs`, `src/query/`, `src/insights.rs` |
| Diagnostics and usage journal | `src/doctor.rs`, `src/journal.rs` |
| MCP integration | `src/mcp.rs` (schemas from `src/service.rs`) |
| Behavior fixtures and safety checks | `tests/fixtures.rs`, `tests/scripts/` |

## Make and verify a change

Keep diffs small and focused. Separate refactoring from behavior changes.
Every resolution-behavior change needs a fixture in `tests/fixtures.rs` that
asserts the exact outcome, including what must **not** resolve.

Run focused tests while developing. Before submitting runtime changes, use the
same main checks as CI:

```sh
cargo fmt --all -- --check
cargo build --locked --release
cargo clippy --all-targets -- -D warnings
cargo test --locked
bash tests/scripts/allowlist_check.sh Cargo.lock
bash tests/scripts/write_path_check.sh
```

CI also configures minimum-Rust-version, installation, dependency-audit, and
network-isolation checks. These do not all run in the command block above.
For prose-only changes, check links, examples, and `git diff --check`; do not
rerun runtime tests. Report checks not run and any failures without lowering
acceptance criteria.

## Submit a pull request

- Explain the problem, proposed behavior, and alternatives considered.
- Link the relevant specification section or amendment.
- Include reproducing fixtures and verification commands with their results.
- For measurements, state the scope and limitations of the evidence.
- Use commit messages in the form `type(scope): summary`, with
  `feat`, `fix`, `docs`, `test`, or `chore` as the type.

## Review

Specification changes and changes to acceptance checks or thresholds require
independent review. Give the reviewer the changed files and checkable acceptance
criteria, without the author's reasoning. Reviewers assess the deliverable itself.

Record the verdict in the pull request: `APPROVE` or `NEEDS FIXES`, with severity
and readable evidence for each finding. Resolve Critical and Important findings
before acceptance. Allow at most two fix-and-review cycles; then escalate unresolved
findings to the maintainer. Public contributors do not need a private review archive.

## Security

Follow [SECURITY.md](SECURITY.md) for vulnerability reporting. Do not publish
security details in an issue or pull request.
