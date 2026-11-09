# Concepts

`eg` answers questions about code relationships from a local index. It does
not run your program, call a language server, or read a remote service. This
page explains the model behind every command, so a result is easier to
interpret and easier to question.

## The index, not the runtime

A build walks the repository, parses supported source files, and writes
`.engram/index.db`. Queries read that index. If source changes after a build, the
index is stale until you run `eg build` again. `eg` never rebuilds in the
background.

Supported source languages are Python, JavaScript, TypeScript/TSX, and Rust.
Source files larger than 1 MiB are skipped. The walk is Git-visible and
ignore-aware; generated files are tagged during the build.

## Files, symbols, and edges

A **symbol** is a definition: a module, class, function, method, trait, or
interface. Each symbol has a name and a qualified name (`qualname`). A method
is normally addressed as `Class.method`; the CLI resolves a bare name when it
is unique and exits `2` with candidates when it is not.

An **edge** records a static relationship between symbols:

| Kind | Meaning |
|---|---|
| `calls` | The source symbol contains a call site for the destination symbol |
| `imports` | The source file imports the destination module or symbol |
| `inherits` | A class or type inherits from, extends, or implements another |
| `type_ref` | A typed receiver or annotation references a class; used as a resolution hint |
| `contains` | Lexical parent relationship; internal to scoping, not a navigation surface |

An edge has a **confidence**:

- `EXTRACTED`: one destination was resolved from source.
- `AMBIGUOUS`: the source construct could refer to more than one candidate, or
  a star import prevents a unique target. The row is reported as a candidate,
  not as a proven call.

A call site with no resolvable destination is not an edge. It is counted in
`unresolved` and may appear in `eg unresolved`. Builtins and external library
calls are commonly unresolved; they are not evidence that the code is wrong.

## Pointers and output

Most navigation rows are **pointers**: `path:line`. Some rows append a detail:

```text
shop/service.py:9 shop.service.apply_discount
shop/models.py:20 [ambiguous] shop.models.DiscountedOrder.discount
```

- The first token is the source location.
- `[ambiguous]` means the edge is a candidate, not a proven call.
- The trailing text identifies the symbol or relationship.
- `showing N of M` means the display cap truncated the result; `M` is the
  indexed total, not proof that every row is reachable at runtime.
- A header such as `eg: 3 of 12 result files changed since build` means the
  displayed files changed on disk after the index was built. Rebuild before
  relying on those rows.

`eg context` prints a definition range, a caller section, and a callee section.
`eg path` prints hop call sites and a final `eg: chain ...` line. `eg impact`
groups rows by caller file by default; `--sites` lists each site.

`--explain` on `callers`, `callees`, or `context` adds the metadata the index
stores for each edge: kind, confidence, site line, destination, grouped edge
count, and a typed-receiver hint when a `type_ref` edge exists. It does not
reconstruct the resolver branch: import chains, `self`/MRO choices, and
ambiguous candidate sets are not persisted. On transitive rows, the metadata
describes one traversed edge or a path projection, not a complete resolution
trace.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success with a result |
| `1` | Empty result, missing target, or missing index |
| `2` | Invalid usage or an ambiguous bare name |
| `3` | Corrupt index |
| `4` | Another build holds the lock |
| `5` | Schema version mismatch; rebuild with `eg build` |
| `6` | Internal or I/O error |
| `7` | Parse-error rate exceeded the build threshold |

Exit `1` is not proof that no relationship exists. It means the index has no
result for that query shape, or the target is absent. Check `unresolved`,
staleness, supported languages, and display limits before concluding absence.

## Trust contract

Navigation output is ground truth for the fields it returns: a pointer names an
indexed file and line, and the detail identifies the symbol or relationship the
index recorded. It is not a complete model of runtime behavior.

These surfaces are **candidates**, not proof:

- `dead`: symbols without indexed inbound references; it does not prove deadness.
- `unresolved`: locations where the static resolver failed; it does not prove
  that a call is external or dynamic.
- `complexity`: a static effective-line-count ranking, not runtime cost.

Source reads still gate edits and behavioral claims.

## Static boundaries

- Dynamic dispatch, reflection, untyped receivers, and external libraries can
  hide callers, callees, paths, and dependents.
- One index covers one repository. `eg` does not answer cross-repository
  questions or search by the meaning of arbitrary behavior descriptions.
