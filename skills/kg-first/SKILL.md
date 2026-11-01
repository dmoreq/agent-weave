---
name: kg-first
description: Routing policy for code agents with the kg code index available. Use when locating callers, callees, impact of a change, symbol definitions, or file structure — query kg first and fall back to grep only for what it cannot answer. Encodes the measured economics (-27% input tokens at equal recall) and the failure modes (ambiguous bare names, result caps, staleness) from 42 instrumented agent runs.
---

# kg-first navigation

kg indexes the repo's symbols and call edges into `.kg/index.db`. Every claim
in this skill is measured; see `docs/EVALUATION.md` in the kg repository.

## Routing (binding)

For these questions, **query kg first** and treat its pointers as the primary
source:

| question | command |
|----------|---------|
| who calls this symbol | `kg callers <name>` |
| what does this call (transitive) | `kg callees <name> --depth N` |
| which files call into a file | `kg impact <file>` |
| find a symbol by (partial) name | `kg search <terms>` |
| what is in this file / its signatures | `kg symbols <file> [--outline]` |
| is this logic duplicated elsewhere | `kg clones [--min-lines N]` — candidates only, never proof of semantic duplication |
| what is risky / dead / unresolved | `kg hotspots`, `kg dead`, `kg unresolved` |

Use grep/rg **only** for what kg cannot answer or to verify one specific line.
Do not re-derive with grep what kg has already answered — that doubles the
context you carry for zero information gain.

## Handling the two contract behaviors

1. **Exit 2 (ambiguous name)**: a bare name matched several symbols. kg prints
   `path:line qualname` candidates. Re-query with the fully qualified name
   (`pkg.mod.symbol` or `Class.method`) — never guess which one you meant.
   Grepping the bare name instead silently mixes all of them.
2. **`showing N of M` on the last line**: the result was capped (default 50).
   Hub symbols have hundreds of call sites. Re-run with `--limit <M>` (max
   10000) when you need the full set; paginate otherwise.

## Staleness discipline

After you edit code, run `kg build` before further navigation queries
(0.1-0.5 s typical). If a query prints
`kg: K of N result files changed since build — run kg build`, rebuild before
trusting those pointers.

## When kg is NOT the tool

- Semantic questions ("where is behavior X implemented") — kg resolves names,
  not meanings. Grep/read.
- Dynamic dispatch, untyped attribute calls, external-library calls — kg
  reports them as unresolved by design (10-48% of raw call sites depending on
  codebase) and says `grep fallback recommended`. Believe it.
- Questions about code in another repository — the index is per-repo.

## Cheap context hygiene

- Pointers are `path:line` — open only the lines you need (`sed -n`), not
  whole files, when the pointer answers the question.
- `kg symbols <file> --outline` replaces most "read the file to see its
  structure" reads.
- `kg stats` / `kg report` answer repo-shape questions (totals, top packages,
  fan-in, hotspots) in one call — do not assemble them from grep.
