---
name: aw-first
description: Routing policy for code agents with the aw code index available. Use when locating callers, callees, the impact of a change, the call route between two symbols, a symbol's surroundings, or a file's imports. Query aw first for those questions; use source reads and grep to verify results or investigate gaps, ambiguity, result caps, and staleness.
---

# aw-first navigation

aw indexes supported source files' symbols and static relationships into `.aw/index.db`.
Results describe the indexed, resolved subset, not every runtime relationship.
See [Evaluation](../../docs/EVALUATION.md) for measurement scope and limitations,
and [Agent integration](../../docs/agent-integration.md) for setup.

## Routing (binding)

For these questions, **query aw first** for navigation pointers. Read the pointed-to
source before making behavioral claims or edits; the index is not a substitute for source evidence.

| question | command |
|----------|---------|
| is there a current index | `aw build` — rebuild after source edits before trusting pointers |
| who calls this symbol | `aw callers <name>` — lines are `path:line qualname`; add `--depth N` for the transitive reverse call closure, tagged `[depth=N]` |
| what does this call (transitive) | `aw callees <name> --depth N` |
| does A reach B, via which call sites | `aw path <from> <to>` — one shortest route; reverse question = swap the arguments |
| what surrounds this symbol (callers + callees + signature) | `aw context <name>` — one call replaces composing callers/callees by hand |
| which files depend on a file | `aw impact <file>` lists files that call into it; add `--edges calls,imports,inherits --depth N` for the resolved dependency closure |
| what does this file import | `aw imports <file>` — outbound resolved imports |

These seven commands are the measured, validated routing set. Other aw commands
exist, but they are not mandatory agent routing; use them only as described below.

Avoid repeating a complete lookup with grep when a current, unambiguous aw result
already answers it. Use source reads or grep for focused verification, missing
relationships, unsupported syntax, or conflicting evidence. Verification required
for correctness, safety, or review takes precedence over avoiding duplicate navigation.

## Trust Contract

When an `aw` command exits 0, accept its output as ground truth for the fields
it returns. Do NOT re-verify with grep, read, or a second `aw` call unless:

- The output is explicitly marked as "candidates only" (`aw tests`, `aw dead`), OR
- The question asks for something not in the output fields.

Commands where output IS ground truth (do not re-verify):
`callers`, `callees`, `context`, `implementers`, `imports`, `impact`, `path`, `symbols`

Commands where output IS candidates (may require source read to filter):
`tests`, `dead`, `unresolved`

This contract covers navigation answers only. Source reads remain required for
behavioral claims, edits, safety, and review.

## Secondary tools (optional)

Reach for these when the seven routed commands do not answer the question. They are
diagnostic or inventory surfaces: their precision and utility vary by repository, so
verify their output before acting on it.

| question | command |
|----------|---------|
| what is in this file / its signatures | `aw symbols <file> [--outline]` — deterministic inventory of one indexed file |
| find a symbol by (partial) name or signature vocabulary | `aw search <terms>` — token search over names, paths, and signatures; exact names rank first |
| who inherits/implements a type | `aw implementers <name>` — resolved `inherits` edges only; cross-file impls omitted |
| which tests reach this symbol/file | `aw tests <name|file>` — static call-edge candidates only, never a test plan |
| what is in this directory | `aw symbols <dir> --outline`; `aw files --path <dir>` for the file inventory |
| resolve many names at once | `aw resolve --stdin < names.txt` — one `found`/`ambiguous`/`missing` line per name |
| is this logic duplicated elsewhere | `aw clones [--min-lines N]` — candidates only, never proof |
| which functions exceed complexity/length | `aw complexity --min-cyc 11` — syntax proxies, not cognitive complexity |
| what is risky / unresolved | `aw hotspots`, `aw unresolved`; `aw unresolved --names` lists sites; `[internal]` marks names that exist in the index (a resolver gap) |
| repository totals and structure | `aw stats`, `aw report` — build-time summaries, not freshness checks |
| dead-code candidates | `aw dead` — candidate list only, never proof of deadness |

## Handling the contract behaviors

1. **Exit 2 (ambiguous name)**: a bare name matched several symbols. aw prints
   `path:line qualname` candidates. Re-query with the fully qualified name
   (`pkg.mod.symbol` or `Class.method`) — never guess which one you meant.
   Grepping the bare name instead silently mixes all of them.
2. **`showing N of M` on the last line**: when N < M, the result was capped.
   Most list commands default to 50; `context` defaults to 10 per section and allows 50.
   Re-run with `--limit <M>` where supported (usually max 10000).
   If the cap still excludes results, narrow supported path filters or inspect source; do not claim a complete set.
3. **`aw path` outcomes**: endpoints accept an exact bare name or full
   qualname; a unique dotted suffix such as `Class.method` also resolves, while
   several suffix matches stay ambiguous — `symbol '<name>' not in index` means
   the input form was wrong or the symbol is absent. Recover with `aw search` or
   `aw symbols <file>`. Exit 2 names the failing endpoint (`as the from|to
   endpoint`) — resolve it the same way, then re-query.
   `'q1' and 'q2' resolve to the same symbol` means both arguments landed on
   one symbol. `no call path ... at any depth` is a definitive negative
   within resolved edges; `within depth D; a longer path may exist` means
   the cap stopped the search (max 10 hops). A K-hop route fails if ANY hop
   edge is unresolved. No path in the resolved graph does not prove no runtime
   connection. Inspect `callees`/`callers` and source or grep for missing hops
   instead of repeating an unchanged `path` query.
4. **Static-surface caveats**: every aw result covers the indexed, resolved
   subset only. Successful cases do not establish complete repository coverage;
   a negative is not proof. Secondary tools inherit the same limit: `tests`
   misses Rust `#[test]` names outside `test_*`, TS `describe`/`it` callbacks,
   and function-local imports; `dead` and `clones` are candidate lists.

## Staleness discipline

After you edit code, run `aw build` before further navigation queries.
If a query prints
`aw: K of N result files changed since build — run aw build`, rebuild before
trusting those pointers. Staleness checks cover displayed result files, not every
file that could change the answer. No warning does not prove the index is current.

## When aw is NOT the tool

- Semantic questions ("where is behavior X implemented") — aw resolves names,
  not meanings. Grep/read.
- Dynamic dispatch, untyped attribute calls, external-library calls — aw may
  leave these unresolved. Unresolved sites can also indicate resolver gaps.
  Follow `grep fallback recommended` with source inspection; do not treat an
  unresolved-site share as a measure of all supported behavior or query recall.
- Questions about code in another repository — the index is per-repo.

## Cheap context hygiene

- Pointers lead with `path:line`; a trailing identity names the call site —
  open only the lines you need (`sed -n`), not whole files, when the pointer
  answers the question.
- `aw symbols <file> --outline` replaces most "read the file to see its
  structure" reads when you only need the definition inventory.
