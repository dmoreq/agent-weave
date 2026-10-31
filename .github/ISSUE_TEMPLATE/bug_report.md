---
name: Bug report
about: kg answered wrongly, crashed, or violated an invariant
labels: bug
---

## Command + actual output

```
kg <subcommand> ...        # paste stderr/stdout verbatim
```

## Expected

<!-- What the response contract promises (README.md "Response contract"). -->

## Environment

- kg version (`kg --version`):
- repo + rough size:
- OS:

## Evidence

- `kg doctor` output:
- If a resolution bug: the true callee (file:line) and the import style
  (direct / aliased / re-export / typed receiver / dynamic):

## Invariant check

- [ ] `git status --porcelain` shows nothing outside `.kg/` after the command
