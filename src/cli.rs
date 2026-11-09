//! clap definitions and help texts. Impact direction semantics are stated in
//! the help: impact = "files that call into <file>".

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "eg",
    version,
    about = "Local code knowledge index: who calls this, what does this call, what is impacted by this file.",
    long_about = "eg indexes a repository's symbols and call/inheritance edges into a local\nSQLite index. It is consultative only: no hooks, no daemon, no network,\nno writes outside .engram/."
)]
pub struct Cli {
    /// Repository root (default: nearest git root ancestor of cwd).
    /// Global: valid before or after the subcommand.
    #[arg(long, global = true)]
    pub root: Option<std::path::PathBuf>,

    /// Disable the usage journal for this invocation.
    #[arg(long, global = true)]
    pub no_journal: bool,

    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Index the repository into .engram/index.db (full atomic replacement)
    Build {
        /// Re-extract everything (disable the unchanged-file fast path) and
        /// bypass the parse-error abort
        #[arg(long)]
        force: bool,
        /// Abort when this fraction of files fails to parse (default 5%)
        #[arg(long, default_value_t = crate::query::DEFAULT_MAX_PARSE_ERROR_RATE)]
        max_parse_error_rate: f64,
    },
    /// Who calls this symbol
    Callers {
        /// Symbol name or dotted qualname (Class.method form disambiguates)
        name: String,
        /// Maximum results shown (default 50, max 10000)
        #[arg(long, default_value_t = crate::query::DEFAULT_LIMIT)]
        limit: usize,
        /// Transitive depth for the caller closure (default 1 = direct callers)
        #[arg(long, default_value_t = crate::query::DEFAULT_DEPTH)]
        depth: usize,
    },
    /// What does this symbol call
    Callees {
        /// Symbol name or dotted qualname (Class.method form disambiguates)
        name: String,
        /// Maximum results shown (default 50, max 10000)
        #[arg(long, default_value_t = crate::query::DEFAULT_LIMIT)]
        limit: usize,
        /// Transitive depth for the call closure (default 1 = direct callees)
        #[arg(long, default_value_t = crate::query::DEFAULT_DEPTH)]
        depth: usize,
    },
    /// One-call symbol context: definition + signature, direct callers, and
    /// direct callees. Resolved static edges plus tagged ambiguous candidates.
    Context {
        /// Symbol name or dotted qualname
        name: String,
        /// Maximum callers and callees shown per section (default 10, max 50)
        #[arg(long, default_value_t = crate::query::DEFAULT_CONTEXT_LIMIT)]
        limit: usize,
    },
    /// What does this file import (outbound resolved imports)
    Imports {
        /// Repo-relative path of the indexed file
        file: String,
        /// Maximum results shown (default 50, max 10000)
        #[arg(long, default_value_t = crate::query::DEFAULT_LIMIT)]
        limit: usize,
    },
    /// Shortest call path between two symbols via resolved call edges.
    /// Resolved static edges only — dynamic dispatch may hide paths.
    /// A reverse question is the same query with swapped arguments.
    Path {
        /// Symbol the path starts from (bare name or dotted qualname)
        from: String,
        /// Symbol the path must reach (bare name or dotted qualname)
        to: String,
        /// Maximum hops to search (default 10, max 10)
        #[arg(long, default_value_t = crate::query::DEFAULT_PATH_DEPTH)]
        depth: usize,
    },
    /// Files that call into <file> (impact direction: callers of the file)
    Impact {
        /// Repo-relative path of the indexed file
        file: String,
        /// Show every call-site pointer instead of one row per caller file
        #[arg(long)]
        sites: bool,
        /// Transitive depth (default 1; max 10)
        #[arg(long, default_value_t = crate::query::DEFAULT_DEPTH)]
        depth: usize,
        /// Edge kinds: calls,imports,inherits,all (default calls)
        #[arg(long, default_value = crate::query::DEFAULT_IMPACT_EDGES)]
        edges: String,
        /// Maximum results shown (default 50, max 10000)
        #[arg(long, default_value_t = crate::query::DEFAULT_LIMIT)]
        limit: usize,
    },
    /// Per-file unresolved-callee counts (where eg could not resolve calls)
    Unresolved {
        /// List unresolved edge sites with file:line instead of per-file counts
        #[arg(long)]
        names: bool,
        /// Restrict to a repo-relative file or directory prefix
        #[arg(long)]
        path: Option<String>,
        /// How many rows/files to show (default 50, max 10000)
        #[arg(long, default_value_t = crate::query::DEFAULT_LIMIT)]
        limit: usize,
    },
    /// Symbols of one indexed file, ordered by line
    Symbols {
        /// Repo-relative path of the indexed file or directory
        file: String,
        /// Append the one-line signature column
        #[arg(long)]
        outline: bool,
        /// Maximum results shown (default 50, max 10000)
        #[arg(long, default_value_t = crate::query::DEFAULT_LIMIT)]
        limit: usize,
    },
    /// Anti-reinvention candidates: detect duplicate helpers and suggest canonical reuse
    Reuse {
        /// Target file paths or symbol names to inspect for reuse candidates
        #[arg(required = false)]
        paths: Vec<String>,
        /// Maximum candidates shown (default 10, max 100)
        #[arg(long, default_value_t = crate::query::DEFAULT_REUSE_LIMIT)]
        limit: usize,
    },
    /// Symbol-level search: bm25-ranked FTS5 over identifier tokens with interface cards
    Search {
        /// Space-separated terms; camelCase/snake_case are split and matched
        terms: String,
        /// Maximum results shown (default 50, max 10000)
        #[arg(long, default_value_t = crate::query::DEFAULT_LIMIT)]
        limit: usize,
    },
    /// Index totals: files, symbols, edges, parse errors, sizes
    Stats {
        /// Restrict counts to a repo-relative file or directory prefix
        #[arg(long)]
        path: Option<String>,
    },
    /// Inspect index health (integrity, versions, staleness, orphans)
    Doctor {
        /// Reset the derived .engram state: index, journals, locks, and orphan
        /// files. The project-memory store (.engram/memory.db) is primary data
        /// and is kept — export it with eg mem export if you want a copy.
        #[arg(long)]
        invalidate: bool,
    },
    /// Summarize the usage journal: calls, errors, latency, empty/stale rates.
    Usage {
        /// Show the empty-result split, capped-result count, and percentages
        /// from existing journal fields
        #[arg(long)]
        quality: bool,

        /// Aggregate telemetry globally across all repositories (~/.engram/usage.log)
        #[arg(long)]
        global: bool,
    },
    /// Project memory: agent-authored notes persisted in .engram/memory.db
    /// (write/query/get/delete/export/gc). Notes are candidates, never proof.
    #[command(alias = "memory")]
    Mem {
        #[command(subcommand)]
        action: MemAction,
    },
    /// Run the MCP stdio server (newline-delimited JSON-RPC 2.0)
    Mcp {
        /// Expose every tool instead of the core navigation set
        #[arg(long)]
        all_tools: bool,
    },
    /// Serve interactive telemetry, anti-reinvention, and memory dashboard
    Dashboard {
        /// Port to bind the local dashboard web server (default: 7890)
        #[arg(long, default_value_t = 7890)]
        port: u16,
        /// Host address to bind (default: 127.0.0.1)
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Open dashboard in default web browser automatically
        #[arg(long)]
        open: bool,
        /// Scope dashboard telemetry exclusively to current repository (.engram/)
        #[arg(long)]
        local: bool,
        /// Deprecated alias: global is now the default mode across all repositories
        #[arg(long)]
        global: bool,
    },
    /// Stream live telemetry and query events in terminal (ANSI stream)
    Monitor {
        /// Filter live query events to specific repository name
        #[arg(long)]
        repo: Option<String>,
        /// Filter for critical errors only (exit code >= 3)
        #[arg(long)]
        errors_only: bool,
        /// Scope monitor stream exclusively to current repository (.engram/)
        #[arg(long)]
        local: bool,
    },
}

