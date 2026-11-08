# Getting Started with agent-weave (`aw`)

Get up and running with `agent-weave` in under 5 minutes. Learn how to index a repository, navigate code relationships deterministically, prevent duplicate code, and carry zero-token memory across sessions.

---

## 1. Installation

### Requirements
- **Rust**: 1.90 or later (`rustc --version`)
- **Cargo**: Bundled with Rust toolchain
- **Git**: For repository discovery
- **C Compiler**: `cc` or `clang` for bundled SQLite and tree-sitter parsers

### 1-Click Automated Setup (Binary + All Agents)
Run the automated installer to compile `aw`, install binaries (`aw`, `aw-dashboard`), and configure detected coding agents (Antigravity, Grok, Zcode, OpenCode, Zed, Claude, Cursor):

```bash
git clone https://github.com/dmoreq/agent-weave.git
cd agent-weave
./install.sh
```

### Manual Install from Source
Alternatively, compile and install the CLI binary only:

```bash
cargo install --path . --locked
```

Confirm that the binary is available in your `$PATH` (typically in `~/.cargo/bin`):

```bash
aw --version
aw --help
```

*Note*: `aw` is built entirely with offline-capable bundled sources. Runtime execution requires zero network access.

---

## 2. The First 5 Minutes: Core USPs in Action

Navigate to any supported code repository (Python, TypeScript, JavaScript, or Rust). You can also run these commands directly inside the `agent-weave` checkout.

### Step 1: Build the Local Index (USP 1)

Generate the local SQLite AST graph:

```bash
aw build
```

`aw` indexes source files into `.aw/index.db`. Initial builds complete in a few hundred milliseconds; subsequent incremental rebuilds take **~0.02s**.

Verify index health and coverage:

```bash
aw doctor
```

`aw doctor` checks database integrity, parser health, and changed file ratios.

---

### Step 2: Deterministic Code Navigation (USP 1)

Traditional agent navigation burns thousands of tokens reading entire files to locate references. `aw` gives you exact, sub-2ms pointers:

#### Inspect Symbol Surroundings
Get definition ranges, signatures, callers, and callees in a single query:

```bash
aw context run_build
```

#### Find Incoming Callers
Trace direct and transitive callers without grepping:

```bash
# Direct callers
aw callers run_build

# Transitive callers up to 2 hops away
aw callers run_build --depth 2
```

Outputs list exact `path:line qualname [depth=N]` pointers. Open only the specific lines in your editor.

#### Assess Change Impact
Identify every file that calls into a target file before you edit it:

```bash
aw impact src/build.rs
```

---

### Step 3: Anti-Reinvention Discovery & Verification (USP 2)

Unguided AI agents frequently reinvent existing helpers because they cannot easily find canonical functions. `aw` provides two-moment protection:

#### Pre-Write Discovery
Search for existing helpers before implementing new utility functions:

```bash
aw search "read bounded"
```

Output delivers **Concise Interface Cards** ranked by caller popularity:
```text
src/walk.rs:42 fn read_bounded(path: &Path, max_bytes: usize) -> Result<Vec<u8>> [callers=14]
```
The `[callers=14]` tag signals that `read_bounded` is an established, canonical helper that you should reuse.

#### Post-Edit Verification
Verify that your changes did not introduce duplicate logic or bypass existing helpers:

```bash
aw reuse
```

`aw reuse` analyzes callee footprints and interface similarities across four calibrated detectors to surface duplicate function candidates.

---

### Step 4: Zero-Token Layered Memory (USP 3)

Avoid the token overhead of external memory servers. Persist durable decisions, gotchas, and preferences using SQLite FTS5 in under 2ms:

#### Save a Code-Anchored Repository Decision
Anchor architectural decisions directly to source files:

```bash
aw mem write resolver-strategy --kind decision \
  --body "Resolver maintains explicit confidence tiers. Ambiguous calls remain dst_id NULL." \
  --refs src/extract/resolve.rs
```

If `src/extract/resolve.rs` is renamed or refactored, `aw` flags `[stale-ref]` alerts automatically.

#### Save Cross-Project Developer Preferences
Use `--global` to persist global habits and profile settings in `~/.aw/memory.db`:

```bash
aw mem write dev-profile --kind decision --global \
  --body "Preferred stack: Rust. Coding style: Minimalist. Reject unrequested abstractions."
```

#### Layered Retrieval
Query across both local repository decisions and global profile preferences:

```bash
aw mem query resolver
aw mem query profile
```

Local repository notes automatically shadow global notes with the same key. Global notes are labeled with `[global]`.

---

## 3. What is Indexed

`aw` indexes code using AST grammars for the following file extensions:

| Language | Extensions |
|---|---|
| **Rust** | `.rs` |
| **Python** | `.py` |
| **TypeScript** | `.ts`, `.tsx`, `.mts`, `.cts` |
| **JavaScript** | `.js`, `.jsx`, `.mjs`, `.cjs` |

### Index Boundaries
- **Size Cap**: Individual files larger than 1 MiB are skipped to avoid indexing large generated assets.
- **Git Awareness**: Automatically respects `.gitignore` rules and excludes untracked ignored paths.
- **Confinement**: `.aw/` directories and external build artifacts are excluded.

---

## 4. Resetting Local Data

Derived data (the SQLite AST index and usage journals) can be cleaned up at any time:

```bash
aw doctor --invalidate
```

`--invalidate` removes `.aw/index.db` and temporary locks, but safely **preserves** your agent-authored project memory (`.aw/memory.db`). To back up memory notes before a full cleanup, run:

```bash
aw mem export > memory_backup.jsonl
```

---

## Next Steps

- **[Usage Guide](usage.md)**: Master real-world developer and agent scenarios.
- **[CLI Reference](cli-reference.md)**: Explore the complete 10-command syntax and exit codes.
- **[Agent Integration](agent-integration.md)**: Connect `aw` to Claude Desktop, Cursor, or Antigravity via MCP.
