# Support

## Getting help

1. Record `kg --version`, the command, stdout, stderr, and exit code.
2. Run `kg doctor` for index integrity, versions, parse coverage, staleness,
   and leftover files. It requires an existing, compatible index; if it cannot
   open the index, keep that error and follow the table below.
3. Check the [README](README.md) for output conventions and exit codes.
   Use `kg <subcommand> --help` to check that command's flags.
4. For a reproducible bug, use the repository's Bug report issue template.
   Include a small example that you can share publicly, not a private repository.

Remove secrets and sensitive paths before sharing output. For vulnerabilities,
follow [SECURITY.md](SECURITY.md); do not post security details in a public issue.
This page does not confirm whether the repository host's reporting features are enabled.

## Common issues

| Symptom | Likely cause | Next step |
|---|---|---|
| `kg: no-results: no .kg/index.db under ...` | No index at the selected root | Run `kg build`; use `--root /path/to/repo` if root discovery selected another repository. |
| `kg: schema-mismatch: index schema vN, binary expects vM` | Index and binary versions differ | Run `kg build` to rebuild with the current binary. |
| `kg: parse-quality: N% of M files failed to parse` | Parse-error fraction exceeds the threshold, which defaults to `0.05` | Fix source syntax first. See the threshold guidance below if partial indexing is acceptable. |
| `kg: locked: build in progress (pid N)` | Another build holds the lock | Wait for that build to finish. Do not remove an active build's lock. |
| Stale warning | Indexed sources changed | Run `kg build` before relying on query results. |
| Missing calls or empty results | Unresolved imports, unsupported syntax, dynamic behavior, or a stale index | Rebuild, inspect `kg unresolved --names`, and search the source directly. |

### Parse-quality failures

`kg build --max-parse-error-rate 0.10` explicitly allows up to a `0.10` fraction
(10%) of files with parse errors. The value must be finite and between `0` and
`1`, inclusive. Raising it accepts less complete results; it does not repair parsing.

`kg build --force` re-extracts every file and **bypasses parse-quality checks**,
even when `--max-parse-error-rate` is supplied. Use it only when you accept that
the resulting index may omit symbols or relationships from files with parse errors.
Prefer fixing sources or setting an explicit acceptable threshold without `--force`.

After a successful build, `kg doctor` reports the stored parse-error count.
After a failed build, it inspects the previous index if one exists, not the failed attempt.

## Scope boundaries

kg resolves static relationships, not every runtime call. Dynamic dispatch,
Python `getattr`, duck typing, monkey patching, untyped receivers, and workspace
import aliases can leave relationships unresolved. Use grep or another source
search tool to investigate missing results.

An empty query is not proof that a symbol has no callers. Test-reachability and
dead-code results are candidates for inspection, not guarantees of coverage or
safe deletion. Include the expected `file:line` and import style when reporting
a resolution problem.
