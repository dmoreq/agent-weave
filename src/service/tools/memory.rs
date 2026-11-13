//! Memory tool schemas, kept in a submodule so `tools.rs` stays under the
//! 500-line hygiene cap. Each item is a `const` referenced by `TOOL_SPECS`.

use super::*;

pub(super) const MEM_WRITE: ToolSpec = ToolSpec {
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
                description: "Refresh created_at (last asserted) when content_hash matches an existing current note",
            },
            ParamSpec {
                name: "confirm",
                kind: ParamKind::Bool,
                required: false,
                description: "Required for reuse-allow/ notes over MCP",
            },
        ],
        core: true,
    };

pub(super) const MEM_QUERY: ToolSpec = ToolSpec {
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
            ParamSpec {
                name: "since",
                kind: ParamKind::Str,
                required: false,
                description: "Lower bound on created_at: YYYY-MM-DD, YYYY-MM-DDTHH:MM:SSZ, or @unix-seconds",
            },
            ParamSpec {
                name: "until",
                kind: ParamKind::Str,
                required: false,
                description: "Exclusive upper bound on created_at: YYYY-MM-DD, YYYY-MM-DDTHH:MM:SSZ, or @unix-seconds",
            },
            ParamSpec {
                name: "local",
                kind: ParamKind::Bool,
                required: false,
                description: "Restrict to the project store only (mutually exclusive with global)",
            },
            ParamSpec {
                name: "explain",
                kind: ParamKind::Bool,
                required: false,
                description: "Add one causal why: line per shown ranked note",
            },
            ParamSpec {
                name: "legacy",
                kind: ParamKind::Bool,
                required: false,
                description: "Use legacy ASCII tokens (diagnostic; bypasses the Unicode index)",
            },
        ],
        core: true,
    };

pub(super) const MEM_TOUCH: ToolSpec = ToolSpec {
    name: "mem_touch",
    description:
        "Refresh anchored_at (last verified) on the active revision without revision churn",
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
};

pub(super) const MEM_GET: ToolSpec = ToolSpec {
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
};

pub(super) const MEM_DELETE: ToolSpec = ToolSpec {
    name: "mem_delete",
    description:
        "Hard-delete every revision of one project-memory key (its search-index rows go with it)",
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
};

pub(super) const MEM_GC: ToolSpec = ToolSpec {
        name: "mem_gc",
        description: "Reclaim project-memory disk: purge expired session notes and superseded revisions older than the 30-day retention clock, then report counts",
        params: &[ParamSpec {
            name: "global",
            kind: ParamKind::Bool,
            required: false,
            description: "Run GC on global memory store",
        }],
        core: false,
    };

pub(super) const MEM_EXPORT: ToolSpec = ToolSpec {
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
            ParamSpec {
                name: "since",
                kind: ParamKind::Str,
                required: false,
                description: "Lower bound on created_at: YYYY-MM-DD, YYYY-MM-DDTHH:MM:SSZ, or @unix-seconds",
            },
            ParamSpec {
                name: "until",
                kind: ParamKind::Str,
                required: false,
                description: "Exclusive upper bound on created_at: YYYY-MM-DD, YYYY-MM-DDTHH:MM:SSZ, or @unix-seconds",
            },
        ],
        core: false,
    };

pub(super) const MEM_IMPORT: ToolSpec = ToolSpec {
        name: "mem_import",
        description: "Restore project-memory notes from a JSONL or fenced-Markdown file; CLI also accepts '-' for stdin. Conflicting keys are skipped unless force overwrites them. MCP writes are confined to the project store",
        params: &[
            ParamSpec {
                name: "path",
                kind: ParamKind::Str,
                required: true,
                description: "Repo-relative input file over MCP; CLI also accepts '-' for stdin",
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
    };

pub(super) const MEM_REINDEX: ToolSpec = ToolSpec {
    name: "mem_reindex",
    description: "Rebuild the derived Unicode memory index; notes are untouched",
    params: &[ParamSpec {
        name: "global",
        kind: ParamKind::Bool,
        required: false,
        description: "Rebuild the cross-project global memory store",
    }],
    core: false,
};
