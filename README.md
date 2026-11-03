# kg — navigate code relationships locally

[![CI](https://github.com/quy-doan/kg/actions/workflows/ci.yml/badge.svg)](https://github.com/quy-doan/kg/actions/workflows/ci.yml)

Before changing a function, you need to find its callers. Before following a
request through a codebase, you need to connect the functions along the way.
kg gives people and coding agents those connections without repeated name
searches and file reads.

kg reads Python, JavaScript, TypeScript, and Rust source into a local index.
Its answers point to files and lines you can inspect. It does not run your code
or prove what will happen at runtime.

## What changes when you use kg?

| Your task | Without kg | With kg |
|---|---|---|
| Find callers before changing a function | Search its name, inspect matches, and follow imports or aliases | `kg callers` lists call sites kg could connect to that function |
| Follow a call route from A to B | Open each function and trace its next calls | `kg path` returns one shortest route through the connections in its index |
| Understand a function's surroundings | Collect its definition, callers, and calls separately | `kg context` brings those pieces together |
| Check which files may need attention after a change | Search for references and connect dependencies manually | `kg impact` lists files that call into the target file; optional flags include imports and inheritance |
| Decide where to read next | Read a file to discover its structure | `kg symbols --outline` lists definitions and signatures without their bodies |

These are workflow differences, not a promise of complete coverage or universal
savings. Existing editor references, language servers, and grep may already
answer a small question efficiently. Keep them for verification and for
relationships kg cannot identify.

### What has been measured?

In a historical navigation experiment on one repository and one host/model,
agents found **98.6% of expected call sites with kg, versus 81.9% with grep-only**.
When instructed to query kg first, they used **27.3% fewer input tokens** than
the grep-only group, with the same 98.6% recall. Input-token means used 10 runs
per group. Simply making kg available did not produce the same saving.

Other experiments found that benefits depend on the task and host/model.
Symbol search and duplicate detection did not consistently reduce effort;
test and API lookups did not show savings in their narrow measured tasks.
These are reported historical results, not measurements of every current build.
See [Evaluation](docs/EVALUATION.md) for comparisons, sample sizes, limitations,
and what can be checked from the public repository.

## Try it

You need Rust 1.90 or later, Cargo, Git, and a C compiler. Install from a clone;
kg is not published on crates.io. CI builds and tests on macOS and Linux.

```sh
git clone https://github.com/quy-doan/kg.git
cd kg
cargo install --path . --locked
kg build
kg symbols src/cli.rs --outline
kg context discover_root
```

The last two commands inspect kg's own code. To use another repository, run
`kg build` from that repository, then ask a question using one of its symbol
names or repository-relative file paths:

```sh
kg callers YourClass.your_method
kg impact path/to/your_file.py
```

Replace these example names with your own. Results use `path:line` locations.
Rebuild with `kg build` after source edits; kg never rebuilds automatically.
See [Getting started](docs/getting-started.md) for setup and result handling.

## Choose the right question

| Question | Command |
|---|---|
| Who calls a function or method? | `kg callers <symbol>` |
| What does it call, including further calls? | `kg callees <symbol> --depth 3` |
| How does A call through to B? | `kg path <from> <to>` |
| What surrounds this symbol? | `kg context <symbol>` |
| Which files call into this file? | `kg impact <file>` |
| Where is a symbol with this name? | `kg search '<terms>'` |
| What definitions are in a file or directory? | `kg symbols <path> --outline` |

For test candidates, import relationships, inheritance, duplication, complexity,
and other commands, see the [CLI reference](docs/cli-reference.md).
Use the [workflow guide](docs/usage.md) to decide what to do with a result.

## Know what the answer does not prove

- **Not every relationship is visible in source.** Dynamic dispatch, untyped
  receivers, and external calls can remain unresolved. No result does not prove
  that no caller, path, dependency, or test exists.
- **Change impact is a starting point for inspection.** It identifies indexed
  dependencies, not every runtime effect or every test you must run.
- **Results can be limited or stale.** Read result counts and warnings. Rebuild
  after edits, and request more results when you need the complete indexed set.
- **Candidates need review.** Duplicate, unused-code, and test candidates are
  not proof of equivalent behavior, safe deletion, or complete test coverage.
- **One index covers one repository.** kg does not answer cross-repository
  questions or search by the meaning of arbitrary behavior descriptions.
- **Language support has boundaries.** Dart is not indexed. Supported source
  files larger than 1 MiB are skipped. See the reference for other limits.

## Local by design

At runtime, kg makes no network requests and changes no source files. Its index
and local usage records live under `.kg/`. It installs no hooks, runs no
background daemon, and does not edit your agent configuration.

Usage records are enabled by default. Pass `--no-journal` or set
`KG_NO_JOURNAL=1` to disable them. Source names, signatures, paths, and query
records can be sensitive: do not publish `.kg/` unintentionally.
See [Security](SECURITY.md) for the security model.

## Use it with a coding agent

- **MCP** gives an agent access to kg tools through `kg mcp`.
- **The kg-first skill** tells an agent when and how to use those tools,
  handle incomplete results, and fall back to source inspection.

Neither is installed automatically. See [Agent integration](docs/agent-integration.md)
for setup, verification, and the difference between tool access and routing.

## Documentation and contribution

- [Documentation index](docs/README.md): guides, reference, and technical design.
- [Evaluation](docs/EVALUATION.md): reported benefits, costs, and evidence limits.
- [Support](SUPPORT.md): common errors and how to report a problem.
- [Contributing](CONTRIBUTING.md): development setup, checks, and review rules.
- [Changelog](CHANGELOG.md): changes and required user actions.
- [Code of conduct](CODE_OF_CONDUCT.md) and [Security policy](SECURITY.md).

kg is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
See [LICENSE](LICENSE) and [NOTICE](NOTICE) for details.
