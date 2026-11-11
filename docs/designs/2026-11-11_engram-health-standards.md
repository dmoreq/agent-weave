# Engram Health Standards and Daily Audit

- Author: Quy Doan
- Status: Proposed (Gate 2 findings resolved; implementation pending)
- Date: 2026-11-11
- Predecessor: `docs/designs/2026-11-11_concepts-brief_engram-health-standards.md`
- Revision 3: seven-lens brainstorm plus Gate 2 five-lens review; all Critical/Important findings resolved below.

## Overview

`eg audit` is a read-only health command for engram. It evaluates a versioned registry of measurements over the retained usage journal, the local index, and project-memory stores. It prints a redacted human report or a stable JSON document. A host automation runs it daily, reports anomalies, and writes one path-free memory note per day.

This spec defines L1 online standards. L2 release gates (`bench/run.py`, recall/precision scorers, memory oracle) and L3 deep evaluation (`docs/evaluation.md`) remain separate. Daily audit does not replace them.

## The Problem

Engram produces telemetry, but it does not define what "healthy" or "effective" means. `eg usage` folds an unbounded retained window and prints text; `eg doctor` is manual, repo-local, and mutates journals by pruning; the dashboard is interactive. Metric definitions already diverge: the CLI savings fold includes exit-1/2 calls' per-call overhead, the dashboard prices only exit 0, and the dashboard graph class includes `build`/`symbols`. A silent index corruption, broken memory refs, or collapse in adoption can persist until someone inspects the machine.

The journal is not a trustworthy substrate by itself: it is size-rotated, pruned by other commands, forgeable by local processes, and blind to journal opt-out and no-root invocations. The standards must expose those limits instead of hiding them.

## Goals

- One normative definition per measurement, stable IDs, published thresholds, and a versioned contract.
- A strictly read-only command: no journal, index, memory, dashboard, or file mutation.
- Explicit coverage and baseline eligibility; missing data never becomes `pass`.
- Redacted output safe for a terminal, an agent context, and a global memory note.
- Hard safety floors for integrity and operational failures; baseline-relative warnings for effectiveness.
- A daily automation that detects anomalies, suggests scope-correct investigation, and writes a fixed daily note.
- No new dependency, daemon, network path, database schema, or write outside existing confinement roots.

## Non-Goals

- Replacing `eg usage`, `eg doctor`, `eg stats`, or the dashboard.
- Changing journal schema, rotation, pruning, live-repo rules, or memory schema.
- Measuring real host/model token consumption; savings remain estimates.
- Proving reuse adoption or post-edit behavior.
- Running release benchmarks daily.
- Adding an MCP tool, dashboard route, webhook, email, or remote alert.
- Auto-fixing findings, editing agent configuration, or mutating the index.
- Version bump, release, or tag in this task.

## Open Questions

None. Owner decisions on 2026-11-11:

- CLI-only, repo-local by default, `--global` opt-in.
- Audit navigation uses six read-only query tools; dashboard `classify()` is unchanged.
- Redacted output by default; `--reveal-paths` is human-only.
- Persist one path-free note per day at `health/YYYY-MM-DD`.

## Critical User Journeys

### CUJ 1: Developer checks the current repository

- Given: a repository with an index and journal entries
- When: the developer runs `eg audit`
- Then: the header states `health=... data=... scope=local`, the requested and observed windows, and check counts; only non-pass checks are shown; the repository is `repo-<hash16>` unless `--reveal-paths`; exit is 0 unless `--fail-on` is breached.

### CUJ 2: Machine-readable audit

- Given: any supported state, including no journal
- When: the caller runs `eg audit --json`
- Then: stdout contains exactly one JSON object with `schema_version: 1`, `standards_version: 1`, `status`, `data_status`, `coverage`, and stable checks; stderr contains only fixed diagnostic codes; JSON contains no absolute path, repository name, journal argument, memory key, or raw error text; a no-data run still emits JSON and exits 1.

### CUJ 3: Daily host automation

