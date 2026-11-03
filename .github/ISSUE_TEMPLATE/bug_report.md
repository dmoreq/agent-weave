---
name: Bug report
about: aw answered wrongly, crashed, or violated an invariant
labels: bug
---

<!-- Do not post secrets or private source. For vulnerabilities, follow SECURITY.md. -->

## Reproduction

<!-- Give a minimal shareable example, the working directory, and exact steps. -->

## Command and actual output

```
aw <subcommand> ...
```

<!-- Paste stdout, stderr, and the exit code. Redact sensitive content and mark redactions. -->

## Expected behavior

<!-- Explain the expected result. Link a public documentation section if applicable. -->

## Environment

- aw version (`aw --version`):
- Repository languages and approximate file count:
- OS and architecture:
- Installation method:

## Evidence

- `aw doctor` output (or its error if the index cannot be opened):
- If a resolution bug: the true callee (file:line) and the import style
  (direct / aliased / re-export / typed receiver / dynamic):

## Unexpected writes

<!-- Compare git status --porcelain before and after, if available. Do not reset existing work.
     Report new changes outside .aw/; existing changes are not evidence that aw wrote them.
     Git status alone cannot prove that every write stayed inside .aw/. -->

- Did the command change files outside `.aw/`? Give paths or state "not checked":
