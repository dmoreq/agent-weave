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
    /// Default `eg mcp` surface (without `--all-tools`): the core navigation
    /// tools plus mem_write/mem_query (OQ1 owner decision, 2026-11-05). One
    /// source of truth — a spec without this flag stays behind `--all-tools`
    /// and cannot silently drop off the default set.
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

mod memory;

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
        description: "Anti-reinvention candidates: detect duplicate helpers and suggest canonical reuse",
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
                name: "min_confidence",
                kind: ParamKind::Str,
                required: false,
                description: "Minimum confidence: low, provisional, medium, or high (default low)",
            },
            ParamSpec {
                name: "propose",
                kind: ParamKind::Str,
                required: false,
                description: "Propose a signature before writing it; paths are not allowed",
            },
            ParamSpec {
                name: "callees",
                kind: ParamKind::StrArray,
                required: false,
                description: "Proposed callee names used with propose",
            },
            ParamSpec {
                name: "explain",
                kind: ParamKind::Bool,
                required: false,
                description: "Append detector and reason codes to each candidate",
            },
            ParamSpec {
                name: "show_suppressed",
                kind: ParamKind::Bool,
                required: false,
                description: "Include allowlisted candidates and report the suppressed count",
            },
            ParamSpec {
                name: "coverage",
                kind: ParamKind::StrArray,
                required: false,
                description: "Opt-in coverage detectors: class, unresolved, tests",
            },
            ParamSpec {
                name: "judge",
                kind: ParamKind::Bool,
                required: false,
                description: "Ask a local eg-judge binary to arbitrate gray-zone candidates",
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
    memory::MEM_WRITE,
    memory::MEM_QUERY,
    memory::MEM_TOUCH,
    memory::MEM_GET,
    memory::MEM_DELETE,
    memory::MEM_GC,
    memory::MEM_EXPORT,
    memory::MEM_IMPORT,
    memory::MEM_REINDEX,
];
