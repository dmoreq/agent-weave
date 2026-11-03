# Agent integration

Use the `kg-first` skill to guide navigation, and CLI or MCP access to execute queries.
These are separate integrations. Neither installs the other or changes host configuration automatically.

## Choose the access mechanism

| Integration | Provides | Does not provide |
|---|---|---|
| CLI | `kg` commands through the agent's command runner | Skill loading or routing rules |
| MCP | kg operations as tools over stdio | Skill loading or a query-first policy |
| `kg-first` skill | Query selection, result handling, and fallback rules | The kg executable, index, or MCP registration |

Use CLI access, MCP access, or both according to your host's capabilities.
The skill works with either. It does not require MCP.

## Prepare the repository

1. Follow [Getting started](getting-started.md) to install kg from a clone.
2. In the repository you want to navigate, run `kg build` and `kg doctor`.
3. Confirm that `kg symbols <file> --outline` returns locations for a known supported source file.

Replace `<file>` with a repository-relative path; it is not literal command input.
Use the same repository root for the build and subsequent queries.

## Register the skill manually

1. Read [`kg-first/SKILL.md`](../skills/kg-first/SKILL.md) and confirm its routing policy fits your workflow.
2. Check your installed host version's documentation for skill discovery or registration.
3. Register the [`skills/kg-first/`](../skills/kg-first/) directory using that mechanism, preserving `SKILL.md` and its frontmatter.
4. Reload the host or start a new session if required. Confirm the host can find and load `kg-first`.
5. Ask the agent to locate callers of a known symbol. Confirm it queries kg before broad text search.
6. Open a returned source location. Confirm the agent can use source reads or grep to investigate missing relationships.

Skill discovery and skill activation are different checks. A visible skill name does not prove its instructions reached the active session.
If the host cannot load skills, use CLI or MCP directly and follow the routing document manually.

## Register MCP manually

kg serves MCP through newline-delimited JSON-RPC over standard input and output.
It is not an HTTP server. The host starts the process; no separate background service is required.

1. Find the installed kg executable's absolute path. Confirm the host can execute it.
2. In your host's documented MCP registration interface, add a local stdio server.
3. Set the executable to that absolute path. Set separate arguments to `--root`, the target repository's absolute path, and `mcp`.
4. Let the host start the server. Confirm initialization succeeds and the tool list includes `symbols` and `callers`.
5. Call the `symbols` tool with `file` set to the known source file and `outline` set to `true`.
6. Compare the returned locations with the CLI query and current source. Confirm the repository root matches.

The equivalent process invocation is shown below. Replace both example paths before use:

```sh
/absolute/path/to/kg --root /absolute/path/to/repository mcp
```

Running this command in a terminal waits for protocol input; it is not a host-registration command.
The repository root is selected when the server starts. Restart or re-register the server to target another repository.

MCP argument names come from the tool schemas, not CLI spelling: for example, `symbols` uses `file` and `outline`.
The `doctor` command is CLI-only. MCP tool errors use `isError`; inspect the returned text before treating results as successful.

## Host verification status

No host-specific installation recipe or host version was verified for this guide.
These instructions describe kg's source-defined CLI and MCP interfaces, not a tested host configuration.

| Host | Verified host version | Setup status |
|---|---|---|
| Grok Build | Not recorded | Skill registration and MCP setup not verified here |
| OpenCode | Not recorded | Skill registration and MCP setup not verified here |
| ZCode | Not recorded | Skill registration and MCP setup not verified here |

Use the documentation for your installed host version. This guide intentionally provides no host configuration syntax or installation paths.
When validating an integration, record the host version, `kg --version`, repository revision, access mechanism, and the query result.

## Interpret results and fall back

- **Missing tools:** confirm MCP registration and initialization. Skill registration alone does not expose tools.
- **Missing executable:** use an absolute executable path and check the host's execution permissions.
- **Wrong or missing index:** verify `--root`; run `kg build` for that repository. Check index health with CLI `kg doctor`.
- **Ambiguous name:** select the intended qualified name from the candidates, then query again. Do not guess.
- **Capped results:** increase the supported limit or narrow the query. Do not treat displayed rows as a complete dependency set.
- **Empty result or no path:** inspect source or use grep. Missing resolved edges do not prove missing runtime behavior.
- **Changed code:** rebuild before further navigation. No staleness warning does not prove the entire index is current.
- **Conflicting evidence:** verify current source. Query-first routing does not prohibit safety checks or required review evidence.

For workflows, see [Usage](usage.md). For arguments, limits, exit codes, and journaling, see [CLI reference](cli-reference.md).
Read [Evaluation](EVALUATION.md) before applying measured benefits to another host or repository.

## Local data and removal

kg confines its generated data to `.kg/` and does not write agent configuration.
Queries can append local usage journals; `--no-journal` or `KG_NO_JOURNAL=1` disables journaling.
Your host controls how it stores tool results and transmits context to a model; review its privacy settings separately.

To remove integration, manually unregister the skill and MCP server through your host's documented mechanism.
Stop the host-managed kg process and start a fresh session to clear loaded instructions.
Neither step removes the repository index. CLI `kg doctor --invalidate` deletes the entire `.kg/` directory,
including its index and usage journals. Use it only when you intend to discard that local data; it does not remove host registration.

## Interface sources

- [`src/cli.rs`](../src/cli.rs): commands, flags, and repository selection.
- [`src/mcp.rs`](../src/mcp.rs): stdio transport, tool schemas, and response handling.
- [`tests/mcp.rs`](../tests/mcp.rs): MCP contract checks; not executed for this documentation update.
