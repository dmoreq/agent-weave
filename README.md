# kg — local code knowledge index

[![CI](https://github.com/quy-doan/kg/actions/workflows/ci.yml/badge.svg)](https://github.com/quy-doan/kg/actions/workflows/ci.yml)

kg indexes a repository's symbols and call/inheritance edges into a local
SQLite DB and answers navigation questions a code agent asks dozens of times
per session:

- **who calls this** — `kg callers <symbol>`
- **what does this call** — `kg callees <symbol>` (`--depth N` transitive closure)
- **how A reaches B** — `kg path <from> <to>` (shortest call route: hop call sites + chain line)
- **what surrounds this symbol** — `kg context <symbol>` (definition + signature, direct callers, direct callees in one call)
- **what is impacted by this file** — `kg impact <file>`
- **find a symbol by name** — `kg search <terms>` (BM25 over identifier, path, and signature tokens)
- **file structure** — `kg symbols <file> --outline` (one-line signatures)

Consultative only: no hooks, no daemon, no network, no writes outside `.kg/`.
One binary with no daemon and no network at runtime, nine direct dependencies, zero unsafe Rust.

Measured results (see [docs/EVALUATION.md](docs/EVALUATION.md) for methodology,
reproduction commands, and threats to validity):

| question | measured |
|----------|----------|
| Does it point at real code? | 777/777 emitted pointers verified against source lines; ground-truth recall 100% on two GT sets plus the refreshed self set; call-site precision 662/664 (99.7%; two known checker/GT failures listed); P8 real-corpus GT 77/77 (imports 40/40, tests 20/20, inheritance 17/17) |
| Is it fast enough for an agent loop? | queries 5-11 ms best-of-7 end-to-end (self-index; < 10 ms on a 382 MB index); builds 0.06-0.46 s on 27-596-file repos and 2.70-2.79 s best-of-7 on the 2,025-file synthetic corpus (1.25x budget: 2.91 s) |
| Does it help a coding agent? | navigation recall 98.6% vs 81.9% grep-only; with query-first routing: **-27% input tokens** at equal recall (40 primary instrumented runs + 2 alternate fills; Grok Build, n=10/arm) |
| Structure and duplication lookups? | `kg search` recall 0.775 (22/28) on frozen NL queries; `symbols --outline` cuts battery tool calls -41.7% vs the grep baseline (+1 call vs the plain-kg control); `kg clones` detects 10/10 seeded pairs with 0 spurious (candidates, not proof) |
| Does everything help every host? | **No.** Route queries (`path`) cut tool calls 57-59% on Grok and 61-86% on OpenCode (Stage 1 and Stage 4); `search` showed no economics gain in its 24-run battery; `clones` transferred on Grok (−59% calls) but not OpenCode (−17%). Utility is host-dependent — see EVALUATION §5. |

## Install

```
cargo install --path . --locked
```

Requires Rust 1.90+ and a C compiler (bundled SQLite and
tree-sitter grammars). Not yet on crates.io; install from a clone. Supported:
macOS arm64/x86_64 and Linux x86_64/aarch64 (CI-built).

## Usage

```
kg build                      # index the repo into .kg/index.db (atomic swap; unchanged trees leave index.db byte-identical)
kg build --force              # re-extract everything (skip the fast path)
kg callers Widget.render      # pointer list: path:line, capped at 50
kg callees parse --depth 3    # transitive call closure
kg path Widget.render parse_fragment  # shortest call route (resolved edges; depth cap 10)
kg context Widget.render       # definition + callers + callees in one call
kg implementers Base          # classes/interfaces/traits that inherit or implement Base
kg imports app.py             # outbound resolved imports
kg tests Widget.render       # test candidates that reach the symbol/file (static)
kg impact app.py              # files that call into app.py (one row per file)
kg impact app.py --edges calls,imports,inherits --depth 3  # dependency closure
kg search parser fragment     # BM25 symbol search (identifier/path/signature tokens)
kg clones                     # duplicate-block candidates with enclosing symbol per side (caveat: not proof)
kg dead --exclude-tests       # dead-code candidates (caveat: not proof)
kg unresolved                 # per-file counts + top unresolved names; [internal] = resolver gap
kg unresolved --names         # unresolved edge sites: path:line kind name
kg symbols app.py             # one file's symbols, ordered by line
kg symbols src/ --outline     # directory inventory with one-line signatures
kg symbols app.py --metrics   # per-symbol cyclomatic/branches/nesting/effective LOC
kg complexity --min-cyc 11    # functions over the complexity ceiling
kg files --path src           # indexed file inventory (symbols, bytes, generated)
kg resolve --stdin < names.txt  # batch name resolution: found/ambiguous/missing
kg stats                      # index totals (includes generated count)
kg usage                      # usage-journal summary: calls, latency, rates
kg usage --verify             # replay-verify sampled payload pointers: OK/CHANGED/FAIL
kg mcp                        # MCP stdio server, JSON-RPC 2.0
kg hotspots --limit 10        # pain-ranking: cyclomatic + LOC + churn-90d + fix-density
kg report                     # 4-section architecture report (freshness caveat when stale)
kg doctor [--invalidate]      # integrity, versions, staleness, orphans
```

`--root <path>` overrides repo discovery (default: nearest git root ancestor).
`--no-journal` (or env `KG_NO_JOURNAL=1`) disables the usage journal
(`.kg/usage.log`, local-only JSONL). Every successful query that displays pointers also
appends one self-contained payload line to `.kg/usage-results.jsonl`
(`files`, `resolve`, and `unresolved` are journal-only by contract):
displayed pointers byte-identical to stdout, plus index `built_at`/`source_rev`
and `KG_SESSION`/`KG_ARM` attribution. `kg usage --verify` replays a
deterministic sample of those pointers against current source.
The index `.kg/index.db` is a portable artifact: cache it in CI and restore it
into a fresh checkout — the dual-key fast path reuses it without re-parse when
content hashes match.

Exit codes: 0 ok; 1 no results / no index; 2 usage or ambiguous name (a
candidate list is printed); 3 corrupt index; 4 locked; 5 schema mismatch;
6 internal; 7 parse-quality abort.

## Agent integration

- **MCP**: `kg mcp` serves every query subcommand except `doctor` as
  JSON-RPC tools over stdio; successful tool text matches CLI stdout, and
  error text appends the CLI stderr detail (conformance-tested).
- **Skill**: [`skills/kg-first/SKILL.md`](skills/kg-first/SKILL.md) is the
  measured routing policy for code agents — when to query kg first, how to
  handle ambiguous names and result caps, and when to fall back to grep.
  Stored here; installation into a host is the owner's call.

## Response contract

- Pointers: `repo-relative/path:line`, 1-based lines, capped at 50
  (`--limit N`); an optional `[ambiguous]` tag and, on callers/callees/context,
  the call-site identity (`path:line qualname`). Final stdout line
  `showing N of M`; stderr cap note. `impact` rows are per file with
  `(N sites)` unless `--sites`; `clones` spans name the enclosing symbol.
- Staleness: post-cap displayed set only; stat short-circuit then FNV-1a
  re-hash; header `kg: K of N result files changed since build — run kg build`.
  Never auto-rebuilds.
- Empty result: exit 1 with a reason line (symbol not in index / path not
  indexed / 0 callers — dynamic dispatch possible, grep fallback recommended).

## Safety invariants (CI-enforced)

1. Zero network (Seatbelt (macOS) + network-namespace (Linux) CI jobs with negative controls, Cargo.lock allowlist check).
2. No long-lived child processes.
3. Read-only over source; writes confined to `.kg/` (tree-snapshot test).
4. No behavior hijack: no hooks, settings, or agent-config writes.

## Limitations

- Resolution is static: static analysis covers 10-48% of raw call sites
  (unresolved 51.8-90.4% across six measured corpora; Django 71.7%, vscode
  62.9%). These are raw unresolved call names and include external/library
  calls; `kg unresolved` marks names that exist in the index (`[internal]`)
  as actionable resolver gaps. Dynamic dispatch and untyped attribute calls
  are unresolved by design; the tool reports the gap per file and recommends
  grep.
- The index is per-repo and local; cross-repo questions are out of scope.
- Staleness is detected, never auto-repaired: run `kg build` after edits
  (0.06-0.46 s on 27-596-file repos; 0.02-0.03 s no-op at 2,201 files —
  unchanged builds leave `.kg/index.db` byte-identical).

## Project docs

| doc | contents |
|-----|----------|
| [docs/EVALUATION.md](docs/EVALUATION.md) | **measured results** — perf, multi-repo verification, correctness gates, agent A/B, limitations |
| [docs/designs/2026-10-31_kg-rust-code-index.md](docs/designs/2026-10-31_kg-rust-code-index.md) | THE SPEC — normative v1.1 + Amendments (gates, algorithm, schema, contract) |
| [docs/designs/2026-10-31_concepts-brief_kg-rust-code-index.md](docs/designs/2026-10-31_concepts-brief_kg-rust-code-index.md) | concepts brief that preceded the spec |
| [CHANGELOG.md](CHANGELOG.md) | phase-by-phase history (Keep a Changelog) |
| [CONTRIBUTING.md](CONTRIBUTING.md) | spec-first rules, gate workflow, review protocol, honesty rule |
| [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) | contestable-findings standard |
| [SECURITY.md](SECURITY.md) | invariants, index safety, reporting |
| [SUPPORT.md](SUPPORT.md) | troubleshooting table, scope boundaries |
| [skills/](skills/README.md) | kg-first agent skill (in-repo; host installation is owner-decided) |
| [LICENSE](LICENSE) | dual MIT / Apache-2.0 (LICENSE-MIT, LICENSE-APACHE) |
| `.github/` | PR template, bug/feature issue templates, ci.yml |
| `docs/baselines/` | evidence layer: frozen tasks, ground truth, transcripts, harness, review records (internal; uses real corpus-repo names) |
