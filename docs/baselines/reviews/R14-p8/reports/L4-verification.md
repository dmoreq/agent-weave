# R14 L4 Verification: Impact Depth and Resolve Contracts

## TL;DR

C1 is **FIXED** at `62b781d`: a module-level caller at depth 2 is returned in both edge modes, with correct `showing 1 of 2` totals. All three R13 blockers **PASS**. No source edit was made by this verifier; the reviewed revision is `62b781d7ce3f4c953c0f6c7507e8bc6e27d32be1` (fix commit `d714f29`).

## Environment and Fixture

Release binary rebuilt at the reviewed revision:

```
cargo build --release --locked
Finished `release` profile [optimized] target(s) in 12.08s
```

Clean fixture under `/tmp/kg-r14-l4-verify` (outside the repository):

| File | Content |
|---|---|
| `m1.py` | `def f():` / `    pass` |
| `mid.py` | `from m1 import f` / blank / `def g():` / `    f()` |
| `top.py` | `from mid import g` / blank / `g()` |

Build command and output:

```
kg --root /tmp/kg-r14-l4-verify build --force
kg: built index: files=3 symbols=5 edges=8 parse_errors=0 unresolved=0 carried=0 wall=0.03s (walk=0.01s parse=0.00s extract=0.00s db=0.02s)
```

## C1: Depth-2 Module-Level Caller

The module-level call site `g()` in `top.py` creates a `calls` edge whose source symbol is `top` with kind `module`. Both depth-2 invocations now include it:

```
$ kg --root /tmp/kg-r14-l4-verify impact m1.py --depth 2
kg: impact — resolved static call edges only; dynamic dispatch may hide dependents
kg: depth cap 2 reached; deeper dependents may exist
mid.py:4
top.py:3

$ kg --root /tmp/kg-r14-l4-verify impact m1.py --depth 2 --edges all
kg: impact — resolved static edges only; unresolved imports/inherits are omitted
kg: depth cap 2 reached; deeper dependents may exist
mid.py:1
top.py:1
```

`--limit 1` exercises the `showing N of M` line; both modes report the full two-file universe:

```
$ kg ... impact m1.py --depth 2 --limit 1
mid.py:4
showing 1 of 2
[kg] capped at 1 results (2 total)

$ kg ... impact m1.py --depth 2 --edges all --limit 1
mid.py:1
showing 1 of 2
```

Meta counters exist after `build --force` and match a direct SQL recomputation:

| Key | Stored | Recompute |
|---|---:|---:|
| `edge_calls_src_files` | 2 | 2 |
| `edge_imports_src_files` | 2 | 2 |
| `edge_inherits_src_files` | 0 | 0 |
| `edge_all_src_files` | 3 | 3 |

Recompute SQL:

```sql
SELECT COUNT(DISTINCT CASE WHEN e.kind='calls' THEN s.file_path END),
       COUNT(DISTINCT CASE WHEN e.kind='imports' THEN s.file_path END),
       COUNT(DISTINCT CASE WHEN e.kind='inherits' THEN s.file_path END),
       COUNT(DISTINCT s.file_path)
FROM edges e JOIN symbols s ON s.id = e.src_id;
```

### Fallback on an index without the counters

A second copy (`/tmp/kg-r14-l4-verify-old`) was built with `--force`, then all four `edge_*_src_files` meta rows were deleted. The edge-scan fallback returned the same rows and totals:

```
$ sqlite3 ... "DELETE FROM meta WHERE key IN (...);"
$ sqlite3 ... "SELECT count(*) ..." -> 0
$ kg ... impact m1.py --depth 2
mid.py:4
top.py:3
$ kg ... impact m1.py --depth 2 --edges all
mid.py:1
top.py:1
```

### Root-cause reproduction

Setting `edge_calls_src_files` to the old callable-only value reproduces C1 exactly:

```
$ sqlite3 ... "INSERT OR REPLACE INTO meta(key,value) VALUES('edge_calls_src_files','1');"
$ kg ... impact m1.py --depth 2
mid.py:4        # top.py absent
$ sqlite3 ... "SELECT COUNT(DISTINCT s.file_path) ... s.kind IN ('function','method')"
1
```

The fix changes the calls universe from 1 callable file to 2 distinct call-edge source files, which lets the depth loop reach `top.py`. Verdict: **FIXED**.

## R13 Blockers

| Blocker | Probe | Observed | Status |
|---|---|---|---|
| `resolve --no-journal` writes no journal | `printf 'f\n' \| kg --root ... --no-journal resolve --stdin` on an index built without a journal | No `.kg/usage.log` created | PASS |
| `--no-journal` preserves an existing journal | Journaled resolve first, `cmp` before/after a no-journal run | File byte-identical | PASS |
| MCP resolve rejects blank names | `{"names":[""]}` and `{"names":["# c","  "]}` | `isError:true`, `kg: no-results: no names on stdin` | PASS |
| MCP resolve rejects non-string names | `{"names":["f",5]}` and `{"names":[null]}` | `isError:true`, `kg: usage: resolve names must be strings` | PASS |
| MCP resolve rejects a non-array | `{"names":"f"}` | JSON-RPC `-32602`, `missing required argument 'names' for tool 'resolve'` | PASS |
| MCP resolve accepts valid names | `{"names":["f"]}` | `isError:false`, `found f m1.py:1 m1.f` | PASS |
| Journal count reflects parsed names | `printf 'f\n# comment\n\n' \| kg resolve --stdin` | `"tool":"resolve","args":{"count":"1","candidate_limit":"50"}` | PASS |

Blank and comment entries are filtered before resolution, so the journal count is 1, not 3. Recorded journal line:

```json
{"ts":1793690900,"surface":"cli","tool":"resolve","args":{"count":"1","candidate_limit":"50"},"result_count":1,"shown":1,"total":1,"ambiguous":0,"exit_code":0,"latency_us":1255,"stale":false,"kg_version":"0.2.1","session":"","arm":""}
```

## Targeted Tests

All three named tests pass. `cargo test` accepts one filter positional, so each exact name was run as its own command:

| Command | Result |
|---|---|
| `cargo test --locked --test cli impact_depth_two_includes_module_level_caller` | 1 passed, 0 failed |
| `cargo test --locked --test cli resolve_honors_no_journal_and_records_parsed_count` | 1 passed, 0 failed |
| `cargo test --locked --test mcp resolve_empty_names_and_non_strings_are_tool_errors` | 1 passed, 0 failed |

## Residual Observations

Neither item affects correctness or blocks release.

| Observation | Effect |
|---|---|
| `edge_all_src_files` is 3, while the `--edges all` universe (calls/imports/inherits) has 2 distinct source files. The counter SQL counts every edge kind, including `contains` from `m1` to `f`. | Safe upper bound. It can only defer the early-stop, never drop a row; `showing` totals stay 2. The calls counter is exact. |
| `impact --depth 2` prints `depth cap 2 reached; deeper dependents may exist` even though `top.py` has no further callers. | Pre-existing, hedged wording; no result loss. Depth 3 terminates without the notice. |

## Verdict

C1 **FIXED**. R13 blocker status: **3 of 3 PASS**. No residual mismatch changes any returned row or contract count.
