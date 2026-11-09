# Concepts Brief: Engram Codebase Rename (`agent-weave` / `aw` -> `engram` / `eg`)

- **Author**: Antigravity Spec Team
- **Date**: 2026-11-09
- **Status**: Review Complete (Post 4-Lens Fan-Out)
- **Predecessor Artifact**: [`codebase_rename_plan.md`](file:///Users/quy.doan/.gemini/antigravity/brain/b820c413-061a-40ac-87ad-c3834bc9fbac/codebase_rename_plan.md)

---

## 1. Executive Summary & Review Verdict

Four stateless, read-only Goldfish reviewers evaluated the rename architecture across distinct analytical lenses:
1. **Contrarian / YAGNI**: Evaluated blast radius, scope creep, and minimal working diff.
2. **UX / Ergonomics**: Evaluated CLI usability, cognitive load, and command collision.
3. **Migration & Compatibility**: Evaluated SQLite database lifecycle, version skew, and agent MCP disruption.
4. **Security & Threat Modeling**: Evaluated trust boundaries, symlink traversal, directory permissions, and malicious repo-planted state.

### Arbitration Precedence Applied
$$\text{Security \& Privacy} > \text{Correctness \& Data Integrity} > \text{Budget \& Quota Ceilings} > \text{Simplicity \& YAGNI} > \text{Ergonomics \& Feature Convenience}$$

---

## 2. Synthesized Review Findings

### Critical Findings (Blockers)

#### 1. Primary Agent Memory Loss & Silent Abandonment (Correctness / Data Integrity)
- **Reviewers**: Migration Goldfish (Finding 1), Contrarian Goldfish (Finding 1), Ergonomics Goldfish (Finding 1), Security Goldfish (Finding 7).
- **Consensus**: `memory.db` is primary user data, not a cache. Shifting directory lookup from `.aw/` to `.engram/` without a migration path silently severs access to stored gotchas, architectural decisions, and tasks.
- **Required Mitigation**:
  - In `src/db/memory_store.rs`, implement safe read-forward/migration: if `.engram/memory.db` is absent but `.aw/memory.db` exists, automatically copy or migrate the database (including `-wal` and `-shm`), run `PRAGMA integrity_check`, and notify the user.
  - Implement a check in `eg doctor` reporting legacy `.aw` presence with an actionable command.

#### 2. Disruption of Active Agent MCP Clients & Tool Permissions (Correctness / Ergonomics)
- **Reviewers**: Migration Goldfish (Finding 2), Contrarian Goldfish (Finding 2).
- **Consensus**: Immediate removal of `aw` and rename of MCP server to `"engram"` abruptly breaks active agent host configs (`mcpServers["agent-weave"]`) and Antigravity permission grants (`mcp(agent-weave/*)`).
- **Required Mitigation**:
  - `scripts/install.sh` must configure both `"engram"` and `"agent-weave"` (or support transition symlink `aw -> eg`).
  - Update permission grants in `~/.gemini/config/config.json` to include `mcp(engram/*)` while retaining `mcp(agent-weave/*)`.

#### 3. Untrusted Repo-Planted Memory Store Injection (Security & Threat Modeling)
- **Reviewers**: Security Goldfish (Finding 1).
- **Consensus**: An attacker can commit `.engram/memory.db` into a public Git repo. A newly cloned workspace would blindly execute `eg mem query` and inject malicious instructions into the LLM context.
- **Required Mitigation**: Before opening repo-local `.engram/memory.db`, check if any file in `.engram/` is tracked by Git (`git ls-files .engram`). If tracked, reject with exit code 2 and demand removal.

---

### Important Findings

#### 4. Privacy Opt-Out and Environment Variable Bypass (Security & Migration)
- **Reviewers**: Security Goldfish (Finding 2), Migration Goldfish (Finding 4), Ergonomics Goldfish (Finding 2).
- **Consensus**: Renaming `AW_NO_JOURNAL` and `AW_GLOBAL_DIR` to `ENGRAM_*` without fallback causes privacy flags to fail open, writing sensitive query strings to `~/.engram/usage.log`.
- **Required Mitigation**: Dual-resolution in `src/journal.rs` and `src/db/`: resolve `ENGRAM_<VAR>`, falling back to `AW_<VAR>` if unset. Disable journaling if either `ENGRAM_NO_JOURNAL=1` or `AW_NO_JOURNAL=1`.

#### 5. Symlink Traversal Vulnerability in `clear_eg_dir` (Security & Threat Modeling)
- **Reviewers**: Security Goldfish (Finding 5).
- **Consensus**: In `src/doctor.rs`, recursive deletion via `entry.path().is_dir()` follows symlinks, enabling malicious directory symlinks in `.engram/` to delete external targets during `doctor --invalidate`.
- **Required Mitigation**: Inspect entries with `fs::symlink_metadata`. If `meta.file_type().is_symlink()`, unlink directly with `fs::remove_file` instead of recursive deletion.

#### 6. Cross-Generation Concurrency Split-Brain on Build Locks (Security & Migration)
- **Reviewers**: Security Goldfish (Finding 3), Migration Goldfish (Finding 5).
- **Consensus**: An active `aw build` holding `.aw/build.lock` and an `eg build` acquiring `.engram/build.lock` run unsynchronized.
- **Required Mitigation**: In `BuildLock::acquire`, inspect whether legacy `.aw/build.lock` is held by an active PID before acquiring `.engram/build.lock`.

#### 7. Insecure File Modes on Multi-User Hosts (Security & Threat Modeling)
- **Reviewers**: Security Goldfish (Finding 4).
- **Consensus**: Creating `~/.engram/` and `usage.log` with default umask (`0755` / `0644`) exposes multi-repo queries and memories to other local users.
- **Required Mitigation**: Set directory permissions to `0700` and file permissions to `0600` on Unix systems (`#[cfg(unix)]`).

#### 8. Binary Name Collision (`eg`) with POSIX Aliases and Unqualified PATH Execution (UX & Security)
- **Reviewers**: Ergonomics Goldfish (Finding 3), Security Goldfish (Finding 6).
- **Consensus**: Two-letter `eg` can collide with `egrep` aliases or third-party tools.
- **Required Mitigation**: In `scripts/eg-dashboard`, resolve binary path via absolute priority (`$REPO_ROOT/target/release/eg`, `~/.cargo/bin/eg`, `~/.local/bin/eg`) and assert `--version` output before delegating.

---

### Suggestions & Nits

- **Suggestion 1 (Finding 4 in Ergonomics)**: Add `clap` alias `memory` for subcommand `mem`.
- **Suggestion 2 (Finding 8 in Security)**: Ensure path exclusion in `src/walk.rs` uses strict segment comparison (`seg == ".engram" || seg == ".aw"`) so application directories like `src/engram/` are indexed.
- **Suggestion 3 (Finding 7 in Ergonomics & Contrarian)**: `eg doctor` should detect orphaned `.aw/` directories and provide cleanup advice.
