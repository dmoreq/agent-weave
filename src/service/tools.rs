//! MCP tool registry: parameter specs and the `TOOL_SPECS` table mirrored
//! one-to-one with the CLI surface.

pub const MAX_STR_ARRAY_ITEMS: usize = 1000;

/// JSON type of one MCP tool argument.
pub enum ParamKind {
    Str,
    Int {
        min: i64,
        max: Option<i64>,
    },
    Float {
        min: f64,
        max: f64,
    },
    Bool,
    /// Array of strings, at most 1000 entries.
    StrArray,
}

pub struct ParamSpec {
    pub name: &'static str,
    pub kind: ParamKind,
    pub required: bool,
    pub description: &'static str,
}

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub params: &'static [ParamSpec],
    /// Default `eg mcp` surface (without `--all-tools`): the eight core
    /// navigation tools plus mem_write/mem_query (OQ1 owner decision,
    /// 2026-11-05). One source of truth — a spec without this flag stays
    /// behind `--all-tools` and cannot silently drop off the default set.
    pub core: bool,
}

const LIMIT_PARAM: ParamSpec = ParamSpec {
    name: "limit",
    kind: ParamKind::Int {
        min: 1,
        max: Some(crate::query::MAX_LIMIT as i64),
    },
    required: false,
    description: "Maximum results shown (default 50)",
};

const DEPTH_PARAM: ParamSpec = ParamSpec {
    name: "depth",
    kind: ParamKind::Int {
        min: 1,
        max: Some(crate::query::MAX_DEPTH as i64),
    },
    required: false,
    description: "Transitive depth (default 1)",
};

const NAME_PARAM: ParamSpec = ParamSpec {
    name: "name",
    kind: ParamKind::Str,
    required: true,
    description: "Symbol name or dotted qualname",
};

const FILE_PARAM: ParamSpec = ParamSpec {
    name: "file",
    kind: ParamKind::Str,
    required: true,
    description: "Repo-relative path of the indexed file",
};

const KEY_PARAM: ParamSpec = ParamSpec {
    name: "key",
    kind: ParamKind::Str,
    required: true,
    description: "The note's stable identity",
};