- Search covers indexed identifiers, paths, and signatures, not file bodies,
  comments, or docstrings.
- An index can be stale. `eg` never rebuilds automatically, and `eg doctor`
  reports health rather than repairing behavior.
- A static result is a starting point for inspection. It is not a test result,
  a runtime trace, or a guarantee of complete coverage.

## Local data

The index, usage journal, and lock files live under `.engram/`. The journal records
command names, arguments, result counts, exit codes, and latency; it does not
record file contents. Pass `--no-journal` or set `ENGRAM_NO_JOURNAL=1` to disable it.
`eg` makes no network requests and installs no hooks or background processes.

## Theoretical Foundations & Agent Failure Modes

`eg` is architected as an empirical countermeasure to failure modes documented in AI agent research:

### 1. Exploration Collapse & The Historical Memory Paradox (RepoReuse)
In [RepoReuse](https://arxiv.org/abs/2609.35357) (*Ma et al., 2026*), empirical auditing of 3,000 multi-turn agent sessions revealed two structural breakdowns:
- **Exploration Gap vs. Execution Gap**: As task turns progress, an agent's repository exploration recall ($\text{recall}_{\text{repo}}$) collapses from 83.6% down to 35.4%. Even when agents inspect their own previous code ($\text{recall}_{\text{self}} \approx 99.6\%$), actual reuse ($\text{reuse}_{\text{self}}$) still declines from 83.9% to 69.1%, resulting in duplicate code in **50.8% of multi-turn task chains** by Turn 5.
- **The Historical Memory Paradox**: Feeding raw source code into prompts (**Source Memory**) severely exacerbates duplication: $C_{dup}$ reaches **70.7%** (and **94.7%** at Turn 5) because models copy-paste and alter code in-context rather than importing existing modules. In contrast, providing **Interface Memory** (signatures, types, and caller counts) lowers $C_{dup}$ to **46.7%** while doubling reuse.
- **`eg` Implementation**: `eg search` generates zero-body Concise Interface Cards ranked by caller centrality (`BM25 + log(1 + callers)`), solving the exploration gap without triggering the copy-paste paradox. Post-edit `eg reuse` guards against $C_{dup} \ge 80\%$ duplication across both direct (1-hop) and indirect (`--deep-callee` 2-hop) call graphs.

### 2. Zero-Token Memory & Provenance Citation Locks (Zero-Mem & Agent Zero Memory)
Traditional agent memory systems introduce latency and hallucination by placing LLMs on memory operations:
- **Zero-Token Memory Operations**: [Zero-Mem](https://arxiv.org/abs/2607.29377) (*Xiao et al., 2026*) proves that eliminating LLMs from memory indexing, retrieval, and conflict resolution cuts latency by 57.6% (0.22s/query), eliminates 100% of LLM token costs, and achieves superior benchmark accuracy compared to prompt-heavy memory.
- **Destructive Summarization**: [Zhou et al.](https://arxiv.org/abs/2606.24775) (2026) demonstrated across 12 memory systems that storing raw interaction records yields 2× higher retrieval accuracy than LLM-generated summaries (24.2 EM vs 8.5 EM on LoCoMo), and local deterministic maintenance is Pareto-optimal ($30\text{--}40\times$ faster than global LLM graph rebuilding).
- **Citation-Locked Provenance**: [Agent Zero Memory](https://arxiv.org/abs/2608.29606) (*Wu & Zhu, 2026*) establishes that memory hallucination can only be eliminated when notes carry immutable provenance triples $\pi(x) = \langle \text{source}, \text{timestamp}, \text{raw evidence pointer} \rangle$ verified by citation locks.
- **`eg` Implementation**: `eg mem` executes all operations with **0 LLM tokens** in sub-millisecond SQLite time. Notes are anchored directly to AST definitions via `--refs`. `eg mem query` and `eg context` automatically verify anchors against the live AST index, badging broken or modified anchors with `[stale-ref]` alerts.

## Where to go next

- [CLI reference](cli-reference.md): command syntax, options, and output shapes.
- [Usage guide](usage.md): workflow choices and fallback steps.
- [Agent integration](agent-integration.md): MCP, skill routing, and the trust contract.
- [Evaluation](evaluation.md): reported measurements and their limits.
- [Public benchmark](../bench/README.md): a reproducible pointer regression corpus.
