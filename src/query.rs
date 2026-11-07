//! Query surface: callers/callees/impact with qualname disambiguation,
//! result-set-scoped staleness, caps, tier tags, and empty-result reason lines
//! (response contract; CUJ2/CUJ3/CUJ4/CUJ7).

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use rusqlite::Connection;

use crate::db;
use crate::error::{AwError, Result};
use crate::extract::{Confidence, EdgeKind, SymbolKind};
use crate::hash::fnv1a64;

pub const DEFAULT_LIMIT: usize = 50;
/// Cap on rendered ambiguous-name candidates (R6f: common method names could
/// print megabytes; the total count still travels in the error).
pub const MAX_AMBIGUOUS_CANDIDATES: usize = 50;
pub const MAX_LIMIT: usize = 10_000;

/// Permanent honesty caveat for `aw dead` (Amendment 5): candidates, not proof.
const DEAD_CAVEAT: &str =
    "aw: dead-code candidates — dynamic dispatch, entry points, star imports, module-level imports, type references, attribute references, and external callers are not proven absent";

/// Permanent honesty caveat for `aw path` (Amendment 13): resolved static
/// edges only; dynamic dispatch may hide paths.
const PATH_CAVEAT: &str = "aw: path — resolved static edges only; dynamic dispatch may hide paths";

/// Permanent honesty caveat for `aw context` (Amendment 17).
const CONTEXT_CAVEAT: &str =
    "aw: context — resolved static edges plus tagged [ambiguous] candidates; dynamic dispatch may hide callers or callees";

/// `--explain` caveat: the index stores edge metadata, not the resolution path.
pub const EXPLAIN_CAVEAT: &str =
    "aw: explain — edge metadata from the index; resolution reason is not stored";

/// Permanent honesty caveat for `aw implementers` (Amendment 19).
const IMPLEMENTERS_CAVEAT: &str = "aw: implementers — resolved static inherits edges only; dynamic inheritance and cross-file impls are omitted";

/// Permanent honesty caveat for `aw complexity` (Amendment 19).
const COMPLEXITY_CAVEAT: &str =
    "aw: complexity — static line-count ranking; not comparable to hotspots";

/// `impact` caveats (Amendment 19): the default calls-only shape stays
/// byte-identical; the extended shapes state their static-edge limit.
const IMPACT_EDGES_CAVEAT: &str =
    "aw: impact — resolved static edges only; unresolved imports/inherits are omitted";
const IMPACT_CALLS_CAVEAT: &str =
    "aw: impact — resolved static call edges only; dynamic dispatch may hide dependents";

/// One import statement is one site even when it resolves to both a module and
/// a symbol edge; every non-import edge remains an individual site.
fn impact_site_key() -> String {
    format!(
        "CASE WHEN e.kind = '{}' THEN 'i:' || ss.id || ':' || e.site_line ELSE 'e:' || e.id END",
        EdgeKind::Imports.as_str()
    )
}

/// `aw imports` caveat (Amendment 19): outbound resolved imports only.
const IMPORTS_CAVEAT: &str =
    "aw: imports — resolved static imports only; external and unresolved imports are omitted";

/// One resolved candidate: (path, line, qualname).
mod scope;

pub(crate) use scope::normalize_scope;
pub(crate) use scope::scoped_params;
pub use scope::validate_path_scope;
use scope::*;

mod analysis;
mod callgraph;
mod impact;

pub use analysis::{
    complexity, dead, files, resolve_names, search, symbols, unresolved, unresolved_names,
    SymbolsOpts,
};
pub use callgraph::{callees, callers, context, implementers, imports, path, GraphQueryOpts};
/// Re-exported for the service boundary: `--edges` is validated where the
/// request is built, not at query time.
pub(crate) use impact::parse_edge_kinds;
pub use impact::{impact, ImpactOpts};

/// One resolved name candidate: (path, line, qualname). Distinct from
/// `memory::Candidate` (a fused search-signal row), so one name keeps one
/// meaning per module.
pub type NameCandidate = (String, i64, String);