- Given: the installed binary and the ZCode schedule
- When: the job runs at 09:00 local
- Then: it checks yesterday's note exists, runs `eg audit --global --json` (default 7-day window and 7-day baseline), reports non-pass checks with scope-correct actions, and writes today's fixed-template note with journaling disabled.

### CUJ 4: Degraded source

- Given: a corrupt index, a missing index, an unreadable memory store, a tracked `.engram`, or a truncated journal
- When: audit runs
- Then: each source contributes an explicit check result; the report completes; no raw SQL, path, or OS error appears in stdout or JSON; stderr is a fixed code.

### CUJ 5: Insufficient history

- Given: a fresh journal below the baseline minimum
- When: audit evaluates baseline-relative checks
- Then: those checks are `insufficient` or `skipped`; hard floors still run; health status is computed only from evaluable checks; `data_status` reports the limitation.

## Technical Plan

```
CLI args -> resolve scope (local root or global resolver)
         -> read each journal generation separately (absent/read/error, 16 MiB cap)
         -> validate entries, live filter, split W and B, count exclusions
         -> one-pass metrics fold (per-tool, per-family, latencies, priced savings)
         -> index snapshots for active repos (read-only, non-aborting, capped)
         -> count-only memory snapshots (read-only, capped, version-gated)
         -> 15 checks -> health status + data status -> human or JSON
```

No HTTP server, no MCP server, no pruning, no rotation, no file writes. The command is excluded from journaling.

### Command contract

```
eg audit [--json] [--global] [--days N] [--baseline-days N]
         [--all-checks] [--reveal-paths] [--fail-on warn|critical]
```

| Flag | Default | Meaning |
|---|---|---|
| `--json` | off | One JSON object on stdout; fixed diagnostic codes on stderr |
| `--global` | off | Aggregate the global journal across live repositories; conflicts with `--root` |
| `--days N` | 7 | Evaluation window; integer 1–90 |
| `--baseline-days N` | 7 | Baseline immediately before the window; integer 3–90 |
| `--all-checks` | off | Human output includes passing checks; conflicts with `--json` |
| `--reveal-paths` | off | Human output replaces aliases with paths; conflicts with `--json` |
| `--fail-on warn\|critical` | off | Exit 1 when health status meets or exceeds the level |

`--baseline-days` below 3 is a usage error because baseline sufficiency requires 3 active days. `--global` with an explicit `--root` is a usage error, not a silent override.

Scope matrix:

| Mode | Journal | Index | Memory | Repo identity |
|---|---|---|---|---|
| local (default) | `<root>/.engram/usage.log` + `.1`; entries whose canonical `repo` equals root | current root only | current root's `.engram/memory.db` only | current root |
| global (`--global`) | `~/.engram/usage.log` + `.1` via `db::global_usage_log_path()` | repos active in `W ∪ B`, sorted by canonical path, cap 50 | global store plus active repos' local stores, cap 50 | canonical absolute path |

Dual-written lines are never summed: each mode reads one log family.

### Exit and stream contract

| Code | stdout | stderr | Condition |
|---|---|---|---|
| 0 | report | fixed diagnostics only | Audit completed; health may be warn/critical |
| 1 | report | fixed diagnostics only | No valid entries in W or B, or `--fail-on` threshold met |
| 2 | empty | clap/usage message | Invalid flag, range, or conflicting flags |
| 6 | empty | `eg: audit: io-error` | A selected journal generation exists but cannot be read |

Index and memory failures are check results, not process exits. A panic is 6 by the global panic boundary. The automation reads `status` and `data_status` from JSON; it treats missing/invalid JSON, exit 2, exit 6, or any code outside {0,1} as transport failure.

### Window, baseline, and clock

