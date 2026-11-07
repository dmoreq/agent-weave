# Get started with agent-weave (`aw`)

Build a local index, find a symbol, and open its source location. This tutorial uses aw's own repository.

aw does not replace source reading, grep, or a language server. It provides indexed navigation without a runtime network connection.

## 1. Install from a clone

You need Git, Rust 1.90 or newer with Cargo, and a C compiler. SQLite and the tree-sitter parsers compile from bundled sources.

CI defines build jobs for macOS and Linux, plus an installation smoke check on Linux. This guide does not promise Windows support or a specific CPU architecture.

```sh
git clone https://github.com/quy-doan/agent-weave.git
cd agent-weave
cargo install --path . --locked
aw --version
aw --help
```

Cargo installs the executable into its configured installation directory, normally `$HOME/.cargo/bin`. Add that directory to `PATH` if your shell cannot find `aw`.

Install from this checkout, not with `cargo install agent-weave`: this package is not published on crates.io. Dependency downloads during installation can require network access; the `aw` runtime does not.

## 2. Build the index

From the cloned repository:

```sh
aw build
aw doctor
```

`build` writes `.aw/index.db` and reports files, symbols, edges, parse errors, unresolved calls, and build timings. Read the actual counts from your checkout; this guide does not supply a captured output transcript.

`doctor` checks integrity and reports index versions, parse coverage, changed indexed files, and leftover build files. A successful build does not mean every call resolved.

aw discovers the nearest ancestor containing `.git`. To index a different directory, pass its root explicitly:

```sh
aw --root . build
```

File arguments in queries remain relative to the selected root, even when you invoke aw from a subdirectory. aw writes only under that root's `.aw/` during normal operation; it does not add ignore rules or change agent configuration.

## 3. Find and inspect real code

```sh
aw symbols src/cli.rs --outline
aw search run_build
aw context run_build
```

- `symbols` lists definitions in `src/cli.rs`, with one-line signatures when available.
- `search` ranks identifier matches and accepts either a bare name or a qualified name.
- `context` combines the definition range, direct callers, and direct callees. It does not print function bodies.

Open a returned `path:line` in your editor. Lines are 1-based and paths are repository-relative.

These examples name existing files and definitions, not guaranteed result counts. If a name becomes ambiguous, copy a full qualified name from the candidate list and repeat the query.

## 4. Check likely callers before editing

```sh
aw callers run_build
aw impact src/build.rs
```

`callers` points to indexed call sites. `impact` lists files that call into `src/build.rs`, not files that `src/build.rs` calls.

Use these locations to choose what to inspect next. Empty results do not prove the function is unused or a change is safe.

Without aw, search for the definition and its references, then read each match. Keep that fallback available:

```sh
git grep -n run_build -- src tests
```

## 5. Rebuild after source changes

```sh
aw build
```

aw does not watch files or rebuild automatically. Normal builds reuse unchanged extraction data; `aw build --force` re-extracts everything and also bypasses the parse-error-rate abort.

Queries can warn about changed result files, but no warning is not a whole-repository freshness guarantee. Rebuild after adding, deleting, renaming, or editing source files.

## Know what is indexed

| Language | Extensions |
|---|---|
| Python | `.py` |
| TypeScript | `.ts`, `.tsx`, `.mts`, `.cts` |
| JavaScript | `.js`, `.jsx`, `.mjs`, `.cjs` |
| Rust | `.rs` |

Files larger than 1 MiB are skipped. `.aw/` is always excluded. In Git repositories, aw uses tracked and untracked-unignored paths when Git enumeration succeeds; otherwise it uses textual ignore rules.

Other languages, including Dart, are not indexed. Dynamic calls, external dependencies, macros, and framework conventions can leave relationships missing.

## Remove local index data

Run this only when you intend to delete aw's local data for the selected repository:

```sh
aw doctor --invalidate
```

This removes the entire `.aw/` directory, including the index, journals, and any other files stored there. It does not uninstall the executable or modify source files. Run `aw build` to start again.

## Continue

- [Usage guide](usage.md): select queries for editing and investigation.
- [Concepts](concepts.md): symbols, edges, pointers, and static limits.
- [CLI reference](cli-reference.md): options, result interpretation, exits, and journal controls.
- [Documentation index](README.md): integrations, evaluation, and contribution guidance.
