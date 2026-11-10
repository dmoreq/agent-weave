//! Query surface: callers/callees/impact with qualname disambiguation,
//! result-set-scoped staleness, caps, tier tags, and empty-result reason lines
//! (response contract; CUJ2/CUJ3/CUJ4/CUJ7).

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use rusqlite::Connection;

use crate::db;
use crate::error::{EgError, Result};
use crate::extract::{Confidence, EdgeKind, SymbolKind};
use crate::hash::fnv1a64;

pub const DEFAULT_LIMIT: usize = 50;
/// Shared CLI/MCP surface defaults, so `cli.rs` and `service.rs::from_tool_args`
/// cannot drift apart on the same flag's default.
pub const DEFAULT_DEPTH: usize = 1;
pub const DEFAULT_CONTEXT_LIMIT: usize = 10;
pub const DEFAULT_PATH_DEPTH: usize = 10;
/// Hard ceiling on transitive traversal (`callers`/`callees`/`path`/`impact`
/// `--depth`): one constant shared by the CLI help, the MCP ParamSpec bounds,
/// and every clamp, so the surfaces cannot disagree.
pub const MAX_DEPTH: usize = 10;
/// Default candidate cap for `eg reuse` (CLI default and MCP fallback).
pub const DEFAULT_REUSE_LIMIT: usize = 10;
pub const DEFAULT_MAX_PARSE_ERROR_RATE: f64 = 0.05;
pub const DEFAULT_IMPACT_EDGES: &str = "calls";
/// Cap on rendered ambiguous-name candidates (R6f: common method names could
/// print megabytes; the total count still travels in the error).
pub const MAX_AMBIGUOUS_CANDIDATES: usize = 50;
pub const MAX_LIMIT: usize = 10_000;

/// Permanent honesty caveat for `eg path` (Amendment 13): resolved static
/// edges only; dynamic dispatch may hide paths.
const PATH_CAVEAT: &str = "eg: path — resolved static edges only; dynamic dispatch may hide paths";

/// Permanent honesty caveat for `eg context` (Amendment 17).
const CONTEXT_CAVEAT: &str =
    "eg: context — resolved static edges plus tagged [ambiguous] candidates; dynamic dispatch may hide callers or callees";

/// `impact` caveats (Amendment 19): the default calls-only shape stays
/// byte-identical; the extended shapes state their static-edge limit.
const IMPACT_EDGES_CAVEAT: &str =
    "eg: impact — resolved static edges only; unresolved imports/inherits are omitted";
const IMPACT_CALLS_CAVEAT: &str =
    "eg: impact — resolved static call edges only; dynamic dispatch may hide dependents";

/// One import statement is one site even when it resolves to both a module and
/// a symbol edge; every non-import edge remains an individual site.
fn impact_site_key() -> String {
    format!(
        "CASE WHEN e.kind = '{}' THEN 'i:' || s.id || ':' || e.site_line ELSE 'e:' || e.id END",
        EdgeKind::Imports.as_str()
    )
}

/// `eg imports` caveat (Amendment 19): outbound resolved imports only.
const IMPORTS_CAVEAT: &str =
    "eg: imports — resolved static imports only; external and unresolved imports are omitted";

mod resolve;
mod scope;
mod staleness;

use resolve::*;
pub(crate) use scope::normalize_scope;
pub(crate) use scope::scoped_params;
pub(crate) use scope::validate_path_scope;
use scope::*;
pub(crate) use staleness::file_changed;
use staleness::*;

mod analysis;
mod callgraph;
mod impact;
pub mod judge;
pub mod reuse;

pub use analysis::{search, symbols, unresolved, unresolved_names, SymbolsOpts};
pub use callgraph::{callees, callers, context, imports, path, GraphQueryOpts};
/// Re-exported for the service boundary: `--edges` is validated where the
/// request is built, not at query time.
pub(crate) use impact::parse_edge_kinds;
pub use impact::{impact, ImpactOpts};
pub use reuse::run_reuse;