- `now = unix_now()`; tests inject a fixed clock into `collect()`.
- `W = [now - days*86400, now + 1)`; `B = [now - (days + baseline_days)*86400, now - days*86400)`.
- Each valid entry belongs to one range by `ts`; missing/out-of-range `ts` is counted and excluded.
- `active_day` is the UTC date of `ts`.
- Baseline sufficiency: `B` has ≥20 exit-0 calls, ≥3 active days, and `coverage.oldest_ts <= B.start_ts` (the baseline is not clipped by rotation). If false, baseline-relative terms are skipped; hard floors still apply.
- Baseline comparisons require the baseline value >0 and the check's own minimum sample. A zero baseline never triggers a warn.
- Baseline terms can raise `pass` to `warn`; `critical` requires an absolute floor.
- No backfill, no log rewrite.

### Coverage contract

```json
"coverage": {
  "window_entries": 120,
  "baseline_entries": 800,
  "oldest_ts": 1794000000,
  "newest_ts": 1794370000,
  "malformed_lines": 0,
  "invalid_lines": 0,
  "legacy_lines": 0,
  "dropped_dead_repos": 0,
  "future_ts_lines": 0,
  "truncated": false,
  "repos_active": 3,
  "repos_checked": 3,
  "repos_truncated": false,
  "stores_checked": 2,
  "stores_truncated": false,
  "generations": [
    {"name": "usage.log.1", "status": "read", "bytes": 126000, "truncated": false},
    {"name": "usage.log", "status": "read", "bytes": 28000, "truncated": false}
  ]
}
```

`malformed_lines` counts JSON parse failures or missing `tool`. `invalid_lines` counts wrong types/ranges for `exit_code`, `ts`, `latency_us`, `result_count`, or `shown`. One trailing unterminated line in the current generation is ignored. A non-`ENOENT` read error on a generation is exit 6 and no JSON. No coverage percentage is emitted; the denominator is unobservable.

### Status model

- Check status: `pass`, `warn`, `critical`, `insufficient`, `skipped`.
- `insufficient`: data should exist but the minimum sample was not met.
- `skipped`: the check does not apply (for example, no memory refs or no eligible tool).
- Overall `status`: `critical > warn > pass`, computed from `warn`/`critical`/`pass` checks only.
- Overall `data_status`: `sufficient`, `partial` (any `insufficient` or `skipped`), or `insufficient` (no valid entries in W+B).
- A no-data run has `status: "pass"`, `data_status: "insufficient"`, no false health claim, and exit 1.

### Check registry (15 stable IDs)

| ID | Dimension | Definition | Hard floors | Baseline | Minimum |
|---|---|---|---|---|---|
| `index.integrity` | correctness | Every checked index: readable, integrity ok, schema = compiled version, not tracked, not symlinked | corrupt/schema/symlink/tracked: critical; missing: warn; zero files: warn | none | 1 repo |
| `memory.integrity` | correctness | Every selected store: readable, version-gated, bounded, integrity ok | unreadable/corrupt/schema/symlink/tracked: critical | none | absent store = pass |
| `ops.errors` | correctness | Exit 3/5/6/7 calls in W; exit 4 counted separately; total outage when calls ≥10 and successes =0 | 3/5/6/7 >0 or outage: critical; exit 4 >3: warn | none | none |
| `index.parse_errors` | freshness | Per repo `parse_errors / files`; requires ≥20 files and ≥2 errors to warn | warn rate >5%; critical rate >10% and errors ≥5 | 1.25×, warn only | 20 files |
| `index.staleness` | freshness | Per repo `changed / indexed`; requires ≥20 files and ≥2 changed | warn >10%; critical >25% and changed ≥5 | 1.5×, warn only | 20 files |
| `telemetry.malformed` | telemetry | `(malformed + invalid) / non-empty lines` across generations | warn >0.5%, critical >2% | none | 20 lines |
| `telemetry.continuity` | telemetry | Newest entry age; blackout only when B had ≥3 active days | warn >96h; critical >336h | none | W or B has entries |
| `performance.p95` | performance | p95 per tool with ≥20 calls | nav 50/150 ms; search 100/300; symbols 100/300; mem read 50/150; mem write 100/300; build 5000/15000; reuse 5000/15000 | 2×, warn only | 20 calls/tool |
| `memory.refs` | memory | Counts-only current, unexpired refs: broken and modified shares | broken ≥2 and >5%: warn; ≥5 and >20%: critical; modified >30%: warn | 1.5×, warn only | 5 checkable refs |
| `adoption.navigation` | effectiveness | Successful calls to six navigation tools / all successful calls in W | warn <10% | 0.5×, warn only | 20 successes |
| `adoption.pillars` | effectiveness | Last 7 days: navigation, prewrite (`search`/`reuse`), memory read (`mem_query`/`mem_get`) each ≥1 successful call | missing pillar: warn | none | 20 calls |
| `quality.empty_results` | effectiveness | Successful calls with `result_count == 0` per family | nav >40%, search >30%, mem query >40% | 1.5×, warn only | 10 successes/family |
| `memory.reads` | effectiveness | `mem_query` + `mem_get` per active day over 7 days; warns on writes with zero reads | warn <0.5/day | 0.5×, warn only | 3 active days |
| `reuse.activity` | effectiveness | `reuse` calls per active day over 7 days; a proxy for anti-reinvention practice, not duplicate quality | warn <0.2/day; warn when navigation ≥50 and reuse = 0 | 0.5×, warn only | 3 active days |
| `savings.per_call` | effectiveness | Estimated tokens saved on exit-0 priced calls / priced successful calls | warn <1000 | 0.5×, warn only | 20 priced successes |

