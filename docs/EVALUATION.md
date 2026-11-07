# aw evaluation

## Former name

Historical benchmarks recorded before version 0.3.0 used the former project and command name `kg`. Quoted artifacts keep that name; current commands are `aw` from the `agent-weave` package.

## Corrections to earlier claims

The earlier introduction incorrectly described all results as measurements of the shipping binary. They span multiple historical snapshots and schemas. No benchmarks or runtime tests were run for this documentation update on 2026-11-03.

The earlier search table also conflated two metrics from `search_recall.py`. Mean query recall was **0.775**; pooled symbol recall was **22/28 = 0.786**. Queries have different expected-set sizes, so these values need not match.

The historical search scorer used permissive substring matching. Its reported
results have not been recomputed using the current identity-aware matching.
The historical caller gate also counts an ambiguous candidate as a hit; the
scorer now reports caller-site recall separately without that credit. Neither
change retroactively verifies the private historical results.

## Summary

Historical experiments found better navigation recall with aw and lower input-token use with query-first routing on one host. Feature-specific savings varied by task and host; test lookup and API inventory did not show savings in their narrow measured task shapes. A negative result in one shape is not a feature-level verdict.

| Question | Reported finding | Scope |
|---|---|---|
| Can pointers be checked against source? | 777/777 sampled pointers matched source locations | Six private corpora, 2026-11-01; not repository-wide recall |
| Can aw improve agent answers? | Mean navigation recall 81.9% without aw versus 98.6% with aw | Three tasks, six runs per arm, Grok Build |
| Does aw reduce tokens automatically? | No: unrouted aw increased tool calls 73%; input tokens fell only 2.8% | Same navigation experiment |
| Can routing reduce tokens? | Query-first input tokens fell 27.3% versus no aw | Ten runs per arm on that host |
| Which feature had repeatable savings? | `path` reduced calls on all four tested host/model pairs | Small, frozen route-task sets; not a general guarantee |
| Does aw understand program behavior? | No: these results concern static relationships and lexical retrieval | Dynamic and semantic questions still need source inspection |

All measurements below are **historical results reported from private evidence**. Raw corpora, frozen answers, agent transcripts, and some runners are unavailable in the public distribution. Public source tests can check behavior, but cannot reproduce these private numbers.

## 1. How to interpret the comparisons

Two different controls answer different questions. Do not interpret a feature comparison as evidence for aw versus no aw.

| Experiment | Control | Treatment |
|---|---|---|
| Navigation, 2026-11-01 | No aw: grep/rg/find and file reads | Same tools plus aw, either unrouted or query-first |
| Feature utility, 2026-11-02 | aw available, target feature forbidden | Same binary, target feature first |
| Structural-command utility, initial 2026-11-03 battery | aw available, target command forbidden | Same binary, target command first |
| Imports/dependency replication, later 2026-11-03 | No aw commands permitted | Target aw command required |

Reported agent metrics use host-recorded counters, not agent estimates. Tool calls count host tool invocations; aw calls count executed aw commands. A single shell invocation can contain multiple aw commands, so those totals are not interchangeable.

Recall means average the fraction of expected answers found per task or query. Pooled recall divides all matched expected items by all expected items. Pointer authenticity checks source locations; it does **not** measure missing relationships or semantic correctness.

### Historical corpus scope

Neutral labels A–F identify the private corpora referenced below. Individual private corpus sizes are withheld to avoid identifying repositories through size fingerprints.

| Corpus | Description | Languages |
|---|---|---|
| A | Address-parsing monorepo | Python |
| B | Product prototype | TypeScript/TSX |
| C | Multi-tenant collection backend | Python |
| D | DAG/notebook generator | Python |
| E | Map-search service | Python |
| F | Data-science pipeline | Python |

The public aw-self corpus used an earlier Rust snapshot of this repository: 37 indexed files, 380 symbols, and 1,171 edges. These are historical counts, not current repository sizes.

## 2. Agent navigation: aw versus no aw

The main comparison used corpus A and five frozen tasks: hub callers, state-hub callers, signature impact, flow tracing, and transitive callees. Answers were mechanically scored against frozen expected sets; the private task collection included 60 verified call-site rows.

The 2026-11-01 run used Grok Build 1.0.70, model `grok-4.7-build`, and a post-optimization snapshot recorded as `82d0b02`. It used `grok --always-approve --max-turns 40`, two runs per task and arm, then ten additional query-first runs. Exact prompts, session IDs, scorer inputs, and accounting scripts are private.

