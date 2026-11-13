# Reuse Phases 2–5 — Six-Lens Effectiveness Review

- Target: commit series ending at the Phase 2–5 implementation
- Date: 2026-11-13
- Lenses: correctness/spec, security/trust, performance/reliability, test quality, API/observability, effectiveness/calibration
- Method: six independent subagent reviews; findings reconciled; fixes and regression tests added; full gate rerun

## Verdict after remediation

| Lens | Initial verdict | Final verdict | Basis |
|---|---|---|---|
| Correctness/spec | CONDITIONAL | PASS | Proposal judge path, stale downgrade, tokenizer, D5 overlap, confidence order, target grammar, empty journal |
| Security/trust | BLOCK | PASS | Symlink-safe journal prune, import containment, allowlist confirm/global/import guards, judge stdin/caps/termination |
| Performance/reliability | CONDITIONAL | PASS | D2 name-token index + generic stoplist; judge drain bound; 5k/10k/hash gates |
| Test quality | CONDITIONAL | PASS | CI perf gates, oracle unit tests, negative coverage/proposal/judge/import/target cases |
| API/observability | CONDITIONAL | PASS | Empty-run counters, freshness journal, confidence UI, MCP limit parity, proposal feed intent |
| Effectiveness/calibration | BLOCK | PASS | D3 High removed; confidence documented as ordinal; McNemar/power/MDD, holdout, negative-count gates |

“PASS” means the shipped contract is internally consistent and verified. It does not claim field-calibrated probabilities; see residual risks.

## High-impact findings and fixes

| Finding | Fix |
|---|---|
| Journal prune followed a symlink and could truncate a user file | `safe_prunable_log` rejects symlinks/non-regular files; atomic temp+rename in `journal.rs`; regression test |
| `mem_import` allowed `../` reads and allowlist-key creation | MCP path rejects absolute/`..`/backslash and enforces canonical root containment; imports reject `reuse-allow/` keys |
| Judge never ran for proposal candidates | Proposal signature is the judge's left source; fake-judge regression test |
| Judge runtime failures were silent | One `eg: judge unavailable:` note for spawn/timeout/write/parse failures; malformed-output test |
| Judge could hang after child exit while a grandchild held stdout | Process group is terminated before draining; stdout/stdin capped at 64 KiB |
| Stale/unknown freshness left Medium/High authority | Every tier caps at `low` when freshness is stale or unknown |
| D3 High produced 0/4 precision on the repository | D3 is capped at Medium until a labeled gate proves High precision |
| Proposal/DT destructured tokenizer tuple incorrectly | Correct `(all_tokens, type_tokens, name_tokens)` order |
| D5 emitted classes with no shared methods | Non-empty method overlap required; same-language guard; deterministic sorted evidence |
| Coverage canonical selection was nondeterministic | Canonical chosen by caller count then file/line/qualname; three-run determinism test |
| D2 index degenerated with common vocabulary | Name-token pairs only, plus 5% generic-name stoplist; common 10k corpus runs in 0.04 s |
| Empty runs lost mode/counts | `set_mode` and `set_reuse_counts` run before the empty return; allowlist/below-threshold journal tests |
| Freshness was printed but not journaled | Journal `freshness` object and `stale` result flag; usage-quality counters |
| Scans sorted by overlap before confidence | Confidence rank is the first sort key |
| Output cap skipped oversized blocks | Rendering stops at the first block that would exceed 30 KiB |
| Coverage ignored path scope | `--coverage` plus paths exits `2` instead of silently scanning the repo |
| `file:line` symlink and trailing-slash targets diverged | Symlink prefix rejected; `./` and trailing slash normalized |
| D4 printed a shared-items line | D4 now emits no shared-items line |
| Proposal single-symbol repo returned zero | Corpus-size guard removed; single-callee matches are Low |
| Oracle validator accepted invalid ledgers | Negative tier count, split/tier consistency, holdout scoring, baseline guard, and unit tests added |
| Perf gates never ran in CI | CI runs `reuse_bench --ignored` and a `--list` presence guard |

## Evidence

```
cargo fmt --all -- --check                         PASS
cargo clippy --all-targets -- -D warnings          PASS
cargo test --locked                                 PASS
  186 lib, 271 CLI, 37 fixtures, 33 MCP
allowlist_check / write_path_check / hygiene_check  PASS
reuse_bench: 5k p95 29.86 ms, 10k p95 57.34 ms, hash +6.85 ms
test_reuse_oracle.py                                4 tests PASS
oracle protocol fixture                             precision 1.0, McNemar power 1.0
```

## Residual risks

- Confidence tiers are ordinal heuristics, not field-calibrated probabilities. `high` is intentionally conservative (D1/D2 only; D3 capped).
- The tracked 160-label oracle file is a protocol fixture. Field calibration requires a frozen repository sample and predictions from real corpora.
- Coverage remains opt-in and low-precision by design; D1L/DT are advisory, not proof.
- Dashboard renders confidence but does not run coverage detectors; CLI coverage remains the authoritative surface.
- Allowlist suppression is an explicit local act with `--show-suppressed` visibility; it is not a security boundary against a malicious same-user agent.
- A judge binary is trusted input; model integrity and installation provenance remain operator responsibilities.
- The final confidence tier claims should be revisited after a real oracle run; until then `high` is a review aid, not a probability.