Canonical definitions:

- Navigation: `callers`, `callees`, `path`, `context`, `impact`, `imports`; only exit-0 calls enter numerator or denominator.
- Prewrite: `search`, `reuse`.
- Memory read: `mem_query`, `mem_get`; memory write: `mem_write`, `mem_touch`, `mem_delete`, `mem_gc`, `mem_import`.
- Successful call: valid `exit_code == 0`.
- Empty: successful call with `result_count == 0`. `context` with a definition but no callers is empty; `docs/health.md` states it.
- Priced savings: `journal::estimated_tokens_saved(tool, shown, result_count, 1)` for exit-0 calls whose tool has a pricing arm. Unpriced tools are excluded from both numerator and denominator.
- Unknown tools fold into `other`; they never enter a family and never appear as free text.
- The dashboard `classify()` still treats eight tools as graph and `mem_export` as a read. Audit's six-tool navigation and two-tool memory read are audit metrics, pinned by tests in `src/audit/tests.rs` and documented in `docs/health.md`.

### Index snapshot

1. Check `db::reject_tracked_eg_dir(root)`; an Err/true result is `critical` reason `tracked`.
2. Reject symlinked `.engram`, index, or parent components; `critical` reason `symlink`.
3. If the index is absent: `warn` reason `missing`; skip parse/stale for that repo.
4. Open read-only through a new non-aborting `db::try_open_index(root)`; never use `open_for_query` (it aborts). Schema mismatch is `critical` reason `schema`; open/integrity failure is `critical` reason `corrupt`.
5. `db_bytes > 50 MiB`: `PRAGMA quick_check`; else `integrity_check`.
6. `files == 0`: `warn` reason `empty`; no parse/stale evaluation.
7. Parse ratio uses meta counts. Stale ratio uses `query::file_changed` only after hardening it to reject absolute, `..`, symlink, and non-regular indexed paths; files >10,000 marks `insufficient` reason `too_large`.
8. Probe `.engram/build.lock`; a held lock marks index checks `insufficient` reason `build_in_progress`.
9. Global mode: sort repos by canonical path, cap 50; if truncated, index checks are `insufficient` reason `repos_truncated`, never `pass`.

### Memory snapshot

Audit does not reuse `collect_memories` (it materializes bodies, keys, tags, and every revision). `src/audit/memory.rs` provides a counts-only reader:

