# R6b — Privacy / fresh-export safety (Goldfish, 2026-11-02)

- Verdict: **NEEDS FIXES** — 3 Blocker, 3 Important, 4 Suggestion/Nit.

| # | Sev | Finding | Evidence |
|---|-----|---------|----------|
| 1 | Blocker | `docs/baselines/**` (581 files / 2.48 MB) cannot ship: 16,305 private-name hits, 12,746 source citations across 202 files, 318 `/Users/...` in 87 files. Name replacement alone is unsafe: `T4-002.txt` has zero identifier hits but contains a verbatim private method body | `git grep` counts; `transcripts/out/T4-002.txt:7,15` |
| 2 | Blocker | Fresh repo would embed employer email: global `user.email=quy.doan@be.com.vn`, all 66 commits use it; `git init` inherits it | `git config --global user.email`; `git log --format='%ae'` |
| 3 | Blocker | 68 privacy lines in the 12 included non-baselines files: private symbols in `src/extract.rs:304,1926,1928,1930`, `tests/fixtures.rs:316-319`, scripts' `--vihap-root`, CHANGELOG, SUPPORT, designs | file:line as listed |
| 4 | Important | A–F labels insufficient: exact per-corpus counts + domain descriptions re-identify (A = 252/3,291/9,128 + "Python address-parsing monorepo") | `EVALUATION.md:24-29,34` |
| 5 | Important | 56 references to the excluded baselines tree → 404 in public docs | `EVALUATION.md:5` + 55 more |
| 6 | Important | Publication contradiction: `LicenseRef-Private` + "private, unpublished" | `Cargo.toml:6`; `LICENSE:3` |
| 7-10 | Suggestion/Nit | Owner name in public docs; `~/` refs in skills README; export must be archive-based (not rsync; `.kg` 1 MB, `target` 2.2 GB, `.zcode`); secret scan clean (0 key formats) | — |

Export manifest: **51 files** (root 11, .github 5, src 16, tests 12, skills 2,
docs 5); exclude 581. Build from `git archive HEAD`, delete `docs/baselines`,
new `git init`. Gate (zero asserts): private names / private symbols /
personal paths / `~/` / employer email / machine names / domain markers /
secret formats = 0; no baselines; no ignored artifacts; file count = 51.
Set public git identity before the first commit.