| Metric | No aw | aw, unrouted | aw, query-first | Sample |
|---|---:|---:|---:|---|
| Mean recall, first three tasks | 81.9% | 98.6% | 98.6% | Six answers per arm |
| Input tokens, mean/run | 1,058,613 | 1,028,665 | 769,306 | Ten runs per arm |
| Tool calls, mean/run | 20.6 | 35.7 | 24.9 | Ten runs per arm |
| aw calls, mean/run | 0 | 3.7 | 8.8 | Ten runs per arm |
| Output tokens, recorded mean/run | 13,773 | 13,403 | 10,850 | Nine-run accounting summaries; ten-run means unavailable |
| Model calls, recorded mean/run | 20.2 | 19.9 | 17.2 | Nine-run accounting summaries; ten-run means unavailable |

Query-first input tokens were 27.3% below the no-aw control, but tool calls remained higher. Merely making aw available did not reproduce an earlier scripted comparison's 50% call reduction or 67% byte reduction.

The state-hub task improved from 50% to 100% recall in both aw runs. Flow tracing succeeded in zero of two no-aw runs, one of two unrouted runs, and both query-first runs. These are small samples, not per-task guarantees.

The native Grok query-first transitive-callees runs scored 0% and 40% recall. A failed initial run was retried; a separate OpenCode `openai/gpt-6-luna` alternate scored 100%, while `gpt-6.1-terra` scored 40%. These alternates are not included in Grok token means.

### Host/model dependence

A ten-run OpenCode `openai/gpt-6-luna` query-first replication scored 85.9% mean recall on the first three tasks, versus Grok's 98.6%. It retained 100% recall on the state-hub and transitive-callees tasks, but had no same-host no-aw economics control.

The primary collection contained 40 runs across the navigation arms and hosts, plus two alternate fills. Token counters differ between hosts; no cross-host token comparison is valid.

### Scoring limitations

- Each task/arm had only two runs. No statistical significance claim is made.
- Grouping and ordering fields failed across arms where prompts lacked JSON-shape examples.
- Some expected answers excluded test callers without stating that exclusion in the question. Extra test pointers reduced precision.
- Flow and transitive-callees tasks were recall-only, not part of the three-task headline mean.
- The deep navigation comparison covered corpus A, not every corpus in the table above.

## 3. Feature utility: aw with and without one command

The 2026-11-02 experiments used sanitized source-only corpora and frozen answers outside the queried tree. Controls could use other aw commands or grep. Treatments had to call the target feature first; both arms used the same binary within each experiment.

The first two hosts were Grok Build 1.0.70 (`grok-4.7-build`) and OpenCode CLI (`openai/gpt-6-sol`). The records identify feature-era builds rather than one common commit. Deltas below compare mean tool calls and mean input tokens **within each host**.

| Feature/task set | Total runs across both arms/hosts | Exact-answer matches, combined | Grok tools / input tokens | OpenCode tools / input tokens |
|---|---:|---:|---|---|
| `path`, three route tasks | 24 | 24/24 | -57% / -57% | -86% / -36% |
| `path`, 2,201-file synthetic chains | 24 | 24/24 | -59% / -43% | -61% / -12% |
| `search`, identifier-vocabulary lookup | 24 | 24/24 | -18% / +0.2% | +3% / -36%, cache-contaminated |
| `search`, signature/delegation retest | 16 | 16/16 | -38% / -28% | -17% / -32%, cache-noisy |
| `clones`, seeded exact duplicates | 16 | 16/16 | -59% / -39% | -17% / +32%, cache-contaminated |

Each task/arm/host had two runs. The search retest followed an index upgrade that added path/signature tokens. It still failed the experiment's requirement for at least 20% call reduction on both hosts.

Clone utility covered a 12-file fixture with four seeded exact pairs, a renamed near-duplicate, and decoys. Controls happened to use no aw commands. This does not establish utility for semantic duplication or arbitrary real repositories.

### Two additional hosts

The same feature-control design was run with agy 1.4.12 (`gemini-3.8-flash-high`) and zcode 0.1.82 (`deepseek-ai/DeepSeek-V4.1-Flash-1017`). The 52 runs comprised 24 path runs and 28 search/clone/context runs; all retained answers matched their expected sets.