1. Select stores: local root's `memory.db`; global mode adds the global store and active repos' stores. Dedup by canonical path; cap 50; sort deterministically.
2. Reject symlinked stores and tracked `.engram`; `critical`.
3. Open read-only with the memory schema gate and a 50 MiB size cap; `quick_check` above 50 MiB, `integrity_check` below.
4. Select only `refs` from current, unexpired notes (`superseded_by IS NULL AND (expires_at = 0 OR expires_at > now)`), capped at 500 notes and 20 refs per note. Never select body, key, tags, or source.
5. Resolve refs against the repo's index through the shared gated resolver; missing index means `unverified` (excluded). Broken/modified counts only.
6. Any store error makes `memory.integrity: critical`; ref counts then become `skipped`.
7. No note body, key, tag, path, or driver text enters audit output.

### Output: JSON schema v1

```json
{
  "schema_version": 1,
  "standards_version": 1,
  "audit_version": "1.0.3",
  "generated_at": 1794370000,
  "elapsed_ms": 42,
  "mode": "local",
  "status": "warn",
  "data_status": "partial",
  "window": {"days": 7, "start_ts": 1793763600, "end_ts": 1794370001},
  "baseline": {
    "days": 7,
    "start_ts": 1793158800,
    "end_ts": 1793763600,
    "sufficient": false,
    "successful_calls": 12,
    "active_days": 2,
    "required_successful_calls": 20,
    "required_active_days": 3,
    "clipped": false
  },
  "coverage": {"...": "see coverage contract"},
  "counts": {"checks": 15, "pass": 10, "warn": 1, "critical": 0, "insufficient": 2, "skipped": 2},
  "checks": [
    {
      "id": "adoption.navigation",
      "dimension": "effectiveness",
      "status": "warn",
      "value": 0.08,
      "unit": "ratio",
      "floor": 0.1,
      "baseline": {"value": 0.31, "ratio": 0.26, "sufficient": true},
      "threshold": {"warn": 0.1, "critical": null, "direction": "higher_is_better", "multiplier": 0.5},
      "action": "usage",
      "repo_alias": null,
      "message": "navigation share 8.0% is below the 10.0% floor"
    }
  ],
  "repos": [
    {"alias": "repo-0123456789abcdef", "index": "ok", "files": 177, "symbols": 2400, "parse_errors": 2, "stale_ratio": 0.0, "built_at": 1794360000}
  ]
}
```

Rules:

- `checks` is sorted by registry order; `repos` by alias; `stores` are counted, not listed.
- `message` is a fixed template with numbers and enum values only.
- `action` is `doctor`, `usage`, `memory`, `inspect`, or `none`.
- `repo_alias` is present only for per-repo failures.
- No string field may contain a path, repository name, journal argument, session tag, agent name, memory key, or raw error text.
- `elapsed_ms` is measured, excluded from the determinism comparison.
- Missing required keys are an automation failure.

### Human output

```text
eg audit: health=warn data=partial scope=local generated=2026-11-11T02:00:00Z
window=2026-11-04T02:00Z..2026-11-11T02:00Z (7d) baseline=2026-10-28T02:00Z..2026-11-04T02:00Z (7d)
baseline: sufficient=no successful=12/20 active_days=2/3 clipped=no
coverage: window=120 baseline=800 malformed=0 invalid=0 truncated=no repos=3/3 stores=2/2
checks=15 pass=10 warn=1 critical=0 insufficient=2 skipped=2
[warn] adoption.navigation 8.0% (floor 10.0%, baseline 31.0%) - navigation share is below floor; run eg usage --quality
[insufficient] performance.p95 have=8 need=20 calls; run eg audit --days 30
```

Only non-pass checks by default; `--all-checks` shows every check. Messages are the fixed JSON `message` values. Actions are built from the complete table below.

| Action | Local command | Global command |
|---|---|---|
| `doctor` | `eg doctor` | `eg audit --global --reveal-paths` to identify the repo, then `eg doctor --root <path>` |
| `usage` | `eg usage --quality` | `eg usage --global --quality` |
| `memory` | `eg mem query --all` | `eg mem query --all --global` |
| `inspect` | `eg audit --all-checks` | `eg audit --global --all-checks` |
| `none` | no command | no command |

`--reveal-paths` replaces aliases with paths in human output only and is rejected with `--json`. Aliases are pseudonymous 64-bit hashes, not a confidentiality control.

