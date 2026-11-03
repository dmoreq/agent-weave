# Get started with kg

Build a local index, find a symbol, and open its source location. This tutorial uses kg's own repository.

kg does not replace source reading, grep, or a language server. It provides indexed navigation without a runtime network connection.

## 1. Install from a clone

You need Git, Rust 1.90 or newer with Cargo, and a C compiler. SQLite and the tree-sitter parsers compile from bundled sources.

CI defines build jobs for macOS and Linux, plus an installation smoke check on Linux. This guide does not promise Windows support or a specific CPU architecture.

```sh
git clone https://github.com/quy-doan/kg.git
cd kg
cargo install --path . --locked
kg --version
kg --help
```

Cargo installs the executable into its configured installation directory, normally `$HOME/.cargo/bin`. Add that directory to `PATH` if your shell cannot find `kg`.

Install from this checkout, not with `cargo install kg`: this package is not published on crates.io. Dependency downloads during installation can require network access; kg's runtime does not.

## 2. Build the index

From the cloned repository:

```sh
kg build
kg doctor
```

`build` writes `.kg/index.db` and reports files, symbols, edges, parse errors, unresolved calls, and build timings. Read the actual counts from your checkout; this guide does not supply a captured output transcript.

`doctor` checks integrity and reports index versions, parse coverage, changed indexed files, and leftover build files. A successful build does not mean every call resolved.

kg discovers the nearest ancestor containing `.git`. To index a different directory, pass its root explicitly:

```sh
kg --root . build
```

File arguments in queries remain relative to the selected root, even when you invoke kg from a subdirectory. kg writes only under that root's `.kg/` during normal operation; it does not add ignore rules or change agent configuration.

## 3. Find and inspect real code

```sh
kg symbols src/cli.rs --outline
kg search run_build --exact
kg context run_build
```

- `symbols` lists definitions in `src/cli.rs`, with one-line signatures when available.
- `search --exact` finds definitions whose bare name or full qualified name equals `run_build`.
- `context` combines the definition range, direct callers, and direct callees. It does not print function bodies.

Open a returned `path:line` in your editor. Lines are 1-based and paths are repository-relative.

These examples name existing files and definitions, not guaranteed result counts. If a name becomes ambiguous, copy a full qualified name from the candidate list and repeat the query.

## 4. Check likely callers before editing

```sh
kg callers run_build
kg impact src/build.rs
```

`callers` points to indexed call sites. `impact` lists files that call into `src/build.rs`, not files that `src/build.rs` calls.

Use these locations to choose what to inspect next. Empty results do not prove the function is unused or a change is safe.

Without kg, search for the definition and its references, then read each match. Keep that fallback available:

```sh
git grep -n run_build -- src tests
```

## 5. Rebuild after source changes

```sh
kg build
```

kg does not watch files or rebuild automatically. Normal builds reuse unchanged extraction data; `kg build --force` re-extracts everything and also bypasses the parse-error-rate abort.

Queries can warn about changed result files, but no warning is not a whole-repository freshness guarantee. Rebuild after adding, deleting, renaming, or editing source files.

## Know what is indexed

| Language | Extensions |
|---|---|
| Python | `.py` |
| TypeScript | `.ts`, `.tsx`, `.mts`, `.cts` |
| JavaScript | `.js`, `.jsx`, `.mjs`, `.cjs` |
| Rust | `.rs` |

Files larger than 1 MiB are skipped. `.kg/` is always excluded. In Git repositories, kg uses tracked and untracked-unignored paths when Git enumeration succeeds; otherwise it uses textual ignore rules.

Other languages, including Dart, are not indexed. Dynamic calls, external dependencies, macros, and framework conventions can leave relationships missing.

## Remove local index data

Run this only when you intend to delete kg's local data for the selected repository:

```sh
kg doctor --invalidate
```

This removes the entire `.kg/` directory, including the index, journals, and any other files stored there. It does not uninstall the executable or modify source files. Run `kg build` to start again.

## Continue

- [Usage guide](usage.md): select queries for editing and investigation.
- [CLI reference](cli-reference.md): options, result interpretation, exits, and journal controls.
- [Documentation index](README.md): integrations, evaluation, and contribution guidance.