| Feature | agy tools / input tokens | zcode tools / input tokens | Runs per task/arm/host |
|---|---|---|---:|
| `path` | -71% / -63% | -63% / -36% | 2 |
| `context` | -28% / -36% | -48% / -25% | 1 |
| `search` | -38% / -47% | -29% / +10% | 1 |
| `clones` | -22% / -18% | -6% / -2% | 1 |

Search increased aw calls on both additional hosts. Clone and context results are directional at one run per cell. Five agy path controls exceeded the 25-tool budget and were retained; one hung control was rerun.

Two contaminated smoke runs were discarded before valid measurement. One context task used a dated host-specific answer overlay containing two additional source-verified sites; it did not use exactly the earlier shared oracle.

### Structural commands, 2026-11-03

The initial experiment used aw `6dbe57a`, schema v7: five tasks, two arms, four hosts, and two runs per cell. Grok, agy, and zcode used the models named above. OpenCode was excluded after workspace-isolation failures; its exact model is not established here.

| Task | Grok mean tool calls | agy mean tool calls | zcode mean tool calls |
|---|---:|---:|---:|
| Imports | 14.0 → 4.0 (-71%) | 18.0 → 14.5 (-19%) | 4.0 → 4.0 (0%) |
| Test reachability | 5.5 → 4.5 (-18%) | 4.5 → 8.5 (+89%) | 6.0 → 6.5 (+8%) |
| Implementers | 5.5 → 4.5 (-18%) | 18.0 → 7.0 (-61%) | 5.0 → 5.0 (0%) |
| Direct import impact | 8.5 → 6.0 (-29%) | 20.5 → 13.0 (-37%) | 10.0 → 5.5 (-45%) |
| API inventory | 3.5 → 4.0 (+14%) | 2.0 → 4.5 (+125%) | 3.5 → 4.5 (+29%) |

Of 80 planned cells, 20 OpenCode cells and two contaminated agy controls were excluded, leaving 58 valid cells. Imports and implementers each retain only one agy control; other valid cells have two runs. All retained answers were correct.

A later audit found that blanket OpenCode exclusion could bias the comparison: some cells reached the intended corpus by absolute path. OpenCode controls had nine correct captured answers; treatment had four wrong answers and one missing answer. These cells need individual workspace audits before reuse, not a claim of correctness parity.

### Replication with a different control

A later replication prohibited **all aw commands in controls**, unlike the initial feature-control battery. It tested imports on aw corpus `bc292ba` and direct dependency impact on corpus A `d060ace`, using the same three valid host/model pairs. The report does not pin a separate common binary revision.

| Task | Grok median tool calls | agy median tool calls | zcode median tool calls |
|---|---:|---:|---:|
| Imports | 6 → 3 (-50.0%) | 21 → 5 (-76.2%) | 7 → 3 (-57.1%) |
| Direct import impact | 6 → 5 (-16.7%) | 18 → 6 (-66.7%) | 15 → 7 (-53.3%) |

The target was five runs per arm/host. The agy imports control instead reports the median of six correct runs, excluding one wrong answer and one forbidden-aw run. Other cells report five correct runs per arm; no token-saving claim accompanies this replication.

Concurrent journals were contaminated, so control validity was checked from transcripts. Imports savings appeared on all three tested hosts; dependency-impact savings remained host-dependent. The small initial battery does not justify removing features that lacked savings on one task shape.

### Task-shape results are not feature-level verdicts

A negative delta means the feature did not reduce effort in that one task
shape; it does not establish that the feature has no value. The feature
review records the dispositions and the missing shapes: `tests` needs a
depth>1 transitive test-selection task; API inventory needs a directory or
multi-file export task; `clones` is partial in its seeded fixture; `dead`
and `complexity` have no utility task. The review recorded no REMOVE
decision; it demoted `search` filters and the `aw api` surface (later
removed). See [the P8 feature review](baselines/reviews/R14-p8/README.md)
and the [2026-11-02 utility battery](baselines/agent-utility-2026-11-02.md).

## 4. Static accuracy and coverage

High accuracy on selected expected relationships does not imply complete repository coverage. Across six measured corpora, 51.8–90.4% of raw call sites remained unresolved; separate large-corpus checks reported 71.7% and 62.9% unresolved. Notebook-heavy corpora had approximately 10–19% resolution coverage.

The following results are historical source/fixture checks, not agent-answer scores. They use several snapshots; model and agent host are not applicable.