#[derive(Subcommand)]
pub enum MemAction {
    /// Persist a note (agent-authored prose; identical content under the same
    /// key is a no-op, different content opens a new revision)
    Write {
        /// Stable identity for the note (<= 200 chars, no control characters)
        key: String,
        /// Note body, agent-authored prose (<= 8192 bytes)
        #[arg(long)]
        body: String,
        /// One of: decision, gotcha, fact, task, procedure
        #[arg(long)]
        kind: String,
        /// Whitespace-separated lowercase tags (<= 16 x 32 chars)
        #[arg(long)]
        tags: Option<String>,
        /// Whitespace-separated qualnames / repo paths the note is about (<= 16 x 256 chars)
        #[arg(long)]
        refs: Option<String>,
        /// Note scope: durable (default, never expires) or session (expires
        /// after the TTL)
        #[arg(long, default_value = "durable")]
        scope: String,
        /// Session lifetime in seconds (only with --scope session; default
        /// 604800 = 7 days)
        #[arg(long)]
        ttl: Option<u64>,
        /// Key whose current note this one replaces (cross-key supersession;
        /// unknown target exits 1, same key exits 2)
        #[arg(long)]
        supersedes: Option<String>,
        /// Persist to the cross-project global memory store (~/.engram/memory.db)
        #[arg(long)]
        global: bool,
        /// Refresh timestamp when content_hash matches an existing current note
        #[arg(long)]
        touch: bool,
    },
    /// Rank current notes for the terms (bm25 + ref-token overlap + recency)
    Query {
        /// Space-separated terms (mutually exclusive with --all)
        terms: Vec<String>,
        /// List current notes by recency instead of ranking
        #[arg(long)]
        all: bool,
        /// Restrict search to notes referencing this path or symbol
        #[arg(long = "ref")]
        reference: Option<String>,
        /// Restrict to one kind: decision, gotcha, fact, task, procedure
        #[arg(long)]
        kind: Option<String>,
        /// Restrict to one scope: durable or session
        #[arg(long)]
        scope: Option<String>,
        /// Maximum results shown (default 10, max 100)
        #[arg(long, default_value_t = crate::memory::DEFAULT_MEM_LIMIT)]
        limit: usize,
        /// With --all, also list superseded revisions (tagged [superseded])
        #[arg(long)]
        include_superseded: bool,
        /// Restrict search exclusively to the global store (~/.engram/memory.db)
        #[arg(long)]
        global: bool,
    },
    /// Refresh created_at timestamp on the active revision without revision churn
    Touch {
        /// The note's stable identity
        key: String,
        /// Touch in the cross-project global memory store (~/.engram/memory.db)
        #[arg(long)]
        global: bool,
    },
    /// Show the current note for one exact key plus its revision count
    Get {
        /// The note's stable identity
        key: String,
        /// Fetch from the cross-project global memory store (~/.engram/memory.db)
        #[arg(long)]
        global: bool,
    },
    /// Hard-delete every revision of one key (its FTS rows go with it)
    Delete {
        /// The note's stable identity
        key: String,
        /// Delete from the cross-project global memory store (~/.engram/memory.db)
        #[arg(long)]
        global: bool,
    },
    /// Write current notes to stdout as JSONL (default) or fenced Markdown
    /// (Git-versionable); the caveat goes to stderr so pipelines parse stdout
    Export {
        /// Output format: jsonl (default, restore format) or markdown
        #[arg(long)]
        format: Option<String>,
        /// Take the N most recent notes instead of all of them
        #[arg(long)]
        limit: Option<usize>,
        /// Export from the cross-project global memory store (~/.engram/memory.db)
        #[arg(long)]
        global: bool,
    },
    /// Restore notes from a JSONL or fenced-Markdown export (use '-' for stdin).
    /// Conflicting keys are skipped unless --force overwrites them.
    Import {
        /// Input file path, or '-' to read stdin
        path: String,
        /// Input format: jsonl or markdown (default: detect from path/first line)
        #[arg(long)]
        format: Option<String>,
        /// Overwrite a key whose local content differs (default: reject and skip)
        #[arg(long, alias = "overwrite")]
        force: bool,
        /// Import into the cross-project global memory store (~/.engram/memory.db)
        #[arg(long)]
        global: bool,
    },
    /// Reclaim disk: purge expired session notes and superseded revisions
    /// older than the 30-day retention clock, then report counts
    Gc {
        /// Run GC on the cross-project global memory store (~/.engram/memory.db)
        #[arg(long)]
        global: bool,
    },
}