pub const TOOL_SPECS: &[ToolSpec] = &[
    ToolSpec {
        name: "build",
        description: "Index the repository into .engram/index.db (full atomic replacement)",
        params: &[
            ParamSpec {
                name: "force",
                kind: ParamKind::Bool,
                required: false,
                description: "Re-extract everything (disable the fast path) and bypass the parse-error abort",
            },
            ParamSpec {
                name: "max_parse_error_rate",
                kind: ParamKind::Float {
                    min: 0.0,
                    max: 1.0,
                },
                required: false,
                description: "Abort above this fraction of parse-error files (default 0.05)",
            },
        ],
        core: true,
    },
    ToolSpec {
        name: "callers",
        description: "Who calls this symbol — direct by default; transitive with depth; pointer list path:line",
        params: &[
            NAME_PARAM,
            DEPTH_PARAM,
            LIMIT_PARAM,
        ],
        core: true,
    },
    ToolSpec {
        name: "callees",
        description: "What does this symbol call — transitive with depth",
        params: &[
            NAME_PARAM,
            DEPTH_PARAM,
            LIMIT_PARAM,
        ],
        core: true,
    },
    ToolSpec {
        name: "path",
        description: "Shortest call path between two symbols — hop call-site pointers path:line over resolved edges; dynamic dispatch may hide paths",
        params: &[
            ParamSpec { name: "from", kind: ParamKind::Str, required: true, description: "Symbol the path starts from" },
            ParamSpec { name: "to", kind: ParamKind::Str, required: true, description: "Symbol the path must reach" },
            ParamSpec { name: "depth", kind: ParamKind::Int { min: 1, max: Some(10) }, required: false, description: "Maximum hops (default 10)" },
        ],
        core: true,
    },
    ToolSpec {
        name: "context",
        description: "One-call symbol context: definition + signature, direct callers, direct callees over resolved edges plus tagged ambiguous candidates",
        params: &[
            NAME_PARAM,
            ParamSpec { name: "limit", kind: ParamKind::Int { min: 1, max: Some(50) }, required: false, description: "Callers/callees shown per section (default 10)" },
        ],
        core: true,
    },
    ToolSpec {
        name: "imports",
        description: "What does this file import — outbound resolved imports; external/unresolved omitted",
        params: &[
            FILE_PARAM,
            LIMIT_PARAM,
        ],
        core: true,
    },
    ToolSpec {
        name: "impact",
        description: "Files that call into <file> — one row per caller file with a site count; an empty calls-only result names import/inherit counts; sites=true lists every call site; edges/depth extend the dependency set",
        params: &[
            FILE_PARAM,
            ParamSpec { name: "sites", kind: ParamKind::Bool, required: false, description: "List every call site instead of one row per file" },
            DEPTH_PARAM,
            ParamSpec { name: "edges", kind: ParamKind::Str, required: false, description: "Edge kinds: calls,imports,inherits,all (default calls)" },
            LIMIT_PARAM,
        ],
        core: true,
    },
    ToolSpec {
        name: "symbols",
        description: "Symbols of one indexed file or directory, ordered by line; outline adds signatures",
        params: &[
            ParamSpec { name: "file", kind: ParamKind::Str, required: true, description: "Repo-relative path of the indexed file or directory" },
            ParamSpec { name: "outline", kind: ParamKind::Bool, required: false, description: "Append the one-line signature column" },
            LIMIT_PARAM,
        ],
        core: true,
    },
    ToolSpec {
        name: "reuse",
        description: "Calibrated anti-reinvention candidates: detect duplicate helpers and suggest canonical reuse",
        params: &[
            ParamSpec {
                name: "paths",
                kind: ParamKind::StrArray,
                required: false,
                description: "File paths or symbols to inspect (default: entire index)",
            },
            ParamSpec {
                name: "limit",
                kind: ParamKind::Int { min: 1, max: Some(100) },
                required: false,
                description: "Maximum candidates shown (default 10)",
            },
            ParamSpec {
                name: "deep_callee",
                kind: ParamKind::Bool,
                required: false,
                description: "Extend the callee footprint to second-hop callees (weight 0.5)",
            },
            ParamSpec {
                name: "judge",
                kind: ParamKind::Bool,
                required: false,
                description: "Ask the local eg-judge model to arbitrate 45-75% overlap candidates",
            },
        ],
        core: true,
    },
    ToolSpec {
        name: "search",
        description: "Symbol-level search: bm25 and caller-ranked concise interface cards",
        params: &[
            ParamSpec { name: "terms", kind: ParamKind::Str, required: true, description: "Space-separated terms; camelCase/snake_case are split and matched" },
            LIMIT_PARAM,
        ],
        core: true,
    },
    ToolSpec {
        name: "usage",
        description: "Telemetry & performance summary: total calls, p50/p95 latency, error rates, token savings, and anti-reinvention effectiveness",
        params: &[
            ParamSpec {
                name: "quality",
                kind: ParamKind::Bool,
                required: false,
                description: "Include latency and error breakdown (default true)",
            },
            ParamSpec {
                name: "global",
                kind: ParamKind::Bool,
                required: false,
                description: "Aggregate telemetry globally across all repositories (~/.engram/usage.log) (default false)",
            },
        ],
        core: true,
    },
    ToolSpec {
        name: "mem_write",
        description: "Persist a project-memory note (agent-authored prose, never source dumps) — identical content under the same key is a no-op; different content opens a new revision",
        params: &[
            ParamSpec {
                name: "key",
                kind: ParamKind::Str,
                required: true,
                description: "Stable identity for the note (<= 200 chars, no control characters)",
            },
            ParamSpec {
                name: "body",
                kind: ParamKind::Str,
                required: true,
                description: "Note body, agent-authored prose (<= 8192 bytes)",
            },
            ParamSpec {
                name: "kind",
                kind: ParamKind::Str,
                required: true,
                description: "One of: decision, gotcha, fact, task, procedure",
            },
            ParamSpec {
                name: "tags",
                kind: ParamKind::Str,
                required: false,
                description: "Whitespace-separated lowercase tags (<= 16 x 32 chars)",
            },
            ParamSpec {
                name: "refs",
                kind: ParamKind::Str,
                required: false,
                description: "Whitespace-separated qualnames / repo paths the note is about",
            },
            ParamSpec {
                name: "scope",
                kind: ParamKind::Str,
                required: false,
                description: "durable (default, never expires) or session (expires at now + ttl, default 7 days)",
            },
            ParamSpec {
                name: "ttl",
                kind: ParamKind::Int {
                    min: 1,
                    max: None,
                },
                required: false,
                description: "Session lifetime in seconds; only valid with scope=session (default 604800 = 7 days)",
            },
            ParamSpec {
                name: "supersedes",
                kind: ParamKind::Str,
                required: false,
                description: "Key whose current note this one replaces (cross-key supersession; unknown target is exit 1)",
            },
            ParamSpec {
                name: "global",
                kind: ParamKind::Bool,
                required: false,
                description: "Write to the cross-project global memory store (~/.engram/memory.db) instead of the local repo",
            },
            ParamSpec {
                name: "touch",
                kind: ParamKind::Bool,
                required: false,
                description: "Refresh timestamp when content_hash matches an existing current note",
            },
        ],
        core: true,
    },
    ToolSpec {
        name: "mem_query",
        description: "Rank current project-memory notes for the terms (bm25 + ref-token overlap + recency, deterministic); all=true lists by recency instead",
        params: &[
            ParamSpec {
                name: "terms",
                kind: ParamKind::Str,
                required: false,
                description: "Space-separated terms (mutually exclusive with all)",
            },
            ParamSpec {
                name: "all",
                kind: ParamKind::Bool,
                required: false,
                description: "List current notes by recency instead of ranking",
            },
            ParamSpec {
                name: "ref",
                kind: ParamKind::Str,
                required: false,
                description: "Restrict search to notes referencing this path or symbol",
            },
            ParamSpec {
                name: "kind",
                kind: ParamKind::Str,
                required: false,
                description: "Restrict to one kind: decision, gotcha, fact, task, procedure",
            },
            ParamSpec {
                name: "scope",
                kind: ParamKind::Str,
                required: false,
                description: "Restrict to one scope: durable or session",
            },
            ParamSpec {
                name: "limit",
                kind: ParamKind::Int {
                    min: 1,
                    max: Some(100),
                },
                required: false,
                description: "Maximum results shown (default 10)",
            },
            ParamSpec {
                name: "include_superseded",
                kind: ParamKind::Bool,
                required: false,
                description: "With all=true, also list superseded revisions (tagged [superseded])",
            },
            ParamSpec {
                name: "global",
                kind: ParamKind::Bool,
                required: false,
                description: "Restrict search exclusively to the global store (default: searches both local and global)",
            },
        ],
        core: true,
    },
    ToolSpec {
        name: "mem_touch",
        description: "Refresh created_at timestamp on the active revision without revision churn",
        params: &[
            KEY_PARAM,
            ParamSpec {
                name: "global",
                kind: ParamKind::Bool,
                required: false,
                description: "Touch in the cross-project global memory store",
            },
        ],
        core: false,
    },
    ToolSpec {
        name: "mem_get",
        description: "Show the current project-memory note for one exact key plus its revision count",
        params: &[
            KEY_PARAM,
            ParamSpec {
                name: "global",
                kind: ParamKind::Bool,
                required: false,
                description: "Fetch from global memory store",
            },
        ],
        core: false,
    },
    ToolSpec {
        name: "mem_delete",
        description: "Hard-delete every revision of one project-memory key (its search-index rows go with it)",
        params: &[
            KEY_PARAM,
            ParamSpec {
                name: "global",
                kind: ParamKind::Bool,
                required: false,
                description: "Delete from global memory store",
            },
        ],
        core: false,
    },
    ToolSpec {
        name: "mem_gc",
        description: "Reclaim project-memory disk: purge expired session notes and superseded revisions older than the 30-day retention clock, then report counts",
        params: &[ParamSpec {
            name: "global",
            kind: ParamKind::Bool,
            required: false,
            description: "Run GC on global memory store",
        }],
        core: false,
    },
    ToolSpec {
        name: "mem_export",
        description: "Write current project-memory notes to the result text as JSONL (default) or fenced Markdown; the caveat is reported separately so the payload parses cleanly",
        params: &[
            ParamSpec {
                name: "format",
                kind: ParamKind::Str,
                required: false,
                description: "Output format: jsonl (default) or markdown",
            },
            ParamSpec {
                name: "limit",
                kind: ParamKind::Int {
                    min: 1,
                    max: None,
                },
                required: false,
                description: "Take the N most recent notes instead of all of them",
            },
            ParamSpec {
                name: "global",
                kind: ParamKind::Bool,
                required: false,
                description: "Export from global memory store",
            },
        ],
        core: false,
    },
    ToolSpec {
        name: "mem_import",
        description: "Restore project-memory notes from a JSONL or fenced-Markdown file (path, or '-' for stdin); conflicting keys are skipped unless force overwrites them. MCP writes are confined to the project store",
        params: &[
            ParamSpec {
                name: "path",
                kind: ParamKind::Str,
                required: true,
                description: "Input file path, or '-' to read stdin (repo-relative over MCP)",
            },
            ParamSpec {
                name: "format",
                kind: ParamKind::Str,
                required: false,
                description: "Input format: jsonl or markdown (default: detect from path/first line)",
            },
            ParamSpec {
                name: "force",
                kind: ParamKind::Bool,
                required: false,
                description: "Overwrite a key whose local content differs (default: reject and skip)",
            },
        ],
        core: false,
    },
];