| Check | Reported result | Snapshot and measurement method |
|---|---|---|
| Pointer authenticity | 777/777 sampled pointers matched source lines; zero parse errors across measured corpora | 2026-11-01 post-optimization build; `callers`, `impact`, `dead`, and `symbols` source checks |
| Caller ground truth | 29/29 and 20/20 private expected sites; refreshed self set 10/10; pooled call-site precision 662/664 | Historical caller checks and later self refresh; `precision_recall.py`; two known precision failures |
| Search, 20 queries / 28 expected symbols | Mean recall 0.850 → 0.800 → 0.775; pooled hits 24/28 → 23/28 → 22/28 | Schema v4 then two schema-v5 snapshots, 2026-11-01/02; `search_recall.py`, top 10 |
| Clone quality fixture | 10/10 seeded pairs, zero spurious pairs, eight distractors and one near-duplicate | 2026-11-01 feature build; `clones_seed_gate` |
| Path quality | Ten exact-output fixtures; 8/8 frozen route pairs agreed | 2026-11-02 feature build; `path_seed_gate` and `path_recall.py` |
| Outline utility | Seven calls versus 12 with grep, but six with aw without outline; bytes -62.7% versus grep | aw `fe1fcc0`, schema v4; private five-task scripted replay, not a free-running agent A/B |

Search ranking improved some signature-only lookups while reducing mean recall on the frozen query set. For example, one wrapped-parameter query moved from outside the top 50 to rank 2 after signature extraction changed. This is a trade-off, not a universal retrieval improvement.

### Imports, tests, and inheritance

The source-derived expected sets were frozen on 2026-11-02 at aw `342ebc3`, corpus A `efcd310`, and corpus B `dddfe07`. A 2026-11-03 rerun used corpus A `d060ace` and refreshed one moved self-source line. The harnesses are public, but their frozen inputs and private working trees are not.

| Relationship | Expected-row recall | Reported precision | Wilson 95% interval for recall | Public harness |
|---|---:|---:|---|---|
| Imports | 40/40 | 183/183 | 0.912–1.000 | [import_recall.py](../tests/scripts/import_recall.py) |
| Test reachability (measured for `aw tests`, removed in 0.5.0) | 20/20 | 2,679/2,679 | 0.839–1.000 | harness retired with the command |
| Inheritance | 17/17 | Not measured | 0.816–1.000 | [inheritance_recall.py](../tests/scripts/inheritance_recall.py) |

Imports comprised 25 corpus-A rows and 15 aw-self rows. Test recall covered 20 corpus-A symbols across 20 files. Inheritance covered 15 Python rows, including five cross-file rows, and two TypeScript rows, including one cross-file row.

Import precision checks whether emitted source blocks are import-shaped and name the target leaf. Test precision checks test-shaped qualnames, not runtime execution or proof that a test validates the target. These are narrower metrics than semantic precision.

Excluded relationships included function-local Python imports as test seeds, cross-file Rust trait implementations, TypeScript `tsconfig` path aliases, and Dart. Tests used `--limit 10000` to avoid the default display cap. A Flutter-tree smoke build indexed 124 supported files and 735 symbols with zero parse errors; it did **not** demonstrate Dart support.

## 5. Historical performance

Timing results describe local measurements, not service-level guarantees. Best-of-N is a minimum, **not p95**; earlier wording mixed those metrics. Cache state and host load affect absolute times, and cold-cache ratios were not established.

The schema-v5 record used macOS 27.0.0 arm64, eight CPUs, and 16 GB RAM on 2026-11-02. Its exact commit was described as a remediation snapshot, not pinned by hash in that record. The public synthetic generator produced 2,025 files, 85,050 symbols, and 164,025 edges.

