# kg — local code knowledge index

Status: P1 + P1.5 complete — gates G1-G6 PASS, G7 8/10 (provisional pain list), G8 -67% calls. Spec: `agent-skills/docs/designs/2026-10-31_kg-rust-code-index.md` (v1.1 + Amendments 1-2).

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
kg stats                      # index totals
kg hotspots --limit 10        # pain-ranking: cyclomatic + LOC + churn-90d + fix-density
kg report                     # 4-section architecture report
kg doctor [--invalidate]      # integrity, versions, staleness, orphans
```

`--root <path>` overrides repo discovery (default: nearest git root ancestor).
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

## Gates (measured, vihap ~102k LOC)

| gate | measured | target |
|------|----------|--------|
| G1 build | 1.91s wall, 21.4MB RSS, 1.9MB DB | < 5s / < 100MB / < 50MB |
| G2 precision/recall | 100% / 100% on 29 scored GT sites | >= 80% / >= 70% |
| G3 query p95 | 10.1ms | < 20ms |
| G6 utility A/B | -50% tool calls, -67% bytes vs grep | >= 30% calls, no byte growth |
| G7 hotspots | 8/10 rank-overlap (provisional pain list) | >= 7/10 |
| G8 extensions A/B | -67% calls on architecture questions (via callees --depth) | >= 30% |

Baseline evidence: `docs/baselines/` (tasks, ground truth, transcripts, D0
harness). Rollback: `rm -rf .kg/` (or `kg doctor --invalidate`).
