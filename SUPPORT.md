# Support

## Getting help

1. Run `kg doctor` — it reports integrity, schema/grammar versions, parse
   coverage, staleness, orphans, and lock state. Most issues are visible there.
2. Check the response contract in `README.md` (exit codes 0-7 and the
   stderr notes) — errors are typed, not stack traces.
3. Consult `docs/baselines/transcripts/SUMMARY.md` for gate evidence and
   known limitations (workspace aliases, JSX usage, untyped receivers).

## Common issues

| symptom | cause | fix |
|---------|-------|-----|
| `kg: no-results: no .kg/index.db under ...` | never built | `kg build` |
| `kg: schema-mismatch: index schema vN, binary expects vM` | binary upgraded | `kg build` (full re-index) |
| `kg: parse-quality: N% of M files failed to parse` | broken sources exceed 5% | fix the files, or `--force` / `--max-parse-error-rate` |
| `kg: locked: build in progress (pid N)` | concurrent build | wait for the pid to exit |
| stale header on every query | sources changed after build | `kg build` |
| external calls missing (react, @finguard/*) | workspace aliases out of scope | documented limitation; grep those call sites |

## Scope boundaries

kg resolves statically only; dynamic dispatch (getattr, duck typing, monkey
patching) is a documented non-goal — grep remains the fallback there, and the
empty-result message says so.
