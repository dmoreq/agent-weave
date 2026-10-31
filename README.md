# kg — local code knowledge index

Status: ALL PHASES COMPLETE — P0/P1/P1.5/P2/P2.5 done, P3 skipped (trigger not met). Gates G1-G8 pass, G2 on Python + TS/TSX + Rust self-index.
Spec (canonical, in-repo, v1.1 + Amendments 1-3): [docs/designs/2026-10-31_kg-rust-code-index.md](docs/designs/2026-10-31_kg-rust-code-index.md).

kg indexes a repository's symbols and call/inheritance edges into a local
SQLite DB and answers three navigation questions:

- **who calls this** — `kg callers <symbol>`
- **what does this call** — `kg callees <symbol>` (`--depth N` for the transitive closure)
- **what is impacted by this file** — `kg impact <file>` (files that call into `<file>`)

Consultative only: no hooks, no prompts, no daemon, no network, no writes
outside `.kg/`. One static binary, five pinned dependencies, zero unsafe Rust.

## Install

```
cargo install --path .
```

## Usage

```
kg build                      # index the repo into .kg/index.db (atomic swap)
kg build --force              # re-extract everything (skip the fast path)
kg callers Widget.render      # pointer list: path:line, capped at 50
kg callees parse --depth 3    # transitive call closure
kg impact app.py              # files that call into app.py
kg dead                       # dead-code candidates (caveat: not proof)
kg unresolved                 # per-file unresolved-callee counts
kg symbols app.py             # one file's symbols, ordered by line
kg stats                      # index totals
kg usage                      # usage-journal summary: calls, latency, rates
kg hotspots --limit 10        # pain-ranking: cyclomatic + LOC + churn-90d + fix-density
kg report                     # 4-section architecture report
kg doctor [--invalidate]      # integrity, versions, staleness, orphans
```

`--root <path>` overrides repo discovery (default: nearest git root ancestor).
`--no-journal` (or env `KG_NO_JOURNAL=1`) disables the usage journal
(`.kg/usage.log`, local-only JSONL — see spec Amendment 6).
Exit codes: 0 ok; 1 no results / no index; 2 usage or ambiguous name (a
candidate list is printed); 3 corrupt index; 4 locked; 5 schema mismatch;
6 internal; 7 parse-quality abort.

## Response contract

- Pointers: `repo-relative/path:line [ambiguous]`, 1-based lines, capped at 50
  (`--limit N`); final stdout line `showing N of M`; stderr cap note.
- Staleness: post-cap displayed set only; stat short-circuit then FNV-1a
  re-hash; header `kg: K of N result files changed since build — run kg build`.
  Never auto-rebuilds.
- Empty result: exit 1 with a reason line (symbol not in index / path not
  indexed / 0 callers — dynamic dispatch possible, grep fallback recommended).

## Safety invariants (CI-enforced)

1. Zero network (Seatbelt + seccomp CI jobs, Cargo.lock allowlist check).
2. No long-lived child processes.
3. Read-only over source; writes confined to `.kg/` (tree-snapshot test).
4. No behavior hijack: no hooks, settings, or agent-config writes.

## Gates (measured, vihap ~102k LOC — re-baselined M0 2026-10-31, see docs/baselines/rebaseline-m0.md)

| gate | measured | target |
|------|----------|--------|
| G1 build | 0.47s wall, 22.8MB RSS, 1.98MB DB | < 5s / < 100MB / < 50MB |
| G2 precision/recall | vihap 100%/100% (493/495); finguard 100%/100%; self 90%/100% | >= 80% / >= 70% |
| G3 query p95 | 9.8ms | < 20ms |
| G6 utility A/B | -50% tool calls, -67% bytes vs grep (P1 measurement; M0 re-run pending) | >= 30% calls, no byte growth |
| G7 hotspots | 8/10 rank-overlap (provisional pain list) | >= 7/10 |
| G8 extensions A/B | -67% calls on architecture questions (P1.5 measurement; M0 re-run pending) | >= 30% |

Baseline evidence: `docs/baselines/` (tasks, ground truth, transcripts, D0
harness). Rollback: `rm -rf .kg/` (or `kg doctor --invalidate`).

## Project docs

| doc | contents |
|-----|----------|
| [docs/designs/2026-10-31_kg-rust-code-index.md](docs/designs/2026-10-31_kg-rust-code-index.md) | THE SPEC — normative v1.1 + Amendments 1-3 (gates, algorithm, schema, contract) |
| [docs/designs/2026-10-31_concepts-brief_kg-rust-code-index.md](docs/designs/2026-10-31_concepts-brief_kg-rust-code-index.md) | concepts brief that preceded the spec |
| [CHANGELOG.md](CHANGELOG.md) | phase-by-phase history (Keep a Changelog) |
| [CONTRIBUTING.md](CONTRIBUTING.md) | spec-first rules, gate workflow, review protocol, honesty rule |
| [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) | contestable-findings standard |
| [SECURITY.md](SECURITY.md) | invariants, index safety, reporting |
| [SUPPORT.md](SUPPORT.md) | troubleshooting table, scope boundaries |
| [LICENSE](LICENSE) | private / all rights reserved (OQ1: swap before any publish) |
| `.github/` | PR template, bug/feature issue templates, ci.yml |
| `docs/baselines/reviews/` | R0/R1 independent review records (R1.5/R2 fix cycles: see CHANGELOG and commit history) |
