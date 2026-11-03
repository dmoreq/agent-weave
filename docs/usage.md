# Use kg for code navigation

Use kg to narrow the source you need to read. Verify decisions against source and tests; indexed relationships are not a complete program model.

The examples below use kg's repository after `kg build`. They show commands, not recorded output or guaranteed nonempty results.

## Choose a query

| Your task | Start with | Without kg |
|---|---|---|
| Find an unfamiliar symbol | `kg search 'parse tree'` | Search names and read matches with grep |
| Read a file's structure | `kg symbols src/build.rs --outline` | Read the file or use an editor outline |
| Inspect a function before editing | `kg context run_build` | Read its definition, then search callers and calls |
| Find callers | `kg callers run_build` | Search references and reject non-call matches |
| Follow outgoing calls | `kg callees run_build --depth 2` | Follow each call manually |
| Trace a route between functions | `kg path run_build fnv1a64` | Read and follow candidate call chains |
| Inspect dependent files | `kg impact src/hash.rs` | Search references and imports across files |
| Find possible tests | `kg tests src/hash.rs` | Read test files and the test runner's discovery rules |
| Inspect imports | `kg imports src/build.rs` | Read import declarations and resolve targets |
| Inspect public definitions | `kg api src` | Inspect visibility and export declarations |

See the [CLI reference](cli-reference.md) for all options and maintenance commands.

## Inspect a change's surroundings

Start with a fresh index, then inspect the target definition and direct neighbors:

```sh
kg build
kg context run_build
kg callers run_build --limit 100
kg impact src/build.rs --sites
```

`context` returns a definition range, caller sites, and callee definitions. Its per-section limit defaults to 10; an empty caller or callee section still permits a successful result.

`callers` identifies call sites, not all textual references. `impact` reverses the dependency direction: it finds files that depend on the target file, excluding that file itself.

By default, impact uses calls and emits one row per caller file with a site count. Use `--sites` for individual sites, or extend the relationships and depth explicitly:

```sh
kg impact src/hash.rs --edges all --depth 3 --context --explain
```

`--context` adds the enclosing callable when available. `--explain` adds the relationship kind and traversal depth, so you can distinguish direct calls from indirect dependencies.

Do not treat this list as every file that can break. Unresolved calls, external callers, imports, and inheritance relationships can be missing.

## Follow execution routes

```sh
kg callees run_build --depth 2
kg path run_build fnv1a64
```

`callees` shows a bounded forward call closure. `path` finds one shortest forward route through resolved call edges, with call-site pointers and a symbol chain.

Swap the two arguments to ask the reverse question. A missing route means no resolved route was found within the depth limit, not that execution cannot reach the destination.

Open each hop before changing behavior. Dynamic dispatch, callbacks, and macros can hide routes from static extraction.

## Find definitions and read structure

```sh
kg search 'parse tree' --path src --kind function
kg search run_build --exact
kg symbols src --outline
kg files --path src
kg api src --exclude-generated
```

Quote multiword search terms: the CLI accepts one positional search string. Search ranks identifier, path, and signature tokens; it does not search source bodies, comments, or docstrings.

`symbols` lists definitions by path and line. Signatures are whitespace-collapsed and capped at 160 characters, so open the source for full declarations and bodies.

`files` reports build-time file inventory and counts. `api` applies language-specific export heuristics to functions, methods, classes, traits, and interfaces; it is not a full public-API audit.

For inheritance questions, query a class, trait, or interface name discovered in your own index with `kg implementers`. It follows resolved inheritance relationships, not arbitrary runtime implementations.

## Select test candidates, not a complete test suite

```sh
kg tests src/hash.rs --depth 10
```

This query selects statically reachable functions named `test_*` or `test` followed by an uppercase letter. It does not execute tests or use the test runner's discovery mechanism.

Rust `#[test]` functions with other names, TypeScript `describe`/`it` callbacks, fixtures, and dynamic relationships can be missed. Read candidate tests, inspect nearby tests, and use the project's test runner before declaring coverage sufficient.

## Investigate cleanup candidates

```sh
kg dead --path src --exclude-tests --exclude-generated
kg clones --path src --min-lines 10 --exclude-generated
kg complexity --path src --min-cyc 11
```

- `dead` reports definitions without indexed inbound calls, imports, or inheritance references after its policy filters. Entry points and external consumers can still use them.
- `clones` reports normalized-text matches, not semantic duplicates. Inspect both regions before extracting shared code.
- `complexity` ranks callable syntax metrics. It is not cognitive complexity or a proof that a function needs refactoring.

Do not delete or refactor code solely because it appears here. Confirm runtime entry points, visibility, framework discovery, and existing tests first.

## Read repository summaries

```sh
kg stats
kg hotspots --path src --limit 10
kg report --path src
```

`stats` describes the index, not current runtime behavior. `report` combines totals, a package map, hotspots, and most-called symbols.

`hotspots` combines file size, a file-level decision-count proxy, recent change counts, and fix-related commit counts. Without usable Git history, change and fix signals become zero; scores are not defect probabilities.

## Recover from uncertain results

| Result | Meaning | Next action |
|---|---|---|
| Name is ambiguous; exit 2 | Multiple indexed definitions match | Copy a full printed qualified name; do not merge candidates silently |
| Symbol or path is missing; exit 1 | The index lacks that target | Check spelling, root, supported extensions, ignore rules, and rebuild |
| No callers, tests, or route; exit 1 | No matching indexed relationship | Search source and inspect dynamic or external relationships |
| `showing N of M` | Output is capped | Narrow the scope or raise `--limit`; do not treat N as the total |
| Changed-result warning | Displayed source differs from the index | Run `kg build`, then repeat the query |
| No freshness warning | Checked files did not trigger a warning | Rebuild after edits anyway; omitted or new files may not be checked |

Find extraction and resolution gaps before trusting an empty result:

```sh
kg doctor
kg unresolved --path src
kg unresolved --names --path src
git grep -n fnv1a64 -- src tests
```

An `[internal]` unresolved call name also exists in the index and can indicate a resolver gap. External and standard-library calls also contribute to unresolved counts; the unresolved rate is not a whole-program missed-call rate.

## Compare workflows fairly

Two comparisons answer different questions:

- **No kg:** use source reading, grep, editor navigation, and the normal test runner without any kg commands.
- **Feature disabled:** keep other kg commands available, but omit the feature under investigation. For example, compose `symbols`, `callers`, and `callees` instead of `context`.

A feature-disabled result does not establish the benefit of kg over no kg. A reduction in tool calls does not establish lower tokens, latency, cost, or production error rates.

This guide makes no measured savings claim. See [Evaluation](EVALUATION.md) for the scope and limitations of reported measurements.

## Keep local usage data under control

Most commands append local usage metadata; pointer queries can also record displayed pointers. Use `kg --no-journal` or `KG_NO_JOURNAL=1` to disable both journals for an invocation.

`kg usage` summarizes recorded activity. `kg usage --verify` checks a sample of recorded pointers, not the completeness of the graph; see [journal controls](cli-reference.md#journal-and-environment).
