# kg P4 Amendment — Agent-Tooling Features Distilled from Reference Implementations

- Status: Approved (owner approved OQ4a + OQ4b, 2026-11-01). Spec review R4a:
  independent Goldfish reviewer, round 1 NEEDS FIXES (12 findings), fix cycle 1,
  round 2 APPROVE (0 open Critical + Important). Record:
  `docs/baselines/reviews/R4-P4.md`.
- Date: 2026-11-01
- Canonical location: `docs/designs/2026-10-31_kg-rust-code-index.md`
  (Amendment 12 + Gate Registry G13-G16). This file is the self-contained
  evidence and design companion.
- Golden references: `dmoreq/pi-sherlock` v0.1.0 (clone analyzed at commit
  57b6071, 2026-05-27), ast-grep CLI (local), semgrep 1.157.0 (local).
- Measured evidence: session 2026-10-31/11-01 on repo `~/Workspace/kg` at an
  unstated pre-review tree revision. Session-temp commands are NOT re-runnable
  from this document alone except where re-measured and noted. Reviewer
  re-measurement note (2026-11-01, current tree): E3 re-run — ast-grep returns
  15 complete multiline matches (not 7); `insights.rs` query_map sites now at
  lines 115/185/204; `query.rs:61-63` unchanged; `rg` still 15 line-hits.
  Conclusion unchanged; figures marked session-dated.

## 1. Evidence base (measured, commands attached; session-dated where marked)

| # | Measurement | Result | Command / reproducibility |
|---|-------------|--------|---------------------------|
| E1 | Skeleton-read byte savings across kg src+tests (session-dated) | 37.3% aggregate (197,549B -> 123,856B); per-file range -44.4% (`src/error.rs`) to +85.6% (`src/query.rs`). Aggregate includes negative files; it is not a per-file mean and implies nothing about any specific file class | bun script invoking `pi-sherlock/src/smart-reader.ts` (session-temp, not committed) |
| E2 | Skeleton output includes FULL function bodies (mechanism verified in-clone by reviewer: `src/read-router.ts:173` pattern `fn $NAME($$$PARAMS) $$$` captures the body) | `error.rs` 2,860B input -> 4,129B output; output text shows complete `match` bodies | session script + reviewer in-clone verification |
| E3 | ast-grep captures multiline call chains in one roundtrip (session-dated; re-measured at review, see preamble) | `$STMT.query_map($$$ARGS)` returned complete call expressions incl. closure bodies; `rg` returned 15 bare line-hits | `ast-grep run -p '$STMT.query_map($$$ARGS)' -l rust src`; `rg -c query_map src` |
| E4 | semgrep latency floor (session-dated) | `--version` alone: 1.74s wall (Python startup); 2-rule scan of kg src: 1.92s, 28 findings, 0 errors | `time semgrep --version`; `time semgrep scan --config <rules.yml> --json src` — rules.yml attached below (session-temp) |
| E5 | semgrep AST-rule precision (session-dated, same rules.yml) | `$X.unwrap()` -> 24 findings vs 26 fixed-string rg matches (2 outside AST); the `dynamic-sql-format` pattern-regex rule found 4 findings, byte-identical in count and sites to `rg -U 'format!\(\s*"(SELECT|INSERT|UPDATE|DELETE)' -c src` (4) | `rg -F -c ".unwrap(" src`; `rg -U <regex> -c src`; findings JSON cross-check |
| E6 | pi-sherlock SmartRouter is dead code at runtime (verified in-clone) | `route()` called only at `src/plugin.ts:101` with constant `"routing readiness ping"`; result discarded; no consumer | `grep -rn "smartRouter\.\|\.route(" src/` |
| E7 | Routing-hot-path defect (verified in-clone) | `src/routing/context-analyzer.ts:166` — `if` with no body; line 167 swallowed as its consequent; glob patternType branch never returns | source inspection |
| E8 | Prompt tax of 14 tools, byte counts only (honesty rule — no token claims) | 14 `prompts/*.md` = 25,603 chars; `guidelines-config.json` = 14,837 chars. In-clone consumers: `src/plugin.ts:68-75` reads the prompt files; `src/routing/prompt-builder.ts:5` imports the guidelines config. Session observation (outside the clone): the host `pi` injects both into its system prompt | `wc -c prompts/*.md src/routing/guidelines-config.json` |
| E9 | Skill-contract drift (verified in-clone) | `skills/pi-sherlock-read-enhanced/SKILL.md:26,94` claims skeleton has "no bodies"; behavior returns full bodies (see E2) | side-by-side read |
| E10 | BM25 corpus cliff (verified in-clone) | corpus capped at `src/shared/constants.ts:28` `DEFAULT_MAX_FILES = 200` (readdir order), files > 50,000 bytes excluded, no truncation marker in collector or BM25 render (the only `truncated` marker belongs to the grep tool) | `src/shared/fs.ts` |
| E11 | shell_enhanced broken translation (verified in-clone) | `src/shell/pipeline-enhancer.ts:77`: `git log` -> `git log --format=json | from json | to json`; `--format=json` emits literal "json" per commit; `from json` fails | source inspection |

