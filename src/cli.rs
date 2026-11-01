//! clap definitions and help texts. Impact direction semantics are stated in
//! the help: impact = "files that call into <file>" (spec response contract).

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "kg",
    version,
    about = "Local code knowledge index: who calls this, what does this call, what is impacted by this file.",
    long_about = "kg indexes a repository's symbols and call/inheritance edges into a local\nSQLite index. It is consultative only: no hooks, no daemon, no network,\nno writes outside .kg/."
)]
pub struct Cli {
    /// Repository root (default: nearest git root ancestor of cwd).
    /// Global: valid before or after the subcommand.
    #[arg(long, global = true)]
    pub root: Option<std::path::PathBuf>,

    /// Disable the usage journal for this invocation (Amendment 6).
    #[arg(long, global = true)]
    pub no_journal: bool,

    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Index the repository into .kg/index.db (full atomic replacement)
    Build {
        /// Re-extract everything (disable the unchanged-file fast path)
        #[arg(long)]
        force: bool,
        /// Abort when this fraction of files fails to parse (default 5%)
        #[arg(long, default_value_t = 0.05)]
        max_parse_error_rate: f64,
    },
    /// Who calls this symbol
    Callers {
        /// Symbol name or dotted qualname (Class.method form disambiguates)
        name: String,
        /// Maximum results shown (default 50, max 10000)
        #[arg(long, default_value_t = crate::query::DEFAULT_LIMIT)]
        limit: usize,
    },
    /// What does this symbol call
    Callees {
        /// Symbol name or dotted qualname (Class.method form disambiguates)
        name: String,
        /// Maximum results shown (default 50, max 10000)
        #[arg(long, default_value_t = crate::query::DEFAULT_LIMIT)]
        limit: usize,
        /// Transitive depth for the call closure (default 1 = direct callees)
        #[arg(long, default_value_t = 1)]
        depth: usize,
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
        #[arg(long, default_value_t = 10)]
        depth: usize,
    },
    /// Files that call into <file> (impact direction: callers of the file)
    Impact {
        /// Repo-relative path of the indexed file
        file: String,
        /// Maximum results shown (default 50, max 10000)
        #[arg(long, default_value_t = crate::query::DEFAULT_LIMIT)]
        limit: usize,
    },
    /// Dead-code candidates: indexed symbols with zero inbound call edges.
    /// Candidates only — dynamic dispatch and entry points are not proven absent.
    Dead {
        /// Maximum results shown (default 50, max 10000)
        #[arg(long, default_value_t = crate::query::DEFAULT_LIMIT)]
        limit: usize,
    },
    /// Per-file unresolved-callee counts (where kg could not resolve calls)
    Unresolved {
        /// How many files to show (default 50, max 10000)
        #[arg(long, default_value_t = crate::query::DEFAULT_LIMIT)]
        limit: usize,
    },
    /// Symbols of one indexed file, ordered by line
    Symbols {
        /// Repo-relative path of the indexed file
        file: String,
        /// Append the one-line signature column (Amendment 12a)
        #[arg(long)]
        outline: bool,
        /// Maximum results shown (default 50, max 10000)
        #[arg(long, default_value_t = crate::query::DEFAULT_LIMIT)]
        limit: usize,
    },
    /// Symbol-level search: bm25-ranked FTS5 over identifier tokens
    Search {
        /// Space-separated terms; camelCase/snake_case are split and matched
        terms: String,
        /// Maximum results shown (default 50, max 10000)
        #[arg(long, default_value_t = crate::query::DEFAULT_LIMIT)]
        limit: usize,
    },
    /// Duplicate-block candidates: normalized 5-line-window shingle matches
    Clones {
        /// Minimum duplicated-block lines per side (floor: the 5-line build window)
        #[arg(long, default_value_t = 5)]
        min_lines: usize,
        /// Maximum pair lines shown (default 50, max 10000)
        #[arg(long, default_value_t = crate::query::DEFAULT_LIMIT)]
        limit: usize,
    },
    /// Index totals: files, symbols, edges, parse errors, sizes
    Stats {},
    /// Pain-ranking of files: cyclomatic + LOC + churn-90d + fix-density
    Hotspots {
        /// How many files to show (default 10, max 100)
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Four-section architecture report (overview, map, hotspots, fan-in)
    Report {},
    /// Inspect index health (integrity, versions, staleness, orphans)
    Doctor {
        /// Delete the index entirely (one-command exit to grep-only)
        #[arg(long)]
        invalidate: bool,
    },
    /// Summarize the usage journal: calls, errors, latency, empty/stale rates.
    /// --verify replays sampled payload pointers against current source.
    Usage {
        /// Replay-verify sampled payload pointers (Amendment 10c)
        #[arg(long)]
        verify: bool,
        /// Verify sample size (deterministic even stride over payload entries)
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Run the MCP stdio server (newline-delimited JSON-RPC 2.0; Amendment 8)
    Mcp {},
}
