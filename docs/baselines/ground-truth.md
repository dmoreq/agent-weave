# Ground Truth — 30 call sites (FILLED 2026-10-31)

- Filled: 2026-10-31, agent verification (owner delegated execution: "tự lựa chọn option tốt nhất và tiếp tục").
- Provenance: every row's caller line was read in the vihap working tree at rev `f7ca48c` (2026-10-30T15:10:07+07:00).
- Verification legend: `[x]` = verified by direct code read (import line + call line); re-confirm by owner optional, not blocking.
- No star imports exist in vihap (verified `rg "from .* import \*"` = 0 hits) — the spec's star-import fixture case must be a synthetic fixture, not a vihap row.
- Known duplicate-name trap: `normalize_ascii` is defined in BOTH `vihap/cleaners/unicode.py:47` and `vihap/cleaners/text_utils.py`; per-module import maps decide the target (G01 vs G02). kg must not merge them.

Instruction (original): pick 30 real call sites from the anchor functions below. For each row, record the exact
call line and how the callee is invoked — this determines the EXTRACTED vs AMBIGUOUS expectation in kg: direct
`from x import y` + call = EXTRACTED; `obj.method()` where obj's type is obvious = EXTRACTED; anything
duck-typed/dynamic = mark "dynamic" (kg will not resolve it; it is excluded from the precision denominator per spec).

Anchor functions (from vihap survey 2026-10-31):
- G01-G08: `normalize_ascii` — vihap/cleaners/unicode.py:47 (45 referencing files)
- G09-G12: `get_knowledge_base` — vihap/knowledge/registry.py:25 (7 files)
- G13-G15: `make_alternation` — vihap/grammar/fragments.py:101 (7 files)
- G16-G20: stage classes of the parse flow — HierarchicalAddressParser, StreetValidator, QualityScorer, AddressFormatter, GsoIndex
- G21-G26: callees one hop from `HierarchicalAddressParser.parse` — vihap/parsers/hierarchical_parser.py:86
- G27-G30: tricky sites (alias import, getattr dispatch, typed-param receiver, module-local shadowing)