/// One symbol row for the inventory renderers.
struct SymbolRow {
    kind: String,
    qualname: String,
    line: i64,
    signature: String,
    loc: i64,
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

/// `[aw] capped at N results (M total)` stderr note, without the newline.
pub fn cap_notice(shown: usize, total: usize) -> Option<String> {
    (total > shown).then(|| format!("[aw] capped at {shown} results ({total} total)"))
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
/// stderr half keeps cap notices on the single `ArmOut.stderr` channel.
pub type LineQuery = (Vec<String>, String);

/// Prepend a caveat to any existing header without changing the default
/// (non-caveat) output.
pub fn prepend_header(out: &mut QueryOutput, caveat: &str) {
    out.header = Some(match out.header.take() {
        Some(existing) => format!("{caveat}\n{existing}"),
        None => caveat.to_string(),
    });
}

/// Prepend the `--explain` caveat to any existing header without changing the
/// default (non-explain) output.
pub fn with_explain_header(mut out: QueryOutput, explain: bool) -> QueryOutput {
    if explain {
        prepend_header(&mut out, EXPLAIN_CAVEAT);
    }
    out
}

pub fn clamp_limit(limit: usize) -> Result<usize> {
    if limit == 0 {
        return Err(AwError::Usage("--limit must be >= 1".to_string()));
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

/// One symbol row for name resolution: (id, qualname, path, line, kind).
type CandidateRow = (i64, String, String, i64, String);

fn candidate_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<CandidateRow> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
}

/// SELECT/JOIN shared by both `target_ids` statements; the column order is
/// fixed by `candidate_row`.
const CANDIDATE_SELECT: &str = "SELECT s.id, s.qualname, f.path, s.line_start, s.kind
     FROM symbols s JOIN files f ON f.id = s.file_id";

/// Exact-identity predicate shared by the resolver (`target_ids`) and the
/// search pages: a symbol matches the query by bare name or full qualname,
/// bound as parameter ?1. Parenthesized so the `OR` cannot leak into a wider
/// `AND`.
const EXACT_IDENTITY_PREDICATE: &str = "(s.name = ?1 OR s.qualname = ?1)";

/// Resolve the query name to target symbol ids. Multiple distinct symbols ->
/// exit 2 with a `path:line qualname` candidate list on stdout (CUJ4).
fn target_ids(conn: &Connection, name: &str) -> Result<(Vec<i64>, Vec<NameCandidate>)> {
    let mut stmt = conn.prepare(&format!(
        "{CANDIDATE_SELECT}
         WHERE {EXACT_IDENTITY_PREDICATE}
         ORDER BY s.qualname, f.path",
    ))?;
    let rows = stmt.query_map(rusqlite::params![name], candidate_row)?;
    let mut all: Vec<_> = rows.collect::<std::result::Result<_, _>>()?;
    // `Class.method` suffix form: the full qualname is `pkg.Class.method`, so
    // an exact name/qualname match misses it. Fall back to a suffix scan and
    // confirm the exact suffix in Rust (LIKE alone treats `_` and `%` as
    // wildcards). Multiple matches stay ambiguous through the caller.
    if all.is_empty() && name.contains('.') {
        let pattern = format!("%.{name}");
        let mut stmt = conn.prepare(&format!(
            "{CANDIDATE_SELECT}
             WHERE s.qualname LIKE ?1
             ORDER BY s.qualname, f.path",
        ))?;
        let rows = stmt.query_map([&pattern], candidate_row)?;
        let suffix = format!(".{name}");
        all = rows
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|r| r.1.ends_with(&suffix))
            .collect();
    }
    // A module shares the bare name of classes/functions (`button` vs
    // `Button`); when a callable candidate exists, module symbols must not
    // create a false ambiguity (R7 F1). Unknown kinds stay non-module, as
    // before the enum.
    let has_callable = all
        .iter()
        .any(|r| SymbolKind::parse(&r.4) != Some(SymbolKind::Module));
    let mut ids = Vec::new();
    let mut cands = Vec::new();
    let mut seen_ids: std::collections::HashSet<i64> = Default::default();
    for (id, qualname, path, line, kind) in all {
        if has_callable && SymbolKind::parse(&kind) == Some(SymbolKind::Module) {
            continue;
        }
        // Dedupe by id: a symbol whose bare name equals its qualname matches
        // both WHERE branches and must not create a false ambiguity. O(1)
        // membership (R6f: a Vec::contains scan was quadratic).
        if seen_ids.insert(id) {
            ids.push(id);
            // Render cap only (R6f): the error carries `total` separately and
            // the caller prints at most MAX_AMBIGUOUS_CANDIDATES lines.
            if cands.len() < MAX_AMBIGUOUS_CANDIDATES {
                cands.push((path, line, qualname));
            }
        }
    }
    Ok((ids, cands))
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
    Ok((
        render_output(&out).lines().map(str::to_string).collect(),
        stderr,
    ))
}

/// Resolve a repo-relative file path and fail with the shared not-indexed
/// error. `imports` and `impact` both start with this gate.
fn indexed_file(conn: &Connection, file: &str) -> Result<String> {
    indexed_file_opt(conn, normalize_scope(file))?.ok_or_else(|| AwError::not_indexed(file))
}

/// The same files-row lookup without the not-indexed error, so `symbols` can
/// reuse the gate and fall through to its directory listing.
fn indexed_file_opt(conn: &Connection, rel: &str) -> Result<Option<String>> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM files WHERE path = ?1", [rel], |r| {
        r.get(0)
    })?;
    Ok((count > 0).then(|| rel.to_string()))
}