E4/E5 rule content (attached for reproducibility; session-temp path
`/tmp/sherlock-eval/rules.yml`):

```yaml
rules:
  - id: dynamic-sql-format
    languages: [rust]
    severity: WARNING
    message: SQL built via format! — verify parameterization
    pattern-regex: 'format!\(\s*"(SELECT|INSERT|UPDATE|DELETE)'
  - id: panic-unwrap
    languages: [rust]
    severity: INFO
    message: unwrap() can panic
    pattern: $X.unwrap()
```

## 2. Proposed features (each names the golden reference it improves on)

### F1 — `kg symbols <file> --outline` : signature column on the existing surface (schema v4, additive) — APPROVED

- Golden reference: pi-sherlock `read_enhanced` skeleton mode (E1 aggregate
  savings) and its verified defect (E2: bodies returned; E9: skill text
  drifts from behavior). F1 takes the demand signal and drops the defect.
- Surface: NO new subcommand. Amendment 5's `kg symbols <file>` already emits
  `path:line kind qualname` ordered by `line_start` with the single-file
  staleness header. F1 extends it with an `--outline` flag that appends a
  signature column. Default output (no flag) stays byte-identical; the
  additive flag ships with the response-contract bump note.
- Mechanism: `symbols.signature TEXT NOT NULL DEFAULT ''` (schema v4,
  additive-only policy), captured at extraction from the already-parsed
  tree-sitter node: first line of the node's text, trimmed, hard cap 160
  chars, no bodies (the E2 defect class, excluded by construction).
- Non-goal amendment (OQ4a — approved 2026-11-01): "No verbatim source
  responses" becomes "no verbatim source *bodies*; signature lines capped at
  160 chars are allowed".
- Wiring: MCP `symbols` tool gains optional `outline` arg (Amendment 8
  mapping); G10 parity fixtures extended (byte-identical result text with and
  without). Journal: covered by the existing `symbols` entry (Amendment 6) —
  the journal `args` shape gains the boolean `outline` key. Payload: covered
  by the existing `symbols` entry (Amendment 10).
- Gate G13 (kill-able): D0 harness (`docs/baselines/harness/`) run with the 5
  standard architecture tasks in outline-first vs read-first mode; pass
  >= 30% tool-call reduction (same bar as G8) and no correctness loss.
  Pass command: the committed harness (`g` logger pattern + `count.py` +
  `check.py`) over a new `transcripts/kgoutlinearm/` arm. Fail -> schema v4
  ships without the signature column (collapse-before-release rule).

### F2 — `kg search <terms>` : symbol-level FTS5 lookup (additive; deferral lifted) — APPROVED

- Golden reference: pi-sherlock `concept_search` — demand for NL discovery is
  real; the implementation is defective (E10: silent 200-file cap, per-call
  reindex, file-level granularity, no camelCase tokenization). kg improves on
  it because the index is the corpus (no collection step, no cap cliff) and
  granularity is the symbol, not the file.
- Deferral handling (OQ4b — approved 2026-11-01): the Non-Goals deferral of
  `search` is lifted for the symbol-search slice only. Content search stays
  out of scope (rg remains the content tool — the existing deferral rationale
  is unchanged for content).
