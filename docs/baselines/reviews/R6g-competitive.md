# R6g — Competitive positioning (Goldfish, 2026-11-02)

- Verdict: **USE-ALTERNATIVES** for generic daily use; kg is an optional
  supplement under narrow conditions. Independent runs on Django 5.x and
  vscode `src/` included.

| workload | winner | why |
|---|---|---|
| Small repo one-off lookup | ripgrep / host Grep | zero setup; exact match; kg's own search A/B failed economics |
| Large-repo call navigation | LSP (Claude Code LSP plugin) / Serena; kg only for shortest-path | LSP resolves typed/method refs; Django `Model.save` → kg 0 callers vs 15 grep sites |
| Refactor/rename | Serena / IDE LSP / ast-grep | kg is read-only |
| Cross-repo/org search | Sourcegraph MCP / GitHub search | kg is per-repo; Sourcegraph $16k/yr |
| Semantic "where is X" | code-graph-rag / Sourcegraph Deep Search / Cursor agent | kg is lexical/static |
| Offline audits (dead/clones/complexity) | Vulture/Knip, jscpd, lizard/radon, ast-grep | specialists are stronger (kg dead false-positive on `AppConfig.get_model`) |

kg unique-as-packaged (not unique in principle): `path` (zero-LLM shortest
route), `impact <file>`, `usage --verify`, `unresolved`. Commodity: callers/
callees/search/symbols/dead/clones/hotspots.

Independent measurements (HEAD binary): Django 3,050 files → 47,712 symbols,
103,037 edges, 118,793 unresolved (71.7%), build 3.72 s / 223 MB / 58 MB DB,
queries <10 ms. vscode 11,275 files → 139,461 symbols, 722,023 edges, 850,992
unresolved (62.9%), build 24.6 s / 1.04 GB / 382 MB DB, queries <10 ms. Both
exceed the spec's 50 MB DB / 100 MB RSS targets. G2 self GT fails on HEAD
(1/10, line shifts); GT gates are not wired into CI.

Adoption recommendation: do not adopt kg as the primary daily tool. Use host
LSP + Serena + ast-grep + rg; specialists for audits. Keep kg only if: you are
the copyright holder (current license forbids anyone else); repo ≤ ~3,000
files / ≤ 50 MB index; the frequent questions are `path`/file-impact; the
kg-first skill is installed; and you verify candidate output. Revisit after a
published ≥10k-file benchmark, per-language method-level recall, fresh GT in
CI, corrected claims, a usable license, and a release.
