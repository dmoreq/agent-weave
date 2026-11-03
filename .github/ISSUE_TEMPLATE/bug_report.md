---
name: Bug report
about: kg answered wrongly, crashed, or violated an invariant
labels: bug
---

<!-- Do not post secrets or private source. For vulnerabilities, follow SECURITY.md. -->

## Reproduction

<!-- Give a minimal shareable example, the working directory, and exact steps. -->

## Command and actual output

```
kg <subcommand> ...
```

<!-- Paste stdout, stderr, and the exit code. Redact sensitive content and mark redactions. -->

## Expected behavior

<!-- Explain the expected result. Link a public documentation section if applicable. -->

## Environment

- kg version (`kg --version`):
- Repository languages and approximate file count:
- OS and architecture:
- Installation method:

## Evidence

- `kg doctor` output (or its error if the index cannot be opened):
- If a resolution bug: the true callee (file:line) and the import style
  (direct / aliased / re-export / typed receiver / dynamic):

## Unexpected writes

<!-- Compare git status --porcelain before and after, if available. Do not reset existing work.
     Report new changes outside .kg/; existing changes are not evidence that kg wrote them.
     Git status alone cannot prove that every write stayed inside .kg/. -->

- Did the command change files outside `.kg/`? Give paths or state "not checked":
