# Concepts Brief — Engram Health Standards and Daily Audit

- Date: 2026-11-11
- Topic: `engram-health-standards`
- Source artifacts: `docs/architecture/code-index.md`, `docs/architecture/memory-store.md`, `docs/evaluation.md`, `docs/usage.md`, `.github/workflows/ci.yml`
- Gate 1 lock (owner, 2026-11-11): add `eg audit`; persist results as chat plus a 30-day global memory note; use hard floors plus a 7-day baseline; run daily at 09:00.

## Anchored hypothesis

Engram emits enough local telemetry to detect real regressions, but it has no normative definitions, no time-windowed evaluation, and no recurring check. A single read-only `eg audit` command can fold the retained journal, index snapshots, and memory refs into a small check registry with hard safety floors and baseline-relative effectiveness warnings. A daily host automation can then call it, report anomalies, and persist one note per day without adding a daemon, network call, or new file outside the confinement roots.

The command is CLI-only and repo-local by default, matching `eg usage` and `eg doctor`; the daily automation passes `--global`. Output is redacted by default: repositories appear as stable aliases, never absolute paths, and free-text journal values never enter the report.

## Consumers

- Human owner/operator: one daily signal, or an on-demand health report.
- AI agents: `eg audit --json` and the MCP `audit` tool for a compact, checkable status.
- Maintainers: a spec that fixes metric semantics and thresholds before code.

## Hard constraints (repo invariants)

- Zero outbound network; loopback HTTP only for the explicit `eg dashboard`.
- Zero background daemons, file watchers, Git hooks, or host-config edits in Rust runtime code.
- Zero Python runtime for operational features; Python stays in tests/bench.
- Writes and deletes confined to `.engram/` and `~/.engram/`; write primitives only in `db.rs`, `doctor.rs`, `journal.rs`.
- Single native binary; no new dependencies; MSRV 1.90; zero `unsafe`.
- ≤500 LOC per Rust source/test file; ASCII CLI output; no emoji/sparkline.
- Journal retention is about 2 generations at 5 MiB, not a time window.

## Out of scope

- Replacing `eg usage`, `eg doctor`, `eg stats`, or the dashboard.
- Changing journal schema, write semantics, rotation, pruning, or live-repo rules.
- Measuring actual host/model token consumption; savings stay estimates.
- Correlating journal entries with later edits to prove reuse adoption.
- Running L2 release benchmarks or L3 deep evaluations on a daily schedule.
- A repo-owned scheduler, launchd job, cron entry, GitHub schedule, or network alert.
- Version bump, release, or tag in this task.

## Capability map

| Module | Capability | Depends on | Build order |
|---|---|---|---|
| C1 | Normative standards: windows, formulas, thresholds, statuses, limitations | none | 1 |
| C2 | `eg audit` CLI/MCP: load logs, index, memory; 15 checks; human/JSON | C1 | 2 |
| C3 | Daily ZCode automation: call C2, report anomalies, write memory note | C2 | 3 |
| C4 | Docs and skill surfaces for C1/C2 | C1, C2 | 2 (parallel with C2 tests) |

## Problem areas

| Area | Observation | Risk |
|---|---|---|
| A. Metric semantics drift | CLI prices error calls; dashboard prices success only; dashboard graph class includes `symbols`/`build`; `session recall` counts memory calls, not retrieval | Two surfaces disagree; a daily audit would inherit the wrong one |
| B. No time window | Every reader folds the whole retained log; no daily slice or baseline | Cannot tell a regression from a busy day |
| C. Retention blindness | 5 MiB rotation, one prior generation, prune on dashboard/doctor; opt-out and no-root calls leave no marker | Trend claims can be wrong or silently incomplete |
| D. Unverified effectiveness | Counts exist, but no floors for adoption, empty rate, memory reads, reuse yield, or savings per call | Tool can look "up" while agents stop using it usefully |
| E. No recurring check | `eg doctor` is manual and repo-local; `eg usage` is manual and text-only | Silent degradation persists until someone looks |
| F. Automation constraints | No scheduler in repo; any daemon or network alert violates invariants | A naive daily job would break the security posture |

## Proposed measurement model

- L1 daily online standards: journal, index, memory, adoption. Implemented by `eg audit`.
- L2 release gates: `bench/run.py`, recall/precision scorers, memory oracle.
- L3 periodic deep evaluation: `docs/evaluation.md`.
- Windows: `W` = last N days (default 1); `B` = preceding M days (default 7), both half-open ranges ending at invocation time. Baseline is usable with ≥20 successful calls and ≥3 active days.
- Status: one enum `pass`, `warn`, `critical`, `insufficient` for checks and overall; precedence `critical > warn > insufficient > pass`. `insufficient` means the check could not be evaluated, never "healthy".
- Fifteen checks with stable ids: `index.integrity`, `memory.integrity`, `ops.errors`, `index.parse_errors`, `index.staleness`, `telemetry.malformed`, `telemetry.continuity`, `performance.p95`, `memory.refs`, `adoption.navigation`, `adoption.pillars`, `quality.empty_results`, `memory.reads`, `reuse.yield`, `savings.per_call`.
- Canonical definitions: audit navigation is six read-only query tools (not `symbols`/`build`); prewrite is `search`/`reuse`; memory read is `mem_query`/`mem_get`. This is an audit metric, not a change to the dashboard's eight-tool `classify()`; the docs and tests pin the difference. Savings are success-only estimates.

