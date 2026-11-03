---
name: kg-first
description: Routing policy for code agents with the kg code index available. Use when locating callers, callees, impact of a change, symbol definitions, file structure, or the call route between two symbols. Query kg first; use source reads and grep to verify results or investigate gaps, ambiguity, result caps, and staleness.
---

# kg-first navigation

kg indexes supported source files' symbols and static relationships into `.kg/index.db`.
Results describe the indexed, resolved subset, not every runtime relationship.
See [Evaluation](../../docs/EVALUATION.md) for measurement scope and limitations,
and [Agent integration](../../docs/agent-integration.md) for setup.

## Routing (binding)

For these questions, **query kg first** for navigation pointers. Read the pointed-to
source before making behavioral claims or edits; the index is not a substitute for source evidence.

| question | command |
|----------|---------|
| who calls this symbol | `kg callers <name>` — lines are `path:line qualname` (the call-site identity) |
| what does this call (transitive) | `kg callees <name> --depth N` |
| does A reach B, via which call sites | `kg path <from> <to>` — one shortest route; reverse question = swap the arguments |
| what surrounds this symbol (callers + callees + signature) | `kg context <name>` — one call replaces composing callers/callees/symbols |
| who inherits/implements a type | `kg implementers <name>` — resolved `inherits` edges only; cross-file impls omitted |
| which tests reach this symbol/file | `kg tests <name|file>` — static call-edge candidates only |
| what does this file import | `kg imports <file>` — outbound resolved imports |
| which files depend on a file | `kg impact <file>` lists files that call into it; add `--edges calls,imports,inherits --depth N` for the resolved dependency closure |
| which functions exceed complexity/length | `kg complexity --min-cyc 11` — cyclomatic/branches/nesting/effective LOC, not cognitive complexity |
| what is in this directory | `kg symbols <dir> --outline`; `kg files --path <dir>` for the file inventory |
| find a symbol by (partial) name, qualified path, or signature vocabulary | `kg search <terms>` — qualified input matches the exact phrase; exact names rank first; plurals fall back to stems |
| resolve many names at once | `kg resolve --stdin < names.txt` — one `found`/`ambiguous`/`missing` line per name |
| what is in this file / its signatures | `kg symbols <file> [--outline]` |
| is this logic duplicated elsewhere | `kg clones [--min-lines N]` — candidates only, never proof; spans name the enclosing symbol per side |
| what is risky / unresolved | `kg hotspots`, `kg unresolved`; `kg unresolved --names` lists sites; `[internal]` marks names that exist in the index (a resolver gap) |

Avoid repeating a complete lookup with grep when a current, unambiguous kg result
already answers it. Use source reads or grep for focused verification, missing
relationships, unsupported syntax, or conflicting evidence. Verification required
for correctness, safety, or review takes precedence over avoiding duplicate navigation.

## Handling the contract behaviors

1. **Exit 2 (ambiguous name)**: a bare name matched several symbols. kg prints
   `path:line qualname` candidates. Re-query with the fully qualified name
   (`pkg.mod.symbol` or `Class.method`) — never guess which one you meant.
   Grepping the bare name instead silently mixes all of them.
2. **`showing N of M` on the last line**: when N < M, the result was capped.
   Most list commands default to 50; `context` defaults to 10 per section and allows 50.
   Re-run with `--limit <M>` where supported (usually max 10000).
   If the cap still excludes results, narrow supported path filters or inspect source; do not claim a complete set.
3. **`kg path` outcomes**: endpoints accept the exact bare name or the full
   qualname only — `symbol '<name>' not in index` means the input form was
   wrong or the symbol is absent; recover with `kg search` or
   `kg symbols <file>`. Exit 2 names the failing endpoint (`as the from|to
   endpoint`) — resolve it the same way, then re-query.
   `'q1' and 'q2' resolve to the same symbol` means both arguments landed on
   one symbol. `no call path ... at any depth` is a definitive negative
   within resolved edges; `within depth D; a longer path may exist` means
   the cap stopped the search (max 10 hops). A K-hop route fails if ANY hop
   edge is unresolved. No path in the resolved graph does not prove no runtime
   connection. Inspect `callees`/`callers` and source or grep for missing hops
   instead of repeating an unchanged `path` query.
4. **Static-surface caveats**: `implementers`, `imports`, `tests`, `api`, and
   `complexity` are static. Successful evaluation cases do not establish complete
   repository coverage; a negative is not proof. `tests` misses Rust `#[test]` names outside `test_*`, TS
   `describe`/`it` callbacks, and function-local imports. `dead` is a
   candidate list, not proof.

## Staleness discipline

After you edit code, run `kg build` before further navigation queries.
If a query prints
`kg: K of N result files changed since build — run kg build`, rebuild before
trusting those pointers. Staleness checks cover displayed result files, not every
file that could change the answer. No warning does not prove the index is current.

## When kg is NOT the tool

- Semantic questions ("where is behavior X implemented") — kg resolves names,
  not meanings. Grep/read.
- Dynamic dispatch, untyped attribute calls, external-library calls — kg may
  leave these unresolved. Unresolved sites can also indicate resolver gaps.
  Follow `grep fallback recommended` with source inspection; do not treat an
  unresolved-site share as a measure of all supported behavior or query recall.
- Questions about code in another repository — the index is per-repo.

## Cheap context hygiene

- Pointers lead with `path:line`; a trailing identity names the call site —
  open only the lines you need (`sed -n`), not whole files, when the pointer
  answers the question.
- `kg symbols <file> --outline` replaces most "read the file to see its
  structure" reads.
- `kg stats` / `kg report` answer repo-shape questions (totals, top packages,
  fan-in, hotspots) in one call — do not assemble them from grep.
