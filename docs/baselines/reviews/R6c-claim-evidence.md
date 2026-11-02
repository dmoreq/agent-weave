# R6c — Claim vs evidence (Goldfish, 2026-11-02)

- Verdict: **NEEDS FIXES** — 3 Blocker, 13 Important, 6 Suggestion, 3 Nit.
- Re-runs today (HEAD `6691b34`): search recall **0.800** (23/28); G2 self
  **1/10 FAIL** (GT lines stale); vihap precision **493/495**; path GT 8/8;
  tests **98/98**; self-index 34/355/1078, build 0.16 s.

| # | Sev | Finding | Evidence |
|---|-----|---------|----------|
| F1 | Blocker | "10–48% unresolved" is INVERTED: actual unresolved share is 51.8–90.4% (10–48% is resolution coverage) | `README.md:103-104`; `EVALUATION.md:176-177`; per-corpus fractions |
| F2 | Blocker | G14 recall 0.850 is not reproducible on the shipped P6 index: **0.800**. The G18(i) "unchanged 0.850" premise behind the keep-P6 decision is false | `README.md:27`; `EVALUATION.md:95`; `g14...md:19`; `CHANGELOG.md:40` |
| F3 | Blocker | Release blocked by license | `LICENSE`; `Cargo.toml:6-7` |
| F5 | Important | G2 self 10/10 stale: cited command returns 1/10 (GT frozen at `56f448a`, lines shifted) | `EVALUATION.md:90`; `multi-repo...md:52-56` |
| F6 | Important | G13 byte reduction −64.8% → actually −62.7% (dropped in-bytes) | `EVALUATION.md:94`; `g13...md:18-23` |
| F7 | Important | G13 41.7% is vs grep baseline; outline arm is **+1 call** vs plain-kg control | `README.md:27`; `g13...md:18-30` |
| F8 | Important | Routing table mixes n=10 and n=9 denominators under one header | `EVALUATION.md:148-156` |
| F9 | Important | "12 runs" second host = **10** primary + 2 alternate fills | `EVALUATION.md:166`; `grok-a-b:103-140` |
| F10 | Important | "42 runs" = 42 artifacts, 40 primary; -27% is n=10 vs n=10 | `README.md:26`; `EVALUATION.md:41-42` |
| F11 | Important | Routed prompts/manifests not committed; -27.3% not auditable from repo | `grok-a-b:68-79`; `git ls-files` |
| F12 | Important | "26+24-run protocol" incoherent; Stage 2 = 24 cells | `EVALUATION.md:112` |
| F13 | Important | Stage 2b not reproducible from HEAD: committed `audit.py` lacks B1/B2 (fix uncommitted); repro block runs bash via python3 | `agent-utility...md:97-104`; `git show HEAD:audit.py` |
| F14 | Important | Test count 91/91 stale; suite is 98/98 | `EVALUATION.md:93` |
| F15 | Important | Self-index table stale (24/306/904, 0.13 s → 34/355/1078, 0.16 s) | `EVALUATION.md:15,30` |
| F16 | Important | G11 (594/FAIL=0) and A11 perf table have no baselines record; evidence map points to a nonexistent section | `EVALUATION.md:55-60,92,201-202` |
| F17 | Important | "precision 100%" = 493/495 (99.6%); two failures listed | `README.md:24`; `EVALUATION.md:90` |
| F18 | Important | Two §3 verification rows unsupported (truncated-file exit 3; parallel-identical tables) | `EVALUATION.md:78,82` |
| F19 | Important | "MCP-connected CLI agent" unsupported: A/B prompts use the CLI; MCP only in G10 parity | `EVALUATION.md:42`; prompts |
| F20 | Important | README omits negative utility results (search FAIL, clones PARTIAL, host dependence) | `README.md:22-27` |
| F21-25 | Suggestion/Nit | "five deps"/"static binary"/"every subcommand" false; small numeric drifts; evidence-map defects; ">20x headroom" actually 14.7x; 60 GT rows scored as 59 | as cited |

Minimal fix order: F1 inversion → F2 re-measure/re-rank → self-index refresh →
G13 correction → denominators → Stage 2b reproducibility → test count → missing
records → precision → README framing.
