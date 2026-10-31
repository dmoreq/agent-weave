# Security Policy

## Security posture

kg is a local, consultative CLI. Its four security invariants are
CI-enforced (see `.github/workflows/ci.yml` and `tests/scripts/`):

1. **Zero network** — no network syscalls; Seatbelt (macOS) and seccomp
   (Linux) CI jobs run `kg build` with networking denied; the Cargo.lock
   allowlist rejects any dependency linking a resolver/TLS stack.
2. **No long-lived processes** — no child process survives a subcommand
   (proc-hygiene test).
3. **Read-only over source** — the only writes are inside `.kg/`
   (tree-snapshot test with content hashes; write-path grep allows writes
   only in `db.rs`/`doctor.rs` and the atomic-swap rename).
4. **No behavior hijack** — no hooks, no agent-config or settings writes, no
   prompt injection; the CLI never reads or modifies agent configuration.

No secrets are handled: kg stores none, requests none, and the index
contains code structure only (paths, symbols, edges) — never file contents.

## Supported versions

Only the latest commit on `main` is supported. The index schema is
version-locked: a binary refuses to open an index built by a different
schema version (exit 5) rather than migrating — rebuild instead.

## Reporting

Private project: report issues directly to the owner (Quy Doan) or open a
restricted issue describing the impact without exploit details. Please
include the `kg doctor` output and the command that triggered the behavior.
