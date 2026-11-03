# R14 — Decision matrix (value re-evaluation)

Final code baseline: `bc292ba` (docs/tests included; implementation through
`cd00e5d`). Six lenses; L1-L5 vote value, L6 does not. R14 rule and evidence
are in `README.md`.

## L6 gate

**INSUFFICIENT for REMOVE.** The utility battery is n=2 per cell, one shape per
feature, six features have no task, all treatment cells paid a failed
mandatory command on non-git corpora, and the battery predates the remediation
commit. No feature reaches the 4/5 REMOVE threshold.

## Vote matrix

| Feature | L1 correctness | L2 utility | L3 latency | L4 contract | L5 removal | Final | Owner action |
|---|---|---|---|---|---|---|---|
| F1 implementers/dead | IMPROVE | IMPROVE | IMPROVE | IMPROVE | DEFER | **IMPROVE (dead demoted)** | Keep `implementers`; `dead` removed from routing until a precision gate exists |
| F2 impact `--context` | IMPROVE | DEFER | KEEP | IMPROVE | REMOVE (1/5) | **IMPROVE / OWNER** | No removal; owner decides flag demotion |
| F3 path/files/symbols dir | IMPROVE | DEFER | KEEP | IMPROVE | DEFER | **IMPROVE** | Add one inventory utility task; no claim change |
| F4 impact edges/depth | KEEP | IMPROVE | IMPROVE | IMPROVE | DEFER | **KEEP (host-dependent)** | n=5: agy -66.7%, zcode -53.3%, grok -16.7%; route with the mixed result stated |
| F5 unresolved names | IMPROVE | DEFER | KEEP | IMPROVE | DEFER | **IMPROVE** | Add one utility task; include or document ambiguous sites |
| F6 resolve/search filters | IMPROVE | DEFER | KEEP | IMPROVE | DEMOTE (filters) | **IMPROVE (filters demoted)** | Keep `resolve`; search-filter claim removed; apply D2 |
| F7 imports | IMPROVE | IMPROVE | KEEP | IMPROVE | DEFER | **KEEP (uniform)** | n=5: grok -50.0%, agy -76.2%, zcode -57.1%; equal correctness |
| F8 tests | IMPROVE | IMPROVE | IMPROVE | IMPROVE | DEFER | **IMPROVE** | Latency fixed for <=20k tests; >20k remains a stress case |
| F9 complexity/metrics | IMPROVE | DEFER | KEEP | IMPROVE | DEFER | **IMPROVE** | Add one ceiling/ranking task; no claim change |
| F10 api | IMPROVE | IMPROVE | KEEP | DEMOTE | DEMOTE | **DEMOTE (applied)** | Removed from routed skill/README; keep code experimental |
| F11 generated tagging | IMPROVE | DEFER | KEEP | IMPROVE | REMOVE (filters, 1/5) | **IMPROVE (tag) / DEMOTE (filters, applied)** | Keep tag/stats/files; six filters marked experimental |

No feature reaches 4/5 REMOVE. L6 blocks REMOVE. F4 and F7 now meet the KEEP
bar on one host with n=5; the other features stay IMPROVE or DEMOTE.

## Significant value

- **F4** is a measured winner: the original battery cut tool calls by 29.4% /
  36.6% / 45.0% on grok / agy / zcode at 12/12 correct answers. The local n=5
  replication on zcode cuts median tool calls 15 -> 7 (-53.3%) at 5/5 correct.
  The 1.37M-edge latency defect is fixed: `impact --depth 3` 97-144 ms vs
  150 ms.
- **F7** was positive on 2/3 hosts in the original battery (grok -71%, agy
  -19%, zcode 0%) and n=2. The local n=5 replication on zcode cuts median tool
  calls 7 -> 3 (-57.1%) at 5/5 correct.
- **F1** is mixed: agy -61% (n=1 control), grok -18%, zcode 0%.
- **F8/F10** have no positive utility in their only shapes; both shapes are
  narrow, and the battery's failed first command inflates treatment cost.
- **F2/F3/F5/F6/F9/F11** have no utility task. Absence of evidence is not
  proof of no value.

## DEMOTE packets

### D1 — `kg api` (F10)

- Evidence: A1 tools +14.3% / +125.0% / +28.6% at equal correctness; 3/3 hosts
  negative; 27 journal calls; no real-corpus export JSONL GT.
- Action: remove the `api` row from binding skill/README routing and mark the
  command experimental. Keep the CLI, MCP tool, `symbols.exported`, and tests.
- Reversal: one routing line; no schema or data migration.
- Residual: exported non-callable declarations (const/static/type aliases) are
  still not indexed; the caveat now says so.

### D2 — search filters (F6)

- Evidence: 1,198 journal search rows, zero with `--kind`/`--path`/`--exact`;
  no utility task; base search is the demand.
- Action: drop filter claims from routing; keep the code and `kg resolve`.
- Reversal: one routing line.

### D3 — `--exclude-generated` filters (F11)

- Evidence: zero filter uses across 2,913 journal rows; no utility task or GT;
  the tag/stats/files surfaces are used passively.
- Action: demote the six filter flags/claims; keep `files.generated`,
  `is_generated`, `stats generated=N`, and the negative-control tests.
- Reversal: one routing line; no schema change.

### D4 — `dead` (F1)

- Evidence: 6/50 sampled vihap candidates have zero external textual
  references; type/attribute references are known false-positive classes; no
  precision gate.
- Action: keep `implementers` routed; do not advertise `dead` as a workflow
  until a precision gate exists.

## REMOVE candidates (not approved)

| Candidate | Votes | Blocker |
|---|---|---|
| F2 `impact --context` | 1/5 REMOVE | L6 INSUFFICIENT; no 4/5; low removal cost but no value evidence |
| F11 `--exclude-generated` filters | 1/5 REMOVE (filters only) | L6 INSUFFICIENT; no 4/5; tag stays regardless |

These are owner decisions, not approved removals. A future REMOVE-grade
battery must meet L6's checklist: pre-registered rule/hash, n>=5, >=2 shapes
per feature, clean git-rooted corpora, multiset scoring, host-blocked analysis,
and a target-correct GT for the candidate class.
