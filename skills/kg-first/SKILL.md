---
name: kg-first
description: Routing policy for code agents with the kg code index available. Use when locating callers, callees, impact of a change, symbol definitions, file structure, or the call route between two symbols — query kg first and fall back to grep only for what it cannot answer. Encodes the measured economics (-27% input tokens at equal recall) and the failure modes (ambiguous bare names, result caps, staleness) from 40 primary instrumented agent runs.
---

# kg-first navigation

kg indexes the repo's symbols and call edges into `.kg/index.db`. Every claim
in this skill is measured; see `docs/EVALUATION.md` in the kg repository.

## Routing (binding)

For these questions, **query kg first** and treat its pointers as the primary
source:

| question | command |
|----------|---------|
| who calls this symbol | `kg callers <name>` — lines are `path:line qualname` (the call-site identity) |
| what does this call (transitive) | `kg callees <name> --depth N` |
| does A reach B, via which call sites | `kg path <from> <to>` — one shortest route; reverse question = swap the arguments |
| what surrounds this symbol (callers + callees + signature) | `kg context <name>` — one call replaces composing callers/callees/symbols |
| which files call into a file | `kg impact <file>` |
| find a symbol by (partial) name, qualified path, or signature vocabulary | `kg search <terms>` — qualified input matches the exact phrase; exact names rank first; plurals fall back to stems |
| what is in this file / its signatures | `kg symbols <file> [--outline]` |
| is this logic duplicated elsewhere | `kg clones [--min-lines N]` — candidates only, never proof; spans name the enclosing symbol per side |
| what is risky / dead / unresolved | `kg hotspots`, `kg dead`, `kg unresolved` — `[internal]` marks names that exist in the index (a resolver gap), other names are external/library calls |

Use grep/rg **only** for what kg cannot answer or to verify one specific line.
Do not re-derive with grep what kg has already answered — that doubles the
context you carry for zero information gain.

## Handling the contract behaviors

1. **Exit 2 (ambiguous name)**: a bare name matched several symbols. kg prints
   `path:line qualname` candidates. Re-query with the fully qualified name
   (`pkg.mod.symbol` or `Class.method`) — never guess which one you meant.
   Grepping the bare name instead silently mixes all of them.
2. **`showing N of M` on the last line**: the result was capped (default 50).
   Hub symbols have hundreds of call sites. Re-run with `--limit <M>` (max
   10000) when you need the full set; paginate otherwise.
3. **`kg path` outcomes**: endpoints accept the exact bare name or the full
   qualname only — `symbol '<name>' not in index` means the input form was
   wrong or the symbol is absent; recover with `kg search` or
   `kg symbols <file>`. Exit 2 names the failing endpoint (`as the from|to
   endpoint`) — resolve it the same way, then re-query.
   `'q1' and 'q2' resolve to the same symbol` means both arguments landed on
   one symbol. `no call path ... at any depth` is a definitive negative
   within resolved edges; `within depth D; a longer path may exist` means
   the cap stopped the search (max 10 hops). A K-hop route fails if ANY hop
   edge is unresolved, so repeated no-path results likely reflect the
   52-90% unresolved call-site share (10-48% resolution coverage), not a missing connection — fall back to
   composing `callees`/`callers` or grep instead of re-querying `path`.

## Staleness discipline

After you edit code, run `kg build` before further navigation queries
(0.1-0.5 s typical; an unchanged tree is 0.02-0.03 s at ~2k files). If a query prints
`kg: K of N result files changed since build — run kg build`, rebuild before
trusting those pointers.

## When kg is NOT the tool

- Semantic questions ("where is behavior X implemented") — kg resolves names,
  not meanings. Grep/read.
- Dynamic dispatch, untyped attribute calls, external-library calls — kg
  reports them as unresolved by design (static coverage 10-48% of raw call
  sites; unresolved 52-90% depending on codebase, external calls included)
  and says `grep fallback recommended`. Believe it.
- Questions about code in another repository — the index is per-repo.

## Cheap context hygiene

- Pointers lead with `path:line`; a trailing identity names the call site —
  open only the lines you need (`sed -n`), not whole files, when the pointer
  answers the question.
- `kg symbols <file> --outline` replaces most "read the file to see its
  structure" reads.
- `kg stats` / `kg report` answer repo-shape questions (totals, top packages,
  fan-in, hotspots) in one call — do not assemble them from grep.
