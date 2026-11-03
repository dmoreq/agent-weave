# P9 protocol amendment — 2026-11-03

Status: withdrawn 2026-11-03 after the user authorized the source change.
`aw callers --depth` is implemented; this file remains for audit only.

## Decision

Adopt the verified current-CLI route instead of changing `src/*.rs`, the schema,
or the CLI. The handoff forbids source and schema changes. The single-command
T1 route is not executable in `aw 0.3.0`; A2 is resolved without an amendment.

Scope of this amendment: the T1 command shape (single `--depth` invocation
versus iterative `callers`) and the T1 target symbol. `FEATURE_TOOL["T1"]`
remains `callers`, and A2 keeps the handoff's interfaces-and-aliases oracle.

## Stream 3 — T1 acceptance

**Original:** `aw callers vihap.cleaners.text_utils.exact_text_key --depth 3`
returns at least 2 items.

**Amended:** follow `aw callers` iteratively for up to three hops from
`vihap.cleaners.vocab_corrector.load_lexicon`, then resolve definition lines
with `aw symbols tests/test_vocab_corrector.py`. The route returns exactly two
test functions at distances two and three.

Evidence:

- `aw callers --help` has no `--depth` flag; `callers` returns direct call edges
  only.
- `exact_text_key` has one direct test and zero indirect tests, so it cannot
  supply 2+ transitive test callers at any depth.
- Hop chain: `load_lexicon` -> `QueryCorrector.from_file` -> the direct test
  `test_missing_vocab_file_raises` and `get_default_query_corrector` ->
  `test_get_default_query_corrector_singleton`.
- `aw symbols tests/test_vocab_corrector.py` gives definition lines 124 and
  166, matching the frozen oracle.

`FEATURE_TOOL["T1"]` remains `callers`, matching the handoff. The target symbol
changes from `exact_text_key` to `load_lexicon` because only the latter has
2+ transitive test callers.

## Stream 2 — A2 expected items

Resolved without an amendment. The oracle now contains the handoff's full set:
15 exported interfaces plus 8 exported type aliases (23 items).

Evidence:

- `aw symbols packages/core-ledger/src/types.ts` reports 15 `interface` rows
  and one `module` row.
- The index does not extract TypeScript type aliases, and `aw symbols` has no
  `--filter exported` flag.
- The A2 prompt runs `aw symbols` for interfaces, then reads the file only to
  collect `export type <name>` declarations. The Trust Contract permits a
  source read for a field the output does not contain.
- No `src/*.rs` or schema change is required.

## Stream 5 — FEATURE_TOOL

`FEATURE_TOOL = {"I1": "imports", "T1": "callers", "H1": "implementers",
"D1": "impact", "A2": "symbols"}` in `run_p9.py`. This matches the handoff and
keeps control-arm violations meaningful. No FEATURE_TOOL amendment is needed.

## AGENTS.md §4 surface sync

The Trust Contract changes the agent skill, not a subcommand, flag, output
format, or crate name. The five surfaces were checked anyway:

| Surface | Files | Action |
|---|---|---|
| CLI | `src/cli.rs`, `src/main.rs` | No change. No subcommand, flag, output format, or crate name changed. |
| MCP | `src/mcp.rs`, `tests/mcp.rs` | No change. Existing descriptions already label `tests`, `dead`, and `unresolved` as candidates (`src/mcp.rs:52`, `:56`, `:58`). |
| Agent skill | `skills/aw-first/SKILL.md` | Trust Contract added after the routing table. |
| Public docs | `README.md`, `docs/agent-integration.md`, `docs/cli-reference.md`, `docs/usage.md`, `CHANGELOG.md` | Trust Contract summarized in each. `docs/getting-started.md` does not describe agent routing, so it is unchanged. |
| Packaging | `Cargo.toml`, `tests/scripts/allowlist.txt`, `.github/workflows/` | No change. No crate, binary name, or packaging change. |

No `src/*.rs` or schema file is modified by this amendment.

## Verification

- `git diff --check`
- `bash tests/scripts/write_path_check.sh`
- Markdown links in the changed public docs resolve.
- Six validation cells (I1, T1, A2; treatment and control): 6/6 exact match,
  0 violations.
- T1 treatment: 4 `callers` + 1 `symbols`, 2/2; control 7 tools, 2/2.
- A2 treatment: 1 `symbols` + 1 source read, 23/23; control 23/23.