| Scenario | Reported measurement | Historical command/method |
|---|---|---|
| Schema-v5 fresh build | 2.33 s; peak RSS 183,222,272 B; index 57,393,152 B | `aw build --force --root <synth>`; repetition count unspecified |
| Schema-v5 no-op rebuild | 1.65 s; peak RSS 88,997,888 B | `aw build --root <synth>`; repetition count unspecified |
| Schema-v5 common queries | Under 10 ms on the synthetic corpus | `callers`, `callees --depth 3`, `path`, `search`, `clones --limit 5`; repetition count unspecified |
| Later schema-v5 no-op, 2,201 files | 0.02–0.03 s; index byte-unchanged | `/usr/bin/time -p aw build`, hash/mtime comparison |
| Schema-v7 fresh build, same 2,025-file generator | 2.70–2.79 s, best-of-seven summaries | `aw build --force`; pre-later-build-optimization snapshot |
| Schema-v7 dead candidates, 85,050 symbols | 147.5 ms, best of seven | `aw dead --limit 50`, `timing_assert.sh` |
| Schema-v7 complexity, 85k / 987k symbols | 19.3 / 6.9 ms | `aw complexity --limit 1`, `timing_assert.sh`; best-of-seven reported for 85k |
| Schema-v7 path-filtered complexity, 85k / 987k symbols | 26.2 / 201 ms | `aw complexity --path synth`, `timing_assert.sh` |
| Schema-v7 symbols, 85k symbols | 21.9 ms, best of seven | `aw symbols synth --limit 50`, `timing_assert.sh` |

The later query records do not consistently pin binary hashes or complete host metadata. Do not merge these rows into one current-performance claim. Early schema-v3 optimization ratios are omitted because later schemas materially changed index contents and build costs.

### Later build comparison and memory trade-off

A 2026-11-03 interleaved comparison used aw 0.2.1/schema v7 at `6dbe57a` versus `968e160`. It used the same 2,025-file synthetic corpus and macOS host specification above. Seven baseline/final pairs ran under heavy external load, with load averages 41.66–53.89 on eight CPUs.

| Metric | Baseline | Optimized | Delta |
|---|---:|---:|---:|
| Best wall time | 3.61 s | 1.87 s | -48.2% |
| Median wall time | 4.04 s | 1.99 s | -50.7% |
| Peak RSS | 155,370,048 B | 194,839,104 B | +25.4% |
| Index size | 69,103,616 B | 66,404,352 B | -3.9% |

Measurement used interleaved `aw build --force` invocations and resource accounting recorded privately. The speed gain came with approximately 40 MB higher peak RSS. A quiet-host remeasurement is needed before presenting 1.87 s as a typical absolute build time.

### Large or adversarial indexes

These historical stress results are not representative-repository averages. Their local synthetic indexes are unavailable publicly; exact binary/host metadata is incomplete.

| Scenario | Reported result | Historical command/method |
|---|---|---|
| Approximately 1.09M matching symbols, schema v5 | Search 0.49 s, 18.9 MB RSS | `aw search a` |
| 700 files sharing a five-line header, schema v5 | Clone candidates capped; 6.3 MB RSS | `aw clones`; lower-bound output, not exhaustive enumeration |
| 4,000-file chain, one tail edit, schema v5 | Incremental rebuild 0.20 s versus 42.5 s before the fix | `aw build` after edit |
| 1.37M-edge hub, schema v7 | Impact 97.1–144.3 ms across best-of-seven summaries | `aw impact synth/core.py --depth 3`, `timing_assert.sh` |
| Same hub with 10,000 tests | 63.0–113.2 ms, depending on seed | `aw tests <seed> --depth 10`, best of seven |
| 3.1M-edge stress hub | Impact 254.3 ms; tests 363.4 ms | Same command shapes, best of five; exceeded the smaller graph's budgets |

The hub measurements did not cover the reverse-search branch used above 20,000 test-shaped symbols. Display caps and bounded collectors do not imply constant query time: traversal and matching still depend on graph shape and match count.

## 6. What you can reproduce publicly

The commands below are **verification recipes, not results from this documentation update**. Run them from the repository root. They check public fixtures and current source behavior, not the historical agent economics or private recall scores.

