# aw skills

Agent skills for the aw code index, stored in this repository. Registration
is manual and owner-controlled. aw does not install skills or write host configuration.

## Available skills

| skill | file | purpose |
|-------|------|---------|
| aw-first | [`aw-first/SKILL.md`](aw-first/SKILL.md) | Query-first navigation, ambiguity handling, result caps, staleness, and grep fallback rules |

## Register the skill

1. Install aw and build the target repository's index using [Getting started](../docs/getting-started.md).
2. Read the skill and confirm that its routing policy fits your workflow.
3. Check your host version's documentation for its supported skill discovery or registration mechanism.
4. Manually register the `aw-first/` directory through that mechanism, preserving `SKILL.md` and its frontmatter.
5. Start or reload a session as your host requires. Confirm that `aw-first` is available and loaded for navigation.
6. Ask about a known symbol. Confirm that the agent queries aw and opens the returned source location.

No host-specific installation path or configuration syntax is verified here.
See [Agent integration](../docs/agent-integration.md) for verification status, MCP setup, fallback, and removal.

## Skill routing is not MCP access

The skill tells an agent **when** to query aw and how to interpret results.
It does not provide an executable or register MCP tools. An agent can use it with CLI access alone.

MCP exposes aw operations as tools through `aw mcp`. Registering MCP does not load the skill or enforce its routing policy.

## Keep instructions aligned

Check command names and flags against the [CLI reference](../docs/cli-reference.md)
and the installed binary's `aw --help` and relevant subcommand help.
For source-only documentation review, compare `src/cli.rs` and `src/mcp.rs`; state that runtime compatibility remains unverified.

Keep routing decisions separate from metric wording changes. Read [Evaluation](../docs/EVALUATION.md)
for measurement scope and limitations; do not interpret benchmark results as complete repository coverage.