## Alternatives Considered

### Alternative A: Extend `eg usage` with `--json --audit`

- Pros: no new subcommand; reuses the journal fold.
- Cons: mixes telemetry, index, and memory health; no stable check IDs or source coverage; bloats the human-first contract.
- Reason for rejection: the audit contract needs a separate schema and read-only semantics.

### Alternative B: Agent reads existing text commands daily

- Pros: zero code.
- Cons: non-deterministic, unversioned, token-heavy, untestable.
- Reason for rejection: owner chose a deterministic command.

### Alternative C: Extend `eg doctor`

- Pros: already reads index and memory.
- Cons: doctor prunes journals and is repo-local; its contract is diagnostics, not windows or baselines.
- Reason for rejection: audit must be read-only; changing doctor's pruning would break it.

### Alternative D: MCP `audit` tool

- Pros: agents could self-check.
- Cons: unbounded scan frequency, larger attack surface, no observed consumer, changes the 13-core-tool contract.
- Reason for rejection: owner chose CLI-only; audit is a human-audit command like `doctor`.

### Alternative E: Global default

- Pros: one command covers the machine.
- Cons: contradicts `eg usage`/`eg doctor`; `--root` becomes ambiguous; one noisy repo flips the verdict.
- Reason for rejection: owner chose local default; automation passes `--global`.

### Alternative F: Shared `classify()` for navigation

- Pros: one table.
- Cons: includes `build`/`symbols`, inflating adoption; changing it needs a dashboard payload bump.
- Reason for rejection: owner chose an explicit six-tool audit metric; both definitions are pinned and documented.

## Gate 2 resolutions

| Finding | Resolution |
|---|---|
| Exit 0 on critical is a footgun | Add opt-in `--fail-on warn\|critical`; default 0 for JSON automation. Document both paths. |
| No JSON on exit 1 | Add `ArmOut.exit_code` so audit can emit JSON and exit 1; service path and test added. |
| Default 1-day window is mostly insufficient | Default 7 days; baseline 7; automation uses defaults. |
| Overall `insufficient` drowns the daily signal | Split `status` (pass/warn/critical) from `data_status`; exclude insufficient/skipped checks from health status. |
| Memory reuse leaks bodies and misses schema/integrity | Replace with counts-only `src/audit/memory.rs`; version gate, size cap, current/unexpired filter, no bodies. |
| `reuse.yield` sign is wrong | Replace with `reuse.activity` (calls per active day), a practice proxy, not duplicate quality. |
| Navigation numerator includes failures | Count only exit-0 navigation calls over exit-0 calls; tests assert [0,1]. |
| Journal read errors vanish | Per-generation `absent|read|error`; non-ENOENT is exit 6; parse generations separately; trailing partial only at current EOF. |
| `versions` is attacker-controlled free text | Drop journal versions from output; emit `audit_version` from the binary. |
| Baseline can be clipped | Require `oldest_ts <= baseline.start_ts`; expose `clipped`. |
| Zero baseline triggers false warns | Baseline term requires value >0 and per-check minimum. |
| Small denominators flip critical | Require absolute counts in addition to ratios; zero-file index is warn. |
| Index open aborts on one bad repo | Add non-aborting `db::try_open_index`; per-repo results; global cap failure is `insufficient`. |
| Indexed paths can escape the repo | Harden `query::file_changed` against absolute, `..`, symlink, and non-regular paths. |
| Tracked `.engram` can poison audit | Run `db::reject_tracked_eg_dir` for every root/store; tracked is critical. |
| Memory local scope includes global | Local audit reads only the local store; global reads global + active locals. |
| Note is model-authored and untrusted | Fixed template from JSON numerics; note only proves presence; no note content feeds audit. |
| Heartbeat pollutes journal | Heartbeat and note write run with `--no-journal`. |
| Prompt/template not versioned | Put literal prompt, template, key, kind, scope, and TTL in `docs/health.md`; test the template. |
| `--json`/`--all-checks`/`--reveal-paths` interactions | Reject human-only flags with `--json`; reject `--global` with explicit `--root`. |