| Behavior | Public source | Targeted command |
|---|---|---|
| Static resolution and negative controls | [tests/fixtures.rs](../tests/fixtures.rs) | `cargo test --locked --test fixtures` |
| Outbound imports | [tests/cli.rs](../tests/cli.rs), `imports_lists_outbound_resolved_edges` | `cargo test --locked --test cli imports_lists_outbound_resolved_edges` |
| Test reachability | Same file, `tests_impact_seed_covers_direct_indirect_depth_and_file_seed` | `cargo test --locked --test cli tests_impact_seed_covers_direct_indirect_depth_and_file_seed` |
| Route fixtures | Same file, `path_seed_gate` | `cargo test --locked --test cli path_seed_gate` |
| Duplicate-block candidates | Same file, `clones_seed_gate` | `cargo test --locked --test cli clones_seed_gate` |
| Search behavior | Same file, `search_finds_symbols_ranked_and_empty_or_nomatch_exit_codes` | `cargo test --locked --test cli search_finds_symbols_ranked_and_empty_or_nomatch_exit_codes` |
| Untouched no-op index | Same file, `noop_build_leaves_the_index_untouched` | `cargo test --locked --test cli noop_build_leaves_the_index_untouched` |
| CLI/MCP contracts | [tests/mcp.rs](../tests/mcp.rs) | `cargo test --locked --test mcp` |
| Public navigation pointers | [bench/](../bench/README.md) | `cargo build --release && python3 -I bench/run.py target/release/aw` |
| Memory retrieval recall + negative control | [tests/fixtures/memory/oracle-v2.json](../tests/fixtures/memory/oracle-v2.json) | `cargo build --release && python3 -I tests/scripts/mem_recall.py --aw target/release/aw` |

The public pointer benchmark is a committed synthetic corpus with expected
pointers per command. It is a regression gate for the public corpus; it does
not reproduce the private agent experiments or measure token savings, tool-call
reduction, or dynamic-dispatch coverage.

For local synthetic measurements, inspect [make_synth_corpus.py](../tests/scripts/make_synth_corpus.py) and [timing_assert.sh](../tests/scripts/timing_assert.sh). The latter reports best-of-N wall time, accepts semantic exit codes 0/1/2, and does not calculate percentiles.

Record the commit, binary build mode, operating system, CPU/RAM, corpus generator arguments, indexed totals, cache conditions, repetitions, and exclusions. Preserve every timing sample if you need a median or p95. Do not label new fixture results as reproductions of private agent experiments.

## 6a. Project memory: retrieval recall (measured 2026-11-06)

The project-memory resolver (design: [memory retrieval](designs/2026-11-05_agent-weave-memory-retrieval.md)) is measured against a frozen oracle, not agent economics. Method follows the standing experiment-plan principles: the fixture freezes 30 notes and 20 queries with expected keys and the negative-control seed before any run; measurement is deterministic (same DB + query + binary → same ranking); the script is stdlib Python only.

- **Command:** `python3 -I tests/scripts/mem_recall.py --aw target/release/aw --json docs/baselines/2026-11-06_memory-phase3/recall.json`
- **Fixture:** [tests/fixtures/memory/oracle-v2.json](../tests/fixtures/memory/oracle-v2.json) (30 notes, 20 queries, seed 20261105)
- **Result (aw, release build, 2026-11-06):** recall@5 = 20/20 = **1.000**, Wilson 95% interval [0.839, 1.000]. Negative control (frozen shuffle of the term→key mapping, scored against the same retrievals): 1/20 = **0.050**, clearly below the 0.8 gate — the ranking is load-bearing, not a pass-everything store. Gate: PASS. Evidence: [docs/baselines/2026-11-06_memory-phase3/recall.json](baselines/2026-11-06_memory-phase3/recall.json).
- **Weight-tuning rule:** the resolver weights (0.5 lexical / 0.3 ref-overlap / 0.2 recency) change only if a frozen-oracle run drops below 0.8, with before/after numbers recorded here and in the changelog. No retune was needed for this measurement.
- **Feature retention:** `aw usage --quality` splits `mem_write`/`mem_query` call and empty rates from the journal. A measurement window with zero `mem_write` calls makes the feature a recorded drop candidate rather than a permanent fixture.

**What this does not establish:** one 30-note synthetic fixture, one repository, one binary build; recall over the oracle is not agent utility (no task-completion A/B), not precision under adversarial duplicates, and not transferable to other note distributions. The negative control rules out a pass-everything store, not every trivial-ranking failure mode.

## 7. Practical limits

- Static edges are navigation candidates, not proof of runtime behavior. Dynamic dispatch and untyped attributes can hide callers and paths.
- Empty output does not prove absence. Check unresolved sites, stale indexes, supported languages, and display limits before drawing conclusions.
- Duplicate-block candidates do not prove semantic duplication; test reachability does not prove test adequacy.
- Fewer calls do not establish lower monetary cost, latency, or production error rates. Measure those outcomes separately.
- Host/model behavior, prompt routing, cache accounting, narrow tasks, and small samples limit transfer to other workflows.
- Private measurements remain reported evidence, not independently reproducible public benchmarks.