/// One resolved name candidate: (path, line, qualname). Defined in `error`
/// beside the `Ambiguous` payload it fills; re-exported here so the query
/// surface and its child modules keep one spelling.
pub use crate::error::NameCandidate;

/// One symbol row for the inventory renderers.
struct SymbolRow {
    kind: String,
    qualname: String,
    line: i64,
    signature: String,
}

/// One BFS level's out-edges grouped per source: `(dst qualname, dst path,
/// site_line, dst id)` sorted by the pinned neighbor key (`path` is the
/// destination file). Amendment 13.
type LevelEdges = std::collections::HashMap<i64, Vec<(String, String, i64, i64)>>;

/// Row tag for an AMBIGUOUS-confidence edge (one spelling everywhere).
pub const AMBIGUOUS_TAG: &str = " [ambiguous]";

/// Tag an edge confidence for display: AMBIGUOUS gets the marker, resolved
/// edges stay empty.
pub fn confidence_tag(confidence: &str) -> &'static str {
    if Confidence::parse(confidence) == Some(Confidence::Ambiguous) {
        AMBIGUOUS_TAG
    } else {
        ""
    }
}

pub struct Pointer {
    pub file: String,
    pub line: i64,
    pub tag: &'static str, // "" or AMBIGUOUS_TAG
    /// Optional trailing detail rendered after the tag (search hits carry the
    /// qualname here, Amendment 12b; empty for call-edge pointers).
    pub detail: String,
}

impl Pointer {
    pub fn render(&self) -> String {
        if self.detail.is_empty() {
            format!("{}:{}{}", self.file, self.line, self.tag)
        } else {
            format!("{}:{}{} {}", self.file, self.line, self.tag, self.detail)
        }
    }
}

pub struct QueryOutput {
    pub header: Option<String>,
    pub pointers: Vec<Pointer>,
    pub shown: usize,
    pub total: usize,
    /// True when the staleness header fired on the displayed set (journal).
    pub stale: bool,
}

/// `showing N of M` stdout footer for a capped result set. `unit` is appended
/// after the total (for example "candidates"); empty keeps the plain shape.
pub fn cap_line(shown: usize, total: usize, unit: &str) -> Option<String> {
    if total <= shown {
        return None;
    }
    let suffix = if unit.is_empty() {
        String::new()
    } else {
        format!(" {unit}")
    };
    Some(format!("showing {shown} of {total}{suffix}"))
}

/// `[eg] capped at N results (M total)` stderr note, without the newline.
pub fn cap_notice(shown: usize, total: usize) -> Option<String> {
    (total > shown).then(|| format!("[eg] capped at {shown} results ({total} total)"))
}

/// Stderr form of the cap notice (trailing newline included): the one
/// cap-notice channel. Query arms return it in the stderr half of their
/// output so `service::execute` carries it in `ExecutionResult.stderr` for
/// both surfaces (the CLI prints it, MCP returns it in the error contract)
/// instead of leaking an `eprintln!` into the server process.
pub fn cap_stderr(shown: usize, total: usize) -> String {
    cap_notice(shown, total)
        .map(|notice| format!("{notice}\n"))
        .unwrap_or_default()
}

/// Line-rendered query result: stdout lines plus the stderr cap notice. The
/// stderr half keeps cap notices on the single `ArmOut.stderr` channel, and
/// `shown` carries the displayed-item count structurally so the MCP KPI footer
/// never re-parses the rendered lines.
pub struct LineQuery {
    pub lines: Vec<String>,
    pub stderr: String,
    pub shown: usize,
}

/// Prepend a caveat to any existing header without changing the default
/// (non-caveat) output.
pub fn prepend_header(out: &mut QueryOutput, caveat: &str) {
    out.header = Some(match out.header.take() {
        Some(existing) => format!("{caveat}\n{existing}"),
        None => caveat.to_string(),
    });
}

pub fn clamp_limit(limit: usize) -> Result<usize> {
    if limit == 0 {
        return Err(EgError::Usage("--limit must be >= 1".to_string()));
    }
    Ok(limit.min(MAX_LIMIT))
}