- Mechanism: standalone FTS5 virtual table `symbols_fts` populated by ONE
  `INSERT ... SELECT` from `symbols` at build end (covers fresh and carried
  rows; no per-row dual writes). Indexed text = lowercased
  camelCase/snake_case-split `qualname + name` (preprocessing helper shared
  by insert and query). FTS5 native `bm25()` ranking; rusqlite `bundled`
  compiles FTS5 (rusqlite 0.41.1 / libsqlite3-sys 0.39.1, no feature change).
- Pointer contract: search hits are symbol identities, not call edges — the
  tier field position is EMPTY (omitted). Only call-edge pointers carry
  ` [ambiguous]`. Documented in the contract-bump note. Cap, staleness
  header, and exit codes per the standard contract.
- Wiring: new subcommand -> Amendment 6 journal list (`args` gains the
  free-text `terms` string key), Amendment 10 payload list
  (pointer-returning), Amendment 8 MCP tool mapping (`search` tool); G10
  parity fixtures extended.
- Schema: the single schema v4 bump carries ALL landed P4 additive changes.
  Kills collapse before release. A post-release drop of a feature's table is
  NOT additive removal — it is a schema v5 rebuild under the version policy.
- Gate G14 (kill-able): recall >= 70% on a frozen 20-query NL-navigation list
  (`docs/baselines/search-gt.jsonl`, frozen BEFORE implementation), FPR
  surfaced alongside with Wilson 95% interval (G-A conventions). Pass
  command: `python3 tests/scripts/search_recall.py --queries docs/baselines/search-gt.jsonl <kg-binary> --kg-root <kg-repo> --vihap-root <vihap-worktree>`. Fail -> F2 dropped before
  release (schema v4 ships without `symbols_fts`).

### F3 — Skill wrapper hardening (OQ2; CI conformance check, not a kill-able gate) — APPROVED

- Golden reference: pi-sherlock ships 14 skills + 14 prompt files (E8:
  40,440 chars of injected guidance) whose router is dead code (E6, E7) and
  whose skill text drifted from behavior (E9). The lesson for kg: routing
  lives in ONE honest, static decision table, and skill text must be gated
  against implementation.
- Scope update (post-exploration): the `code-index` skill ALREADY exists in
  the agent-skills repo (`~/Workspace/personal/agent-skills`, commit d0c6136;
  count asserts updated to 28/36/39, usage text "all 39"). F3 is therefore
  HARDENING, not creation: audit the SKILL.md trigger table against all
  subcommands, add the "Do NOT use for" column, grep-fallback guidance for
  dynamic dispatch, output-contract summary; size budget <= 3,000 bytes
  (raised from the proposed 2,500: the agent-skills repo invariant requires a
  '## Related Skills' handoff section with >= 2 outgoing + >= 2 incoming
  links; repo schema bound 12KB/250 lines unchanged).
- Drift gate: `tests/skill_contract_drift.py` in the agent-skills repo —
  env `KG_BIN` points at a kg binary; renders `kg --help` + each
  subcommand's `--help`; asserts every subcommand/flag mentioned in the
  skill text exists in the binary's help output; skips when `KG_BIN` is
  unset (the agent-skills CI cannot build the private kg repo — enforcement
  is the owner's local pre-commit check when editing the skill; this
  limitation is recorded here). Pass command (local): `KG_BIN=<path-to-kg>
  python3 tests/skill_contract_drift.py`.

### F4 — `kg clones` : duplicate-block candidates (optional, last; schema v4 shared) — APPROVED

- Golden reference: pi-sherlock `find_duplicates` (jscpd) — validated demand
  for copy-paste detection in refactor workflows. jscpd itself is a Node
  tool with its own report stack; kg implements token-shingle hashing
  (FNV-1a already in-crate, `src/hash.rs`) over normalized line windows, no
  new dependency.
- Mechanism: schema v4 additional table `clone_groups(shingle_hash, file_id,
  line_start, line_end)` computed at build for fresh files and carried
  forward with the fast path; `kg clones [--min-lines N]` (default 5) emits
  pointer pairs under the standard cap/staleness contract. Candidate-list
  semantics identical to `kg dead`: mandatory first-line caveat
  "kg: clone candidates — normalized-text equality, not proof of semantic
  duplication".
