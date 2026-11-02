# R6a — Public hygiene / legal readiness (Goldfish, 2026-11-02)

- Reviewer: stateless Goldfish, inputs = repo paths + acceptance criteria + lens
  only (no conversation history, no author rationale). Read-only.
- Verdict: **NEEDS FIXES** — 8 Blocker, 7 Important, 2 Suggestion, 1 Nit.

| # | Sev | Finding | Evidence |
|---|-----|---------|----------|
| 1 | Blocker | Proprietary license blocks any public release | `LICENSE:3-8`; `Cargo.toml:6`; `README.md:122`; spec OQ1 |
| 2 | Blocker | Private corpus content committed (10,085 name hits, private source lines in logs) | `docs/baselines/*`; see R6b |
| 3 | Blocker | Private repo path + revision exposed | `transcripts/SUMMARY.md:5`; `g17-path...md:27` (`~/Workspace/mapkit/vihap` @ `f7ca48c`) |
| 4 | Blocker | Right to relicense unconfirmed (commits authored with employer email; `@finguard/*` refs) | `git log --format='%an <%ae>'`; `SUPPORT.md:21` |
| 5 | Blocker | crates.io identity unavailable: `kg` and `kg-cli` already taken | crates.io API |
| 6 | Blocker | Release path missing: `publish = false`, no tags, no remotes, no release workflow | `Cargo.toml:7`; `git tag -l` |
| 7 | Blocker | No private vulnerability channel | `SECURITY.md:30-32` |
| 8 | Blocker | "Private project" framing in public-facing docs | `CONTRIBUTING.md:3`; `README.md:122`; `SECURITY.md:30`; `LICENSE:3` |
| 9 | Important | "Internal" evidence layer would ship in the crate | `README.md:124`; `cargo package --list` includes 586 docs files |
| 10 | Important | Install path untestable: no prerequisites/MSRV/OS table, no `--locked` | `README.md:29-33` |
| 11 | Important | "static binary" and "five pinned dependencies" are false (libSystem; 9 direct deps) | `otool -L`; `Cargo.toml:9-20` |
| 12 | Important | CHANGELOG has no released section | `CHANGELOG.md:5-8` |
| 13 | Important | CI lacks fmt/MSRV/install/audit/release; `cargo fmt --check` fails 356 hunks | `ci.yml`; local run |
| 14 | Important | Public URLs inconsistent (`quy.doan/kg` invalid; no remote) | `.github/ISSUE_TEMPLATE/config.yml:4`; `README.md:3` |
| 15 | Important | Spec makes uncited negative claims about named third-party tools | spec `:17,236` |
| 16 | Suggestion | Governance polish: CoC contact, dependabot, SPDX headers, SHA-pinned actions | — |
| 17 | Nit | Dirty tree at review time | `git status` |

Minimal fix order: ownership gate → sanitized export → dual license → identity
(rename or binaries-only `publish=false`) → NOTICE → community files → install
docs → CI gates → re-verify with `cargo package --list`.
