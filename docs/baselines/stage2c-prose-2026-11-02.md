# Stage 2c — docstring-vocabulary redesign result (Amendment 16, G20)

- Date: 2026-11-02. Corpus: sanitized kg (`/tmp/prose-new`, 25 files) and
  vihap (`/tmp/prosev-new`, 252 files), schema v6 docstring index.
- Design: schema v6 `symbols.docstring` (one-line docs/comments) added to the
  FTS text; frozen prose-gap set `docs/baselines/search-prose-gt.jsonl`
  (6 queries whose words live only in docstrings/comments).
- G20(i) G14 recall: 0.800 (23/28), unchanged — PASS.
- G20(ii) prose-gap recall: 5/6 improved vs the pre-P7 binary (old 0.00 -> new
  1.00 on five queries; the sixth already matched by name) — PASS.
- G20(iii) Stage 2c utility A/B (24 cells, 3 prose lookups, 2 arms x 2 hosts
  x 2 runs): correctness 24/24 in both arms; economics FAIL — Grok control
  5.5 tools / 1.0 kg calls vs treatment 6.17 tools / 3.5 kg calls (+12% tools,
  +30% tokens); OpenCode 5.83 vs 5.33 tools (−9%), tokens +20%. The routed
  arm adds search calls without replacing grep work.

**Verdict: G20(iii) FAIL -> Amendment 16 reverted.** The capability improves
retrieval (5/6 prose queries), but it does not produce an agent-economics
gain: grep already reads docstrings as ordinary source text, so symbol-level
lexical search adds a step rather than replacing one. Same root cause as
Stage 2: lexical-vs-lexical on small corpora. The revert restores schema v5;
the prose GT and this record stay as evidence. The proven large-value feature
remains `path` at scale (Stage 4).