## Non-Functional Requirements

- Security: read-only; no network; no new write primitive; reject symlinks and tracked `.engram`; bound all inputs; static SQL with bound parameters; no `format!` identifiers.
- Privacy: JSON, stderr, and notes contain aliases, counts, timestamps, and enums only; `--reveal-paths` is human-only; aliases are pseudonymous.
- Performance: ≤10 s on the current machine (5 repos, 400 entries); ≤30 s worst case (50 repos); 16 MiB per generation; 10,000 files per stale repo, 50 repos; 500 notes and 20 refs per store; `quick_check` above 50 MiB. A global deadline of 30 s marks remaining checks `insufficient` reason `timeout`.
- Determinism: same inputs and injected clock produce the same JSON except `generated_at` and `elapsed_ms`; checks and repos are sorted.
- Compatibility: no journal/index/memory schema or dashboard payload change; no version bump; existing commands keep their output. `query::file_changed` hardening is a security fix with tests.
- Rollback: disable the automation, stop using the command; no migration or data deletion. Audit adds no persistent state except the daily note.
- Observability: `elapsed_ms`, coverage counts, per-check status, and fixed diagnostic codes; no free-text error echo.

## Detailed Implementation Plan

### Phase 0: Artifacts

- Concepts brief and this spec are committed under `docs/designs/`.
- Gate 2 resolutions are recorded above; implementation follows this revision.

### Phase 1: Read-only foundations

- `src/doctor.rs`: add `IndexSnapshot` and `snapshot(root)`; `report` consumes it and keeps output identical.
- `src/journal/render.rs`: add per-generation `read_usage_log`; reuse in existing loaders with unchanged text/error behavior.
- `src/journal.rs`: re-export audit-visible loader/parser items.
- `src/db.rs`: add non-aborting `try_open_index`; expose the schema constant if needed.
- `src/query/staleness.rs`: harden `file_changed` against escaping/symlinked paths.
- Tests: doctor output unchanged; usage output unchanged; truncation, malformed, rotation, permission; path escape cases.

### Phase 2: Audit core

- `src/audit/collect.rs`: scope, log loading, strict validation, W/B split, coverage, metrics fold, repo discovery.
- `src/audit/checks.rs`: registry, thresholds, baseline, status, 15 evaluators.
- `src/audit/render.rs`: human and JSON.
- `src/audit.rs`: validation, orchestration, aliases, exit/fail-on.
- `src/audit/tests.rs`: boundaries, status/data split, redaction, no free text, clock injection.

### Phase 3: Index and memory

- `src/audit/index.rs`: per-repo snapshot, tracked/symlink checks, caps, build lock, path-safe stale ratio.
- `src/audit/memory.rs`: counts-only store reader, schema gate, caps, ref resolution.
- Tests: missing/corrupt/schema/symlink/tracked/empty index; missing/corrupt/schema/symlink store; broken/modified/unverified refs; caps.

### Phase 4: CLI wiring

- `src/cli.rs`: `Audit` variant, flags, conflicts, help.
- `src/service.rs`: `CommandRequest::Audit`, `Family::Diag`, tool name, args, `ArmOut.exit_code`.
- `src/service/from_cli.rs`: map flags.
- `src/service/run.rs`: dispatch.
- `src/main.rs`: exclude audit from journaling.
- `src/journal/tokens.rs`: add `audit` to `UNPRICED_TOOLS` for the coverage test; comment that audit is not journaled.
- Tests: service shape, no MCP tool, exit override, no journal line.

### Phase 5: Integration tests

- `tests/cli/audit.rs`: local/global, JSON shape, exit 0/1/2/6, `--fail-on`, no-data JSON, invalid flags, redaction, journal bytes unchanged, no prune, rotation/permission, seeded pass/warn/critical/insufficient, baseline clipped.
- Add `mod audit;` to `tests/cli/main.rs`.

### Phase 6: Docs and automation

