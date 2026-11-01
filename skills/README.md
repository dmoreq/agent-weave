# kg skills

Agent skills for the kg code index, stored in-repo. **Installation into an
agent host is the owner's decision** — nothing here auto-installs.

## Available skills

| skill | file | purpose |
|-------|------|---------|
| kg-first | [`kg-first/SKILL.md`](kg-first/SKILL.md) | measured routing policy: kg-first navigation, ambiguity handling, caps, staleness, grep fallback rules |

## Install matrix (host-specific)

| host | action |
|------|--------|
| Grok Build | copy `kg-first/` to `~/.grok/skills/kg-first/` |
| OpenCode | add the skill body as an instructions entry in `~/.config/opencode/AGENTS.md`, or register `kg mcp` in `~/.config/opencode/opencode.json` |
| ZCode | copy `kg-first/` to `~/.zcode/skills/kg-first/` |

The skill content is host-agnostic; only the registration step differs.

## Anti-drift rule

Every command and flag named in `SKILL.md` must exist in the shipped binary's
`kg --help`. The reference lesson (spec Amendment 12, finding E9): skill text
that drifts from binary behavior produces confidently wrong agents. Before
editing the skill, run `kg --help` (and the relevant subcommand's `--help`)
and diff against the tables in the skill. A drift check
(`tests/skill_contract_drift.py` pattern) exists in the companion agent-skills
repository.