## Strongest objections (owner must resolve)

1. **YAGNI:** `eg usage --global --quality` plus `eg doctor` already cover most numbers. A new command adds a public surface, MCP updates, docs, and tests. Why not extend `usage` with `--json`?
2. **Proxy validity:** reuse yield, graph share, and savings per call are proxies. They can warn on a legitimately different workflow. The audit needs an explicit "insufficient" state and docs that forbid treating them as causal proof.
3. **Retention bias:** a 5 MiB rotation can discard the baseline. Daily reports may compare unequal windows. The audit must publish coverage and never claim lifetime trends.
4. **Alert noise and cost:** a daily agent turn can produce noise or consume tokens. Hard floors plus baseline reduces false alarms, but the first days lack history.
5. **Privacy:** JSON and human output contain absolute repo paths. They must stay local, never enter tracked docs, and the memory note should summarize without paths.
6. **Cross-surface cost:** adding a subcommand triggers CLI, MCP, skill, public docs, changelog, and test updates. Is the MCP tool worth the surface, or should audit stay CLI-only?

## Review findings and resolutions

Seven lenses ran in isolated contexts. Findings repeated four themes; the owner resolved them before the spec.

| Theme | Finding | Resolution |
|---|---|---|
| T1 Taxonomy | A fourth tool table would disagree with `classify()`, the dashboard, and the skill | Owner chose an explicit six-tool `navigation` metric. `classify()` stays unchanged; audit documents and tests the difference. No dashboard schema change. |
| T2 Scope | Global default contradicts `eg usage`/`eg doctor` and silently ignores `--root` | CLI-only; repo-local default; `--global` opt-in. Automation always passes `--global`. No MCP tool; audit stays a human-audit command like `doctor`. |
| T3 Privacy | Absolute paths and free-text journal values can enter model context or a global note | Redacted by default: repositories are `repo-<hash8>` aliases; JSON has no paths, names, args, or error text. `--reveal-paths` is human-only. Note body is a fixed aggregate template. |
| T4 Observer effect | Audit and the daily note would pollute call counts, memory reads, p95, and error rates | Audit is excluded from journaling in `main.rs`; the note write uses `--no-journal`. A test asserts an audit run changes no check input. |
| T5 Coverage | "Telemetry coverage" has no denominator; opt-out and no-root calls leave no marker | Replace with `telemetry.continuity`: observed counts, oldest/newest timestamps, malformed and truncated counts. No coverage percentage. Zero entries is `insufficient`, never `pass`. |
| T6 Status and exit | Mixed vocabularies and undefined exit codes make automation guess | One status enum for checks and overall. Exit `0` when the audit completes (including warn/critical), `1` when no telemetry exists, `2` usage, `6` unreadable input. The automation reads JSON status, not exit code, for health. |
| T7 Read-only | `doctor::report` prunes the journal; reusing it would mutate the measured data | Extract `doctor::snapshot`; audit never calls `report`, never prunes, rotates, or writes. Journal bytes are asserted unchanged. |
| T8 Cost | Global integrity checks over every repo can saturate disk and CPU | Repo-local default. Global audits only repos active in `W`, cap at 50, use `quick_check` for DBs above 50 MiB, and open every database read-only. |
| T9 Untrusted input | Journal fields are attacker-writable and can inject into prompts or terminals | Only whitelisted tool names, enum statuses, integers, timestamps, and repo aliases enter output. Unknown tools fold into `other`. No free-text echo. |
| T10 Empty index | A zero-file index passes doctor's ratio checks | `index.integrity` treats an active repo index with zero files as critical. |
| T11 Lock contention | Exit 4 is routine during concurrent builds, not corruption | `ops.errors` is critical for exits 3/5/6/7; exit 4 warns only when it repeats more than three times in `W`. |
| T12 Window semantics | UTC day, local schedule, and daily note can disagree | Windows are trailing half-open epoch ranges ending at invocation, printed as UTC. The note key uses the local calendar date of the run. |
| T13 Automation absence | A stopped automation looks like a healthy day | The daily prompt checks yesterday's `health/YYYY-MM-DD` note and reports a missed run before running today's audit. |
| T14 Persistence noise | Daily notes enter every `mem_query` and can outlive TTL through revisions | Owner accepted daily keys. Body is fixed, path-free, tagged `health audit`, session scope, 30-day TTL. The prompt never passes raw JSON to `mem_write`. |
| T15 Baseline availability | A fresh journal cannot support a 7-day baseline | Baseline-relative comparisons are skipped when `B` lacks 20 successes or 3 active days; hard floors still apply. The report prints the deficit. |

## Decision log (Gate 1, owner)

- Delivery: new `eg audit` command; CLI-only; repo-local default with `--global`.
- Persistence: chat report plus one global memory note per day (`health/YYYY-MM-DD`, session scope, 30-day TTL); no new audit log file.
- Thresholds: hard safety floors plus a 7-day baseline; no 14-day observation-only period.
- Schedule: daily at 09:00 local through the ZCode host automation; no repo daemon.
- Taxonomy: audit uses six-tool `navigation`; dashboard `classify()` is unchanged.
- Privacy: redacted default; `--reveal-paths` human-only; JSON and notes are path-free.