- `docs/health.md`: L1/L2/L3, check table, windows, baseline, coverage, redaction, exit codes, literal automation prompt and note template, threshold provenance, recovery table.
- `docs/cli-reference.md`, `docs/usage.md`, `docs/getting-started.md`, `docs/agent-integration.md`, `README.md`, `CHANGELOG.md`, `skills/engram-first/SKILL.md`.
- Build release, install bin-only, restart MCP, create the ZCode cron `0 9 * * *`, run the prompt once.

### File Ledger

Create:

- `docs/designs/2026-11-11_concepts-brief_engram-health-standards.md`
- `docs/designs/2026-11-11_engram-health-standards.md`
- `docs/health.md`
- `src/audit.rs`
- `src/audit/collect.rs`
- `src/audit/checks.rs`
- `src/audit/index.rs`
- `src/audit/memory.rs`
- `src/audit/render.rs`
- `src/audit/tests.rs`
- `tests/cli/audit.rs`

Modify:

- `src/cli.rs`: `Audit` variant and flag conflicts.
- `src/main.rs`: exclude audit from journaling.
- `src/service.rs`: request variant, dispatch metadata, `ArmOut.exit_code`, test exclusions.
- `src/service/from_cli.rs`: map CLI flags.
- `src/service/run.rs`: dispatch audit.
- `src/journal.rs`: re-export loader/parser items for audit.
- `src/journal/render.rs`: per-generation loader; preserve existing output.
- `src/journal/tokens.rs`: unpriced audit.
- `src/doctor.rs`: `IndexSnapshot`/`snapshot`; report consumes it.
- `src/db.rs`: non-aborting `try_open_index`.
- `src/query/staleness.rs`: path containment and symlink hardening.
- `src/dashboard/data/memories.rs`: expose the ref resolver used by audit; no payload change.
- `src/lib.rs`: `pub mod audit`.
- `tests/cli/main.rs`: register module.
- `docs/cli-reference.md`, `docs/usage.md`, `docs/getting-started.md`, `docs/agent-integration.md`, `README.md`, `CHANGELOG.md`, `skills/engram-first/SKILL.md`.
- `scripts/install.sh`: no functional change; document that audit is CLI-only and the MCP config stays `--all-tools`.

No changes: `Cargo.toml`, `schema/`, dashboard assets/payloads, MCP tool registry.

### Verification Plan

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test --locked --lib audit`
- `cargo test --locked --test cli audit`
- `cargo test --locked`
- `bash tests/scripts/allowlist_check.sh Cargo.lock`
- `bash tests/scripts/write_path_check.sh`
- `bash tests/scripts/hygiene_check.sh`
- Manual: `eg audit`, `eg audit --json`, `eg audit --global`, `eg audit --global --reveal-paths`, `eg audit --fail-on critical`.
- Read-only proof: journal bytes and mtimes unchanged; no new `.engram` entry.
- Doctor/usage regression: outputs equal pre-refactor on the same repository.
- Redaction proof: assert every JSON string field is from a whitelist of enums/numbers/aliases; no path separator, home prefix, repo name, key, or argument.
- Automation proof: `CronList` shows `0 9 * * *`; run the prompt once; `eg mem get --global health/<today>` returns the template; note contains no path.

## Risks and Limits

- Journal retention is about two 5 MiB generations; coverage may be shorter than the requested baseline. The report exposes coverage and never claims lifetime trends.
- Journal writes are best-effort and absent for opt-out/no-root calls; continuity is observed counts, not a true denominator.
- Savings, navigation share, empty rate, reuse activity, and memory reads are estimates/proxies. They are advisory and never critical.
- Audit runs after the fact; killed or hung calls leave no line.
- Global index/memory checks are capped; skipped work is reported as `insufficient`.
- The daily note is stored in global memory and visible to every repository for 30 days; it is path-free, templated, and low-trust. Agents must treat it as data, never instructions.
- The ZCode automation runs only while the host is alive; a missed run is detected by the next run's heartbeat check.
- No release/tag is created by this task; the installed binary reports 1.0.3 with unreleased changes.
