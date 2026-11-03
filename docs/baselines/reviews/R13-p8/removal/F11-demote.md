# F11 — DEMOTE packet (owner decision)

L5: DEMOTE filters; L4/L1: IMPROVE coverage; L2/L3: DEFER; L6 insufficient
for REMOVE. Recommended: keep `files.generated`, `stats generated=N`, and
`files` tag; remove `--exclude-generated` from routing until controls exist.

Required before promotion:
- Add committed positive/negative controls: `gen/`, bare `DO NOT EDIT`,
  lowercase/uppercase suffix behavior.
- Add real generated corpus.
- Assert exclusion on dead/clones/hotspots/complexity/api.
- Decide whether report scoped counts include generated.

Do not drop `files.generated` post-release; additive-only schema.
