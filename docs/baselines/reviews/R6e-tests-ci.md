# R6e — Tests and CI (Goldfish, 2026-11-02)

- Verdict: **NEEDS FIXES** — 5 Blocker, 12 Important, 5 Suggestion.
- Local: build/clippy/tests (98)/allowlist/write-path all green;
  `cargo fmt --check` fails 356 hunks; `cargo install --path . --locked` works.

| # | Sev | Finding | Evidence |
|---|-----|---------|----------|
| 1 | Blocker | macOS seatbelt job **vacuous**: profile written but never applied; a network-reaching fake binary "PASSES" | `tests/scripts/seatbelt-build.sh:7-22`; probe `NETWORK REACHED` + PASS |
| 2 | Blocker | Linux bwrap job red as written (`--ro-bind / /` blocks `.kg/` creation) | `ci.yml:56-57`; build on read-only root → exit 6 |
| 3 | Blocker | "Cargo.lock allowlist" is a 19-token denylist; `mio` regex broken (`mio$`); `tungstenite`/`async-std` pass | `allowlist_check.sh:8-9`; probes |
| 4 | Blocker | G2 gate stale (1/10) and `precision_recall.py` exits 0 on FAIL — cannot gate | script:150 |
| 5 | Blocker | With #1-#3, invariant 1 ("zero network, CI-enforced") has no working enforcement | `README.md:93-95`; `SECURITY.md:5-10` |
| 6 | Important | write-path gate bypassable (`File::create`, `fs::copy`, `remove_file`); whole `walk.rs` exempted | `write_path_check.sh:18-21` |
| 7 | Important | Child-process test vacuous (marker only in fixture path; asserts after exit) | `tests/cli.rs:942-957` |
| 8 | Important | G15 documented command points to a missing script | spec `:43`; `ls tests/skill_contract_drift.py` |
| 9 | Important | G18 documented args mismatch; baseline binary in `/tmp` only | spec `:46`; `search_gap_recall.py:41-48` |
| 10 | Important | Most GT gates not runnable from this repo; no CI job runs any GT gate | vihap/finguard worktrees; finguard |
| 11 | Important | G12 "parallel == serial" untested | `src/build.rs:103-116`; no test |
| 12 | Important | MCP parity: 7/15 tools byte-compared; 8 never compared; comment says 11 vs 14 asserted | `tests/mcp.rs:109-112,170` |
| 13 | Important | `import_x_as_y_binds_module_root` vacuous (only asserts absence) | `tests/fixtures.rs:78-90` |
| 14 | Important | Missing contract tests: exit 6, `doctor --invalidate`, truncated index, caps for 5 commands, MAX_LIMIT, non-git root, `--max-parse-error-rate`, FTS metacharacters, MCP malformed JSON, carried loc/cyclomatic | `grep` results |
| 15 | Important | CI hygiene: actions unpinned, no `permissions:`, no timeout/concurrency/fail-fast | `ci.yml` |
| 16 | Important | No release/install/doc/MSRV jobs; no `rust-version` | `.github/workflows` |
| 17 | Important | No fmt gate; tree unformatted | `cargo fmt --check` 356 hunks |
| 18-22 | Suggestion | Tests `unwrap()` python3/sqlite3/pgrep; 91/91 stale; weak assertions; snapshot covers build only; nits (`fresh_db_has_schema_v2`, timing.sh BSD-only) | as cited |
