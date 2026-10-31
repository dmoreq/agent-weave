# P0 Navigation Tasks — vihap (frozen for A/B)

- Date frozen: 2026-10-31
- Rule: each task is executed by the agent with ONLY grep/Read/Glob (baseline arm) or with `kg` available (treatment arm, P1). Same model, same day, transcripts recorded under `transcripts/`.
- Success: expected answer found (files + lines), scored by D0 correctness checker.

## T1 — hub callers (cross-cutting function)
- Question: "Find every call site of `normalize_ascii` (defined in vihap/cleaners/unicode.py) outside tests/, and group them by top-level package dir (cleaners, parsers, validators, ...)."
- Expected essence: call-site pointers in ~40+ files across most package dirs; grouping correct.
- Ground-truth anchor: GT rows G01-G08 (owner-verified sample).

## T2 — state-hub callers
- Question: "Who calls `get_knowledge_base` (vihap/knowledge/registry.py), and in what order do those callers run during a single parse request?"
- Expected essence: ~7 caller files + ordering consistent with the parse flow (parsers -> validators/reform -> quality).
- Ground-truth anchor: GT rows G09-G12.

## T3 — impact of a signature change
- Question: "If `make_alternation` in vihap/grammar/fragments.py changes its signature, which files (outside tests/) must be updated, and at which lines do they call it?"
- Expected essence: 7 caller files with exact call lines.
- Ground-truth anchor: GT rows G13-G15.

## T4 — architecture data flow
- Question: "Trace the full parse flow from raw address text to a `ParsedAddress`: which stages/modules run in order, and which class in each stage?"
- Expected essence: cleaners -> parsers (HierarchicalAddressParser + grammar) -> validators (GsoIndex) -> quality (QualityScorer) -> formatters (AddressFormatter) -> models.ParsedAddress.
- Ground-truth anchor: GT rows G16-G20 (stage classes).

## T5 — callees of an entry class
- Question: "List the cross-module callees of `HierarchicalAddressParser.parse` (vihap/parsers/hierarchical_parser.py): which functions/classes in other package dirs does it invoke, directly or one hop deep?"
- Expected essence: callees in grammar/, validators/, quality/, knowledge/ with defining files.
- Ground-truth anchor: GT rows G21-G26.
- Correction (2026-10-31, measured at f7ca48c): one-hop cross-module callees are parsers/macro_resolver (x3 methods), formatters/AddressFormatter.format, quality/QualityScorer.score (one hop via _assemble_parsed_address). grammar/, validators/, knowledge/ are >= 2 hops. Scoring uses expected/T5.json, not the prose above.

## Scoring notes
- Tool calls + bytes counted per task by D0 harness (both directions).
- Correctness: checker (harness/check.py) compares found pointers vs the frozen expected set (expected/<TASK>.json); precision/recall per task; AMBIGUOUS-candidate hits count as recall hits only when the true callee appears in the candidate list (answer key `candidate_hits`); dynamic-dispatch answers are excluded from the precision denominator (answer key `dynamic_excluded`); T1 grouping is scored per-dir; ordering keys are normalized to the token before " (" so prose annotations do not gate; T4/T5 expected sets are verified minimal cores scored `recall_only` (extras are not false positives).
- Baseline expectation to beat (G6): treatment arm must cut total tool calls >= 30% without byte growth or correctness loss.
