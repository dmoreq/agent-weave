# Security Policy

## Supported versions

The latest release on the `main` branch is supported.

## Reporting a vulnerability

Please use GitHub's private vulnerability reporting on this repository
("Security" tab -> "Report a vulnerability"). Do not open a public issue for
security problems. If the form is unavailable, contact the maintainer through
the repository's issue tracker without details and ask for a private channel.

We aim to acknowledge reports within 7 days.

## Security model

- aw runs locally, reads repository files, and writes only inside `.aw/`.
- It performs no network I/O at runtime and spawns no long-lived processes.
- The MCP server is a stdio JSON-RPC surface; frames are size-capped and tool
  panics are isolated.
- The index stores symbol names, signatures, paths, and call edges. It does
  not store source file bodies.
