# P8 dependency / removal graph

```
F7 imports ──> impact --edges imports
F7 imports ──> dead (inbound imports suppress dead)
F1 inherits ─> impact --edges inherits
F1 inherits ─> dead (inbound inherits suppress dead)
F1/F7      ─> unresolved kind=import/inherit
F1/F7/F8   ─> gap_files_for_new_names healing
F9 metrics ─> complexity, symbols --metrics, schema v6/v7, meta.callable_count
F10 exports ─> api, schema symbols.exported, dead kind set
F11 generated ─> stats/files tags and --exclude-generated filters
F3 --path ─> dead/clones/hotspots/unresolved/report/complexity/api/files/symbols
F2 context ─> impact --context (shares F4 BFS)
F4 impact ─> imports/inherits edge kinds
F5 unresolved ─> extraction/resolution across all languages
F6 resolve/search ─> target_ids, search filters
```

Schema removal rule: post-release columns are not dropped in place; feature
withdrawal removes command/flags/MCP/journal/payload/docs/skills/tests first.