- Wiring: new subcommand -> Amendment 6 journal list (`args` gains
  `min_lines`), Amendment 8 MCP tool mapping (`clones` tool); G10 parity
  fixtures extended. NOT in the Amendment 10 payload list — documented
  deviation: pair lines are not single pointers and `usage --verify`
  structural checks are undefined for pairs.
- Gate G16 (kill-able): seeded-corpus precision — inject 10 known clone
  pairs into the fixture tree; require >= 8/10 detected with <= 20% spurious
  groups; build-time overhead <= 25% wall on G1's corpus (bar amended from
  the proposed 15% by owner decision 2026-11-01: the measured cost is
  clone-row INSERT I/O — attribution in `docs/baselines/g16-clones-2026-11-01.md`).
  Pass command:
  `cargo test clones_seed` + interleaved before/after build timing (A/B rounds
  on identical corpora; hyperfine if installed — the executed measurement used
  a 30-round interleaved python timing loop, see the G16 report). Fail -> F4 dropped before release (schema v4 ships without
  `clone_groups`).
- Priority: explicitly last; only after G13 and G14 pass.

## 3. Evaluated and rejected (with the evidence that rejects them)

| Rejected | Golden reference | Reason |
|----------|------------------|--------|
| semgrep-class audit surface | semgrep 1.157.0 (E4, E5) | Out of charter (three navigation questions); 1.74s Python startup floor per call; registry rule packs require network — kg's zero-network invariant (G-D/G-E) forbids wrapping it. Agents can run semgrep themselves when a task warrants it. |
| tokei-class LOC tool | tokei via pi-sherlock `count_lines` (`src/tokei.ts:29`) | Already covered: `files.loc` + `files.cyclomatic` stored at build (Amendment 11), surfaced by `stats`/`hotspots`. |
| fd/fzf/rg/bat/nu/broot/jq/yq wrappers | pi-sherlock 12-binary surface | kg must remain a single binary with the pinned dependency set (G-E as amended: five original + `serde_json` = six, Amendment 8); these are agent-side tools outside kg's charter. pi-sherlock's own fallback-chain metadata (`method`, `fallbacks[]` — `src/read-render.ts:31,38`) is the only pattern worth keeping, and kg's tier tags + staleness headers already provide it. |
| write/edit/shell enhanced tools, incl. `ast_editor` | pi-sherlock enhanced core + `src/ast-editor.ts`, `src/ast-editor/` | kg is consultative and read-only over source (safety invariant 3) — inviolable. Additionally `shell_enhanced`'s POSIX->Nushell translation is verified broken for `git log` (E11). |
| Smart-routing layer | pi-sherlock `src/routing/` (1,365 lines) | Dead code at runtime (E6) with a verified defect (E7). kg's routing is the static decision table in F3 — exactly what actually worked in the golden reference. |
| File-topology listing | pi-sherlock `ls_enhanced` (broot, `src/broot/`) | Outside navigation charter; agent's own ls/find covers it. |
| Char/line truncation markers | pi-sherlock `shared/truncate.ts` | kg already bounds output (cap contract, `showing N of M`, stderr note). The pointer format keeps results short by construction. |

## 4. Acceptance criteria carried into implementation (R4b review will gate on these)

- Every feature claim cites verifiable evidence: a golden-reference file path
  (+ line) in the analyzed clone, or a measurement with its command from
  Section 1, or an explicit session-dated caveat.
- Every kill-able gate (G13, G14, G16) carries a concrete pass command and a
  stated kill action. G15/F3 is a CI conformance check, not a kill-able gate.
- No feature violates the four safety invariants (zero network, no long-lived
  process, read-only over source, no behavior hijack) — F1's signature text
  is the sole non-goal amendment (OQ4a, approved).
- Schema: ONE schema v4 bump carries all landed P4 additive changes;
  post-release drops are schema v5 rebuilds, never in-place removals.
- Exit codes and the response contract remain byte-stable; additive
  flags/args ship with the contract-bump note; MCP G10 parity fixtures
  extended for every new arg/tool.
- Implementation review R4b bound: <= 2 fix cycles per the Per-Phase
  Independent Review Protocol.
