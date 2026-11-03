# R14 — P8 value re-evaluation after remediation

- Code baseline: `bc292ba` (implementation through `cd00e5d`; docs/tests in
  `bc292ba`).
- Six independent Goldfish lenses: L1 correctness, L2 utility, L3 latency,
  L4 contract, L5 removal cost, L6 evidence integrity. L1-L5 voted; L6 did
  not. Reports: `reviews/R14-p8/reports/L1.md` … `L6.md`,
  `L4-verification.md`.
- Decision rule was pre-registered in `reviews/R14-p8/README.md` before the
  L5/L6 wave.

## TL;DR

- **No feature is removed.** L6 is **INSUFFICIENT for REMOVE**; no feature
  reached 4/5 REMOVE votes.
- **F4 and F7 are KEEP, with different cross-host strength.** F7 passes on all
  three hosts at n=5: grok -50.0%, agy -76.2%, zcode -57.1%, equal
  correctness. F4 passes on agy -66.7% and zcode -53.3% but is -16.7% on grok,
  below the 25% bar. F4 latency is fixed (97-144 ms vs 150 ms at 1.37M edges).
- **F6's R13 contract blocker is fixed**: `resolve` honors `--no-journal`,
  MCP rejects blank/non-string names, and journal counts reflect parsed names.
- **F8's 1M-edge test latency is fixed** for indexes with <=20,000 test
  symbols: zero-test 6.9-9.5 ms, 100-test 8-10 ms, 10,000-test 63-113 ms at
  1.37M edges. Above 20,000 tests the reverse BFS remains a stress case.
- **F7 has uniform n=5 value; F4 has n=5 value on two of three hosts. F1 is
  mixed; F8/F10 are negative in their only shapes; F2/F3/F5/F6/F9/F11 have no
  utility task.**
- **DEMOTE decisions applied** (owner, 2026-11-03): `kg api` routing (F10),
  search-filter claims (F6), six `--exclude-generated` flags (F11), and `dead`
  routing (F1). Code, MCP tools, tags, and `resolve` remain. Owner record:
  `reviews/R14-p8/owner-decision.md`.
- **REMOVE candidates** exist but are blocked: F2 `--context` (1/5) and F11
  filters (1/5). L6 blocks removal on the current battery.

## What changed since R13

| Item | R13 | R14 |
|---|---|---|
| F4/F8 1M-edge hub latency | Critical: 1,132 / 1,336 ms | Fixed: impact 97-144 ms, tests 6.9-113 ms |
| `impact --edges all` | 1,323-1,345 ms | 121.6-146.9 ms |
| `tests` non-hub, test-rich | 596-679 ms | 8-113 ms up to 20k tests |
| Module-level impact callers | silently dropped at depth>1 | fixed; guarded regression |
| `resolve --no-journal` | bypassed | fixed; no journal write |
| MCP resolve blank/non-string | accepted/empty success | tool error |
| Inheritance GT | 11 rows, same-file | 17 rows, 6 cross-file |
| Dead probe | claimed 0/50 | corrected 6/50 |
| API caveat | silent non-callable omission | disclosed |
| `api`/`dead`/MCP docs | stale claims | corrected |

## Findings that still matter

### Critical (evidence, not feature value)

1. **The utility battery is not REMOVE-grade.** n=2 per cell, one shape per
   feature, six features untested, all 30 valid treatment cells paid a failed
   `kg` command on non-git corpora, and the battery ran at `6dbe57a`. The
   measured savings are lower bounds; the negative F8/F10 deltas include a
   fixed handicap.
2. **No feature meets the pre-registered KEEP bar.** The bar requires n>=5;
   the battery has n=2. F4 is the only robust positive and needs replication.
3. **Test latency above 20,000 test symbols is unmeasured and still uses the
   reverse BFS.** The 1.37M gate is green for 0-10k tests; >20k is a stress
   case.
4. **Ambiguous call sites are invisible in `impact` and `unresolved`.**
   `impact` joins `symbols ds ON ds.id = e.dst_id`, so `AMBIGUOUS` edges with
   `dst_id NULL` never match a target file. A file with an ambiguous inbound
   call can report "0 impacting files" while `callers` shows candidates.
5. **`api` omits exported non-callable declarations.** TS type aliases/consts
   and Rust `pub const/static/type` are not indexed; the caveat now says so.
6. **`dead` has low precision on dynamic Python.** 6/50 sampled vihap
   candidates have zero external textual references; type/attribute references
   are known false positives.

### Important

- MCP optional arguments with wrong JSON types silently default instead of
  returning invalid-params.
- README previously claimed byte-identical MCP errors; now says success text
  matches and errors append CLI stderr detail.
- `stats`/`hotspots`/`report` journal `result_count=0`; `kg usage` reports
  them as empty.
- GT precision for tests is a name-shape check, not reachability; imports use
  a leaf-in-block proxy; inheritance is recall-only.
- The dead probe oracle is not checked in; the report states the indexed-file
  method.

## Verdict by feature

See `reviews/R14-p8/decision-matrix.md`. Summary:

- KEEP: **F4, F7** (local n=5 replication; owner sign-off pending).
- IMPROVE: F1, F2, F3, F5, F6, F8, F9, F11 (tag).
- DEMOTE recommended: F10 (routing), F6 search filters, F11 filters, F1
  `dead` claims.
- OWNER: F2 flag demotion.
- REMOVE: none approved; F2 1/5, F11 filters 1/5; L6 INSUFFICIENT.

## Next steps

1. F4/F7 cleared the local n=5 replication; run the same protocol on grok/agy
   before a cross-host KEEP claim. Do not promote H1/F1 yet.
3. Add one utility task each for F2/F3/F5/F6/F9/F11 before any routing claim.
4. Fix ambiguous-edge visibility in `impact`/`unresolved` or document it in
   the user-facing caveat.
5. Decide the DEMOTE packets with the owner; no schema drop is required.
6. Do not run a REMOVE decision until L6's checklist is met.
