# Ground Truth — 30 call sites (OWNER-FILLED, ~20 min)

Instruction: pick 30 real call sites from the anchor functions below (or others you consider representative).
For each row, open the file and record the exact call line. Mark how the callee is invoked — this determines
the EXTRACTED vs AMBIGUOUS expectation in kg: direct `from x import y` + call = EXTRACTED; `obj.method()`
where obj's type is obvious = EXTRACTED; anything duck-typed/dynamic = mark "dynamic" (kg will not resolve it;
it is excluded from the precision denominator per spec).

Anchor functions (from vihap survey 2026-10-31):
- G01-G08: `normalize_ascii` — vihap/cleaners/unicode.py:47 (45 referencing files)
- G09-G12: `get_knowledge_base` — vihap/knowledge/registry.py:25 (7 files)
- G13-G15: `make_alternation` — vihap/grammar/fragments.py:101 (7 files)
- G16-G20: stage classes of the parse flow — HierarchicalAddressParser, StreetValidator, QualityScorer, AddressFormatter, GsoIndex
- G21-G26: callees one hop from `HierarchicalAddressParser.parse` — vihap/parsers/hierarchical_parser.py:48
- G27-G30: your choice — any call site you consider tricky (alias imports, same-name methods, star imports)

| # | caller file:line | callee qualname | invocation kind (import-call / self / instance / dynamic) | owner-verified |
|---|------------------|-----------------|-----------------------------------------------------------|----------------|
| G01 |  |  |  | [ ] |
| ... |  |  |  |  |
| G30 |  |  |  | [ ] |
