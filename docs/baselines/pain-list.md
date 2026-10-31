# Pain List — top-10 files (PROVISIONAL — agent-derived, owner confirmation pending)

- Status: PROVISIONAL. Filled 2026-10-31 from objective git signals at vihap rev `f7ca48c` (owner delegated execution, but felt pain is owner ground truth — re-rank or veto rows before P1.5).
- Methodology: rank by fix-density (fix/bug-keyword commits touching the file, 180d) + churn-90d (all commits touching the file); LOC shown for context. Repo has 491 commits in the 90d window, so churn is a real signal, not squashed-history noise.
- Usage: ONLY gate G7 (hotspots rank-overlap >= 7/10). Until the owner confirms, G7 runs in degraded mode and its verdict must not stop the project.

Instruction (original): rank the 10 files in vihap you consider most "painful" — the ones you dread touching,
that hide bugs, or that you would refactor first. Used ONLY by gate G7 (hotspots rank-overlap >= 7/10).
Your list is the ground truth for whether the score mirrors reality; no other use.

| rank | file | fix-180d | churn-90d | LOC | one-line reason |
|------|------|----------|-----------|-----|-----------------|
| 1 | vihap/parsers/street_parser.py | 25 | 51 | 2086 | Highest fix density + largest file; MicroStreetParser mega-class owns street/alley/house-number heuristics |
| 2 | vihap/__init__.py | 19 | 70 | 147 | Facade hub: 70 touches in 90d; every API change fans through here |
| 3 | vihap/parsers/hierarchical_parser.py | 12 | 40 | 639 | Parse-flow orchestrator; stage wiring changes ripple into validators/formatters |
| 4 | vihap/validators/street_validator.py | 16 | 30 | 840 | Alias maps, function-local imports, package re-export calls — the resolution-hard file |
| 5 | vihap/cleaners/__init__.py | 12 | 39 | 306 | Re-export hub (incl. normalize_ascii); rename hazards propagate to every importer |
| 6 | vihap/constants.py | 12 | 26 | 190 | Cross-cutting constants; 12 fix commits mean value drift keeps breaking callers |
| 7 | vihap/parsers/alley.py | 8 | 21 | 587 | Alley/house-number extraction churns with address-format edge cases |
| 8 | vihap/validators/gso_loader.py | 8 | 19 | 721 | GSO loading + alias-import caller; artifact-path coupling with knowledge/ |
| 9 | vihap/validators/category.py | 0* | 20 | 741 | Churn without tagged fixes; category keyword normalization logic |
| 10 | vihap/cleaners/text_utils.py | 0* | 20 | 658 | Second home of duplicate-name `normalize_ascii` — merge-trap module |

(*fix-180d = 0 means no fix-keyword commits touched the file in the window, not "no bugs".)

msb2c/ files (e.g. msb2c/reranker/candidate_scorer.py, 28 churn) excluded: pain-list scope is the vihap package.
