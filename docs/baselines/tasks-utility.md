# Agent-Utility Task Battery — Features Without a Utility A/B

- Frozen: 2026-11-02, BEFORE any run. Do not edit prompts after the first
  cell of a stage executes; corrections go to this file with a dated note
  and invalidate earlier cells of the affected task.
- Scope: features whose gates were quality-only (G14 search recall, G16
  clones seeded detection, G17 path correctness+latency). Features already
  utility-tested (T1-T5 navigation, G13 outline) are NOT here.
- Stages: 1 = `path` (P1-P3, kg + vihap, expected sets reuse the frozen
  `path-gt.jsonl` oracle). 2 = `search` (S1-S3, expected sets derived +
  Goldfish-audited before Stage 2 starts). 3 = `clones` (C1-C2, seeded
  fixture repo + manifest).
- Hosts: Grok Build (`grok --always-approve --max-turns 40`,
  model grok-4.7-build) and OpenCode CLI (`opencode run --auto
  -m openai/gpt-6-sol --format json`). Same prompts for both.
- Arms (routing-level isolation, G13 precedent — SAME binary for both):
  - `kgc` control: the run does NOT use the feature under test; for
    reachability, compose `kg callees` / `kg callers` (or grep) and reason.
  - `kgp` treatment (Stage 1): run `kg path <from> <to>` first.
  Contamination guard: a control cell with > 0 feature calls in the kg
  journal is flagged `violations` and excluded from conclusions.
- Answer contract (every task): the agent writes ONE JSON file to the
  absolute path given in its prompt:
  `{"exists": true|false, "hops": <int>, "route": ["<path>:<line>", ...],
  "answer": "<one or two sentences>"}`.
  `route` = call-site lines in hop order; empty when `exists` is false.
- Budget: ≤ 25 tool calls per run; no file modification; no network.
- Recording: per-cell `runid = <arm>-r<run>-<task>`; KG_SESSION and KG_ARM
  both set to the runid for the agent process; kg journal
  (`.kg/usage.log`, `.kg/usage-results.jsonl`) carries `arm` fields.
- Scoring (Stage 1): correctness = exact `exists` + exact `hops` + route
  well-formed (repo-relative `path:line`, line within current file bounds,
  count == hops). No precision/recall (route tasks are not set tasks).
- Economics: host-recorded tokens (Grok `usage.json`; OpenCode session
  store), tool calls and output bytes from host histories, kg calls from
  the journal. Cross-host token comparison is INVALID (precedent:
  grok-a-b-2026-11-01.md) — report per host.

## Stage 1 prompts (frozen)

### P1 — route exists (kg repo)
Question: "Does `src.main.main` reach `src.extract.walk_node` through
calls? If yes, report the route: number of hops and the call-site line of
every hop."
Expected (frozen oracle, `path-gt.jsonl`): exists=true, hops=5.

### P2 — no route (kg repo, negative calibration)
Question: "Does `src.parse` reach `src.extract.record_ts_arrow_function`
through calls within the resolved call graph? If yes, report the route; if
not, answer that no route exists."
Expected: exists=false (exhaustive negative verified structurally at GT
freeze: 0 cross references).

### P3 — route exists (vihap)
Question: "Does `scripts.build_knowledge_base.main` reach
`vihap.cleaners.unicode.normalize_ascii` through calls? If yes, report the
route."
Expected: exists=true, hops=3.

## Stage 2 prompts (frozen; expected sets derived at stage start)

### S1 — locate by identifier vocabulary (kg repo)
"Which function splits camelCase and snake_case identifiers into tokens?
Report its path:line and qualname in the answer field (exists=true,
hops=0, route with the one definition pointer)."

### S2 — locate a replay verifier (kg repo)
"Which function replays sampled payload pointers and verifies them against
current source? Report its path:line and qualname in the answer field
(exists=true, hops=0, route with the one definition pointer)."

### S3 — locate a cleaner (vihap)
"Which cleaner function transliterates diacritics to ASCII? Report its
path:line and qualname in the answer field (exists=true, hops=0, route
with the one definition pointer)."

## Stage 3 prompts (frozen; seeded fixture + manifest at stage start)

### C1 — duplication audit (fixture repo)
"List every duplicated block pair in this repository with both
file:line-range spans. Put the pairs in the answer field."
### C2 — near-duplicate rejection (fixture repo)
"`near.py` resembles other files. Is it a duplicate block pair with any
file? Answer yes/no and justify from the tool output."

## Arm instructions (verbatim, embedded per prompt)

- kgc: "This run does NOT include the `kg path` command (Stage 1) /
  `kg search` (Stage 2) / `kg clones` (Stage 3). Determine the answer from
  `kg callers`, `kg callees`, `kg symbols`, and grep/read output."
- kgp: "For reachability questions run `kg path <from> <to>` FIRST; it
  prints the shortest route with one call-site pointer per hop. Use
  `kg callers`/`kg callees` only when `path` reports no route."

## Dated notes (protocol corrections; do not edit prompts retroactively)

- 2026-11-02, pre-run hardening (before any VALID cell): Stage 1 smoke
  revealed task leakage — the live kg repo contained `expected/P2.json` and
  `path-gt.jsonl`, readable by the agent (one smoke cell answered from the
  GT with 0 kg calls). Both smoke rounds were discarded. Corpora are now
  source-only sanitized copies under `/tmp/utility-corpus-2026-11-02/`
  (`docs/`, `skills/`, `tests/scripts/` stripped) and answer files live
  outside the corpus. Prompt templates were regenerated for the corpus
  paths under the same rule set; zero valid cells existed at edit time.
- 2026-11-02, runner fix: the skip pattern matched `^<runid> ` but status
  lines start with `[start]`/`[done]`, so two smoke cells were re-executed
  in the first background pass. Fixed to `^"\[done\] $runid "`. Duplicated
  status lines are retained as process evidence; unique cells = 24 and all
  artifacts (answer/log/journal/manifest) come from the last execution.

- 2026-11-02, S-stage GT audit (Goldfish, before any Stage-2 cell): S3
  expected corrected from `vihap/cleaners/unicode.py:23`
  (`_ascii_diacritic_trans`, a translate-table builder) to
  `vihap/cleaners/unicode.py:47` (`normalize_ascii`, the exported cleaner
  that takes text). S2/S3 prompts now carry the same route-schema
  parenthetical as S1. Expected sets gain `qualname`, and the scorer
  checks it in the answer text (the questions ask for it).