/// Shared read context for one query execution: the open index connection and
/// the repo root the staleness headers stat against. Follows the ImpactCtx
/// pattern (one context struct passed by reference, not a generic framework);
/// bodies destructure the context (`let QueryCtx { conn, .. } = *ctx;`) to
/// keep their `conn`/`root` locals.
pub struct QueryCtx<'a> {
    pub conn: &'a Connection,
    pub root: &'a Path,
}

/// Journal one QueryOutput and return its stderr cap notice: the shared tail
/// of the pointer-listing query arms (`emit` renders the joined CLI string,
/// `search_lines` renders lines). `result_count` is the shown set —
/// `collect_output` caps `pointers` to `shown`, so the two are equal by
/// construction. Each caller keeps its own stdout render and passes its
/// ambiguous-site count.
pub(crate) fn journal_output(out: &QueryOutput, ambiguous: usize) -> String {
    crate::journal::record_result(
        crate::journal::OutcomeCounts::of(out.shown, out.total),
        out.stale,
        ambiguous,
    );
    cap_stderr(out.shown, out.total)
}

/// Render a search QueryOutput (header, hits, cap footer) as stdout lines and
/// journal it. Every search exit path (exact, phrase, fuzzy) journals here, so
/// an exact-name hit is not recorded as an empty call (R7 review). The cap
/// notice rides the stderr channel, not `eprintln!`.
fn search_lines(out: QueryOutput) -> Result<LineQuery> {
    let stderr = journal_output(&out, 0);
    Ok(LineQuery {
        lines: render_output(&out).lines().map(str::to_string).collect(),
        stderr,
        shown: out.shown,
    })
}

/// Resolve a repo-relative file path and fail with the shared not-indexed
/// error. `imports` and `impact` both start with this gate.
fn indexed_file(conn: &Connection, file: &str) -> Result<String> {
    indexed_file_opt(conn, normalize_scope(file))?.ok_or_else(|| EgError::not_indexed(file))
}

/// The same files-row lookup without the not-indexed error, so `symbols` can
/// reuse the gate and fall through to its directory listing.
fn indexed_file_opt(conn: &Connection, rel: &str) -> Result<Option<String>> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM files WHERE path = ?1", [rel], |r| {
        r.get(0)
    })?;
    Ok((count > 0).then(|| rel.to_string()))
}

/// Shared tail of the pointer-listing query arms: bounded materialization,
/// post-cap staleness, journaling-ready counts. Caller invariant: `known_total`
/// must be > 0 — a caller that can produce an empty result returns its own
/// empty-reason error before calling this, because a 0 total here would
/// silently render an empty page.
fn collect_output(
    ctx: &QueryCtx<'_>,
    rows: impl Iterator<Item = std::result::Result<Pointer, rusqlite::Error>>,
    limit: usize,
    known_total: usize,
) -> Result<QueryOutput> {
    // Bounded materialization (R6f Important 5): callers pass a SQL LIMIT
    // plus a separate COUNT, so memory stays proportional to the displayed
    // set instead of the total hit count.
    let mut all: Vec<Pointer> = Vec::new();
    for r in rows {
        all.push(r?);
    }
    let shown = all.len().min(limit);
    let displayed: Vec<Pointer> = all.into_iter().take(shown).collect();
    let files: BTreeSet<String> = displayed.iter().map(|p| p.file.clone()).collect();
    let (header, stale) = staleness_header(ctx, &files);
    let out = QueryOutput {
        header,
        pointers: displayed,
        shown,
        total: known_total,
        stale,
    };
    Ok(out)
}

/// Render a QueryOutput exactly as the CLI prints it on stdout (header,
/// pointers, cap line). The cap stderr note is NOT part of this string.
pub fn render_output(out: &QueryOutput) -> String {
    let mut text = String::new();
    if let Some(h) = &out.header {
        text.push_str(h);
        text.push('\n');
    }
    for p in &out.pointers {
        text.push_str(&p.render());
        text.push('\n');
    }
    if let Some(line) = cap_line(out.shown, out.total, "") {
        text.push_str(&line);
        text.push('\n');
    }
    text
}
