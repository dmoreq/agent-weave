# `dead` candidate precision probe (2026-11-03)

Question: does `dead --exclude-tests` produce actionable candidates on a
real Python corpus?

## Method

- Corpus: vihap working tree, rebuilt with the current release binary
  (`files=252 symbols=3291 edges=11294 unresolved=4984`).
- Command: `kg --root <vihap> dead --exclude-tests --limit 50`.
- Oracle: locate each candidate with Python `ast` at the emitted line, then
  count textual occurrences of its bare name in every `.py` file except the
  definition line.
- A candidate with zero external textual occurrences is a safe dead
  candidate for this probe. The oracle over-approximates references, so a
  non-zero count does not prove the symbol is live.

## Result

| metric | value |
|---|---:|
| candidates sampled | 50 |
| candidates with zero external textual occurrences | **0** |
| candidates with 1 external occurrence | 7 |
| candidates with >1 external occurrence | 43 |

Examples:

| candidate | kind | external refs | why it is not dead |
|---|---|---:|---|
| `scripts/compare_parse_output.py:73 cmd_dump` | function | 1 | argparse `set_defaults(func=cmd_dump)` callback |
| `vihap/cleaners/abbreviation_engine.py:95 default_confidence` | method | 7 | attribute reference passed as a callable value |
| `vihap/cleaners/base.py:16 __call__` | method | 852 | dynamic protocol dispatch |
| `vihap/cleaners/spelling_corrector.py:62 clean` | method | 3834 | dynamic dispatch / common name |

The sample is sorted by path, not by score; it is not a precision estimate for
all 228 candidates. It does show that the top of the output contains known
false-positive classes.

## Interpretation

- `dead` currently has no demonstrated precision on dynamic Python. It is a
  candidate generator whose caveat names the main false-positive classes.
- Type references (Rust structs used as types) and attribute references
  (methods passed as callables) emit no call edge, so `dead` flags them.
- A future improvement would need type-reference and attribute-reference
  edges; a narrower option is to demote `dead` from advertised workflows.
- No REMOVE decision is made here; R14 owns that judgement.

## Reproduction

```bash
target/release/kg --root <vihap> build --force
target/release/kg --root <vihap> --no-journal dead --exclude-tests --limit 50
```

The AST/textual oracle is a local probe, not a checked-in gate.
