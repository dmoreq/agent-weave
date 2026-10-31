# Real-world effectiveness measurement protocol (R0-R3)

Purpose: measure kg's real-world value in the owner's daily agent workflow —
the field complement to the harness G6/G8 A/B (which remains the gate input
for M3's G10). kg observes only its own calls; the counterfactual side comes
from agent-host session transcripts. No network, no telemetry, no code
leaves the machine.

## Data sources

| Source | Provides | Where |
|--------|----------|-------|
| `.kg/usage.log` (Amendment 6) | kg-side: tool, args class, result counts, exit codes, latency, stale/empty/ambiguous rates | per repo, local |
| Agent-host session transcripts | counterfactual: Grep/Glob/Read tool calls and output bytes per session | host session logs |

## R1 — baseline period (kg unavailable in the daily loop)

- Duration: 14 calendar days.
- Discipline: do NOT invoke kg from the daily agent workflow (development
  dogfooding inside kg's own test suite does not count — it never enters
  daily-work sessions).
- Capture per work session: date, primary repo, task class (navigate / flow
  / impact / architecture), Grep+Glob+Read call counts, approximate output
  bytes. Derived from host transcripts after the fact; no special logging
  during the window.

## R2 — treatment period (kg available)

- Duration: 14 calendar days, same task mix discipline (log the task class
  per session as in R1).
- kg reachable as a plain CLI command (J1). MCP surface comparison is a
  separate follow-up after M3 — do not mix surfaces inside one window.
- Capture the same transcript metrics PLUS `.kg/usage.log` from every repo
  touched.

## R3 — analysis

- Unit of comparison: session task class, matched R1-vs-R2 where the task
  classes align; report per-class medians, never pooled single averages.
- Metrics: median tool calls per task, median transcript bytes per task,
  kg calls per task, kg empty-result rate, kg stale rate, kg ambiguous rate.
- Confounders to state in the report: single user, self-selected task mix,
  novelty effect, task complexity drift between windows. The report labels
  every number "observational" — it cannot be a randomized trial.
- Deliverable: `docs/baselines/real-world-g6-results.md` with one table per
  task class and an explicit verdict on whether the field displacement
  (grep/read -> kg) is consistent with the harness G6 direction and
  magnitude.

## Interpretation guardrails

- kg call volume rising is NOT effectiveness; displacement and empty/stale
  rates falling are.
- A rising stale rate means the rebuild discipline (or the staleness UX) is
  the bottleneck — action item, not a verdict on the index.
- A rising empty-result rate on a stable query mix means extraction gaps —
  feed the top unresolved symbol classes into M2+ backlog.
