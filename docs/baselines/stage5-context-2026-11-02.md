# Stage 5 — `kg context` one-call utility A/B (Amendment 17, G21)

- Date: 2026-11-02. Corpus: sanitized vihap (`/tmp/prosev-new`, 252 files).
- Tasks: F1/F2/F3 = report qualname, every direct caller call-site pointer, and
  every direct callee definition pointer (small fan-in/out targets; one empty
  callee section). GT frozen from the shipped binary at freeze.
- Arms: control composes `kg callers` + `kg callees` + `kg symbols`; treatment
  runs `kg context` first. 24 cells (3 tasks x 2 arms x 2 hosts x 2 runs).

## Correctness: 23/24

One control cell (OpenCode, F2) over-reported two indirect callers found by
reading; every treatment cell matched the GT exactly. No treatment loss.

## Economics (mean per run; host-recorded)

| host | arm | kg calls | tool calls | input tokens |
|------|-----|----------|------------|--------------|
| Grok Build | control | 5.17 | 5.33 | 178,935 |
| Grok Build | treatment | **0.83 (−84%)** | 4.33 (−19%) | 143,794 (−20%) |
| OpenCode gpt-6-sol | control | 0.67 | 9.0 | 3,994 |
| OpenCode gpt-6-sol | treatment | 0.5 | **4.83 (−46%)** | 2,519 (−37%) |

## G21(iii) verdict: NOT MET by the letter (Grok 19% vs the 20% bar)

OpenCode clears the bar strongly (−46% tool calls). Grok cuts kg calls by 84%
and tokens by 20% but tool calls by 18.75% — 1.25 points under the bar. Grok's
control arm was already cheap (5.3 calls); its treatment runs split: one
10-call outlier and two 3-call runs (one round used grep only, no kg calls).

Per Amendment 17's kill rule the default is to remove `context`; the same
owner-re-decision clause used for the P6 search upgrade applies. The feature
is implemented, tested (exact-byte fixtures, MCP parity, schema unchanged) and
committed; its value is host-dependent but the OpenCode economics are the
strongest single-feature result since `path`.