/// Count result files changed since the build and render the standard header.
/// Every query surface uses the same wording and the same stat short-circuit.
fn staleness_header(ctx: &QueryCtx<'_>, files: &BTreeSet<String>) -> (Option<String>, bool) {
    let mut changed = 0usize;
    for f in files {
        if file_changed(ctx, f) {
            changed += 1;
        }
    }
    let stale = changed > 0;
    let header = stale.then(|| {
        format!(
            "aw: {changed} of {} result files changed since build — run aw build",
            files.len()
        )
    });
    (header, stale)
}

fn collect_output(
    ctx: &QueryCtx<'_>,
    rows: impl Iterator<Item = std::result::Result<Pointer, rusqlite::Error>>,
    limit: usize,
    empty_reason: &str,
    known_total: usize,
) -> Result<QueryOutput> {
    // Bounded materialization (R6f Important 5): callers pass a SQL LIMIT
    // plus a separate COUNT, so memory stays proportional to the displayed
    // set instead of the total hit count.
    let mut all: Vec<Pointer> = Vec::new();
    for r in rows {
        all.push(r?);
    }
    if known_total == 0 {
        return Err(AwError::NoResults(empty_reason.to_string()));
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

/// Staleness: (mtime, size) stat short-circuit; FNV-1a re-hash on mismatch.
pub fn file_changed(ctx: &QueryCtx<'_>, rel: &str) -> bool {
    let QueryCtx { conn, root } = *ctx;
    let row = conn.query_row(
        "SELECT mtime, size, content_hash FROM files WHERE path = ?1",
        [rel],
        crate::db::ints3,
    );
    let Ok((stored_mtime, stored_size, stored_hash)) = row else {
        return false;
    };
    let abs = root.join(rel);
    let Some(size) = db::stat_size(&abs) else {
        return true;
    };
    if size != stored_size {
        return true;
    }
    let Some(mtime) = db::stat_mtime(&abs) else {
        return true;
    };
    if mtime != stored_mtime {
        // Same guard as the walk (regular file + hard 1 MiB cap): a FIFO,
        // socket, or over-cap replacement must not block or blow up the
        // re-hash. Any such oddity is conservatively treated as changed.
        let Ok(bytes) = crate::walk::read_bounded(&abs) else {
            return true;
        };
        let hash = fnv1a64(&bytes) as i64;
        return hash != stored_hash;
    }
    false
}
