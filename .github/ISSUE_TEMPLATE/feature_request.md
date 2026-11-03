---
name: Feature request
about: A new capability for kg
labels: enhancement
---

## Problem

<!-- What code-navigation question can you not answer today? Give a shareable example;
     no private session transcript is required. -->

## Proposed surface

<!-- Suggest a command or flags and example output, if useful. A complete design is not required. -->

## Compatibility and safety

<!-- Read CONTRIBUTING.md and the linked specification. Note known conflicts or
     questions below; leave unknown implementation details for discussion. -->

- Runtime safety: no daemon, network access, LLM calls, writes outside `.kg/`,
  or agent-configuration writes.
- Scope: no verbatim source bodies or promises to resolve dynamic dispatch.
- Compatibility: schema changes must be additive. Output-format or exit-code
  changes require a specification contract-bump note before implementation.
- Verification: what example or measurement would demonstrate that this helps?

Known conflicts or open questions:

## Alternatives tried

<!-- grep/ctags/ast-grep or an existing kg command. -->