| # | caller file:line | callee qualname | invocation kind (import-call / self / instance / dynamic) | verified |
|---|------------------|-----------------|-----------------------------------------------------------|----------|
| G01 | vihap/grammar/hamlet.py:150 | vihap.cleaners.unicode.normalize_ascii | import-call (rel import hamlet.py:14) | [x] |
| G02 | vihap/short_parser.py:34 | vihap.cleaners.text_utils.normalize_ascii | import-call (rel import short_parser.py:8 — text_utils, NOT unicode) | [x] |
| G03 | vihap/parsers/alley.py:497 | vihap.cleaners.unicode.normalize_ascii | import-call (rel import alley.py:11) | [x] |
| G04 | vihap/validators/street_classifier.py:215 | vihap.cleaners.unicode.normalize_ascii | import-call (street_classifier.py:11) | [x] |
| G05 | vihap/validators/category.py:174 | vihap.cleaners.unicode.normalize_ascii | import-call (category.py:19) | [x] |
| G06 | vihap/validators/gso_index.py:43 | vihap.cleaners.unicode.normalize_ascii | import-call (gso_index.py:10) | [x] |
| G07 | vihap/reform/street_district_lookup.py:30 | vihap.cleaners.unicode.normalize_ascii | import-call (absolute import street_district_lookup.py:9) | [x] |
| G08 | vihap/validators/street_validator.py:100 | vihap.cleaners.unicode.normalize_ascii | import-call via package re-export (`from ..cleaners import ... normalize_ascii`, street_validator.py:13) | [x] |
| G09 | vihap/reform/mapping.py:26 | vihap.knowledge.registry.get_knowledge_base | import-call (mapping.py:21) | [x] |
| G10 | vihap/validators/gso_loader.py:613 | vihap.knowledge.registry.get_knowledge_base | import-call (gso_loader.py:22) | [x] |
| G11 | vihap/validators/street_validator.py:295 | vihap.knowledge.registry.get_knowledge_base | import-call, function-LOCAL import (street_validator.py:293, inside function body) | [x] |
| G12 | vihap/knowledge/reform_utils.py:30 | vihap.knowledge.registry.get_knowledge_base | import-call (rel import reform_utils.py:9) | [x] |
| G13 | vihap/grammar/civic.py:61 | vihap.grammar.fragments.make_alternation | import-call, module-level call site (civic.py:15) | [x] |
| G14 | vihap/grammar/admin.py:69 | vihap.grammar.fragments.make_alternation | import-call, multi-line import (admin.py:12-19), module-level call | [x] |
| G15 | vihap/grammar/fragments.py:128 | vihap.grammar.fragments.make_alternation | same-file local module call | [x] |
| G16 | vihap/pipeline.py:40 | vihap.parsers.hierarchical_parser.HierarchicalAddressParser.__init__ | import-call constructor (pipeline.py:17) | [x] |
| G17 | vihap/parsers/street_parser.py:353 | vihap.validators.street_validator.StreetValidator.__init__ | import-call constructor (street_parser.py:209) | [x] |
| G18 | vihap/parsers/hierarchical_parser.py:79 | vihap.quality.confidence.QualityScorer.__init__ | import-call, function-LOCAL import (hierarchical_parser.py:77) | [x] |
| G19 | vihap/reform/translator.py:55 | vihap.formatters.standard_formatter.AddressFormatter.__init__ | import-call, function-LOCAL import (translator.py:53) | [x] |
| G20 | vihap/validators/gso_index.py:633 | vihap.validators.gso_index.GsoIndex.__init__ | same-file constructor call | [x] |
| G21 | vihap/parsers/hierarchical_parser.py:102 | vihap.parsers.hierarchical_parser.HierarchicalAddressParser._parse_address_fused | self (direct callee of parse, def at :110) | [x] |
| G22 | vihap/parsers/hierarchical_parser.py:121 | vihap.parsers.hierarchical_parser.HierarchicalAddressParser._preprocess_input | self (one hop via _parse_address_fused) | [x] |
| G23 | vihap/parsers/hierarchical_parser.py:137 | vihap.parsers.hierarchical_parser.HierarchicalAddressParser._resolve_admin_nodes | self (one hop) | [x] |
| G24 | vihap/parsers/hierarchical_parser.py:179 | vihap.parsers.macro_resolver.MacroAdminResolver.reconcile_admin_nodes | instance — receiver assigned from single constructor `macro_resolver or MacroAdminResolver(...)` (hierarchical_parser.py:68) | [x] |
| G25 | vihap/parsers/hierarchical_parser.py:195 | vihap.parsers.macro_resolver.MacroAdminResolver.resolve_centroid_coordinates | instance — same receiver as G24 | [x] |
| G26 | vihap/parsers/hierarchical_parser.py:426 | vihap.formatters.standard_formatter.AddressFormatter.format | instance — receiver from annotated param `active_formatter: AddressFormatter` | [x] |
| G27 | vihap/validators/gso_loader.py:703 | vihap.knowledge.marshal_io.default_artifacts_dir | import-call via ALIAS (`default_artifacts_dir as knowledge_artifacts_dir`, gso_loader.py:21) | [x] |
| G28 | vihap/pipeline.py:113 | (dynamic) getattr(self.parser, "get_params", None) | dynamic — excluded from precision denominator per spec | [x] |
| G29 | vihap/parsers/street_parser.py:2023 | vihap.parsers.street_parser.MicroStreetParser.parse | instance — receiver from annotated param `parser: MicroStreetParser` in `_harvest_from_address_segment`; same-name `parse` exists on 3 classes + module func (__init__.py:47) so bare-name resolution would be AMBIGUOUS — receiver-class path required | [x] |
| G30 | scripts/build_knowledge_base.py:636 | scripts.build_knowledge_base.normalize_ascii (module-LOCAL def at :64) | same-file local shadowing — must NOT resolve to vihap.cleaners.unicode.normalize_ascii | [x] |

Spares (verified, use if a row is invalidated by refactors):
- vihap/knowledge/base.py:209 (import-call)
- vihap/validators/building_validator.py:406 (import-call, building_validator.py:14)
- vihap/validators/street_validator.py:88 (module-level import-call)
- vihap/parsers/street_parser.py:1855 `self.parse` (self, MicroStreetParser batch path)
- vihap/validators/street_validator.py:310 instance `self.streets_by_ward.get(...)`

Notes for G2 scoring:
- G28 is dynamic: kg drops it (no edge); excluded from precision denominator per spec.
- G29: if kg resolves only via constructor-assignment receivers and ignores param annotations, expected outcome is AMBIGUOUS (not a precision miss) — record which rule fired.
- G30: a merged or cross-module resolution here is a precision MISS (hard trap against name-index-only solutions).
