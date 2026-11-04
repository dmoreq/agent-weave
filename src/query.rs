//! Query surface: callers/callees/impact with qualname disambiguation,
//! result-set-scoped staleness, caps, tier tags, and empty-result reason lines
//! (response contract; CUJ2/CUJ3/CUJ4/CUJ7).

use std::collections::{BTreeSet, BinaryHeap, HashSet};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use crate::db;
use crate::error::{AwError, Result};
use crate::hash::fnv1a64;

pub const DEFAULT_LIMIT: usize = 50;
/// Cap on rendered ambiguous-name candidates (R6f: common method names could
/// print megabytes; the total count still travels in the error).
pub const MAX_AMBIGUOUS_CANDIDATES: usize = 50;
pub const MAX_LIMIT: usize = 10_000;

/// Permanent honesty caveat for `aw dead` (Amendment 5): candidates, not proof.
const DEAD_CAVEAT: &str =
    "aw: dead-code candidates — dynamic dispatch, entry points, star imports, module-level imports, type references, attribute references, and external callers are not proven absent";

/// Permanent honesty caveat for `aw clones` (Amendment 12d): candidates, not proof.
const CLONES_CAVEAT: &str =
    "aw: clone candidates — normalized-text equality, not proof of semantic duplication";

/// Permanent honesty caveat for `aw path` (Amendment 13): resolved static
/// edges only; dynamic dispatch may hide paths.
const PATH_CAVEAT: &str = "aw: path — resolved static edges only; dynamic dispatch may hide paths";

/// Permanent honesty caveat for `aw context` (Amendment 17).
const CONTEXT_CAVEAT: &str =
    "aw: context — resolved static edges plus tagged [ambiguous] candidates; dynamic dispatch may hide callers or callees";

/// Permanent honesty caveat for `aw implementers` (Amendment 19).
const IMPLEMENTERS_CAVEAT: &str = "aw: implementers — resolved static inherits edges only; dynamic inheritance and cross-file impls are omitted";

/// `impact` caveats (Amendment 19): the default calls-only shape stays
/// byte-identical; the extended shapes state their static-edge limit.
const IMPACT_EDGES_CAVEAT: &str =
    "aw: impact — resolved static edges only; unresolved imports/inherits are omitted";
const IMPACT_CALLS_CAVEAT: &str =
    "aw: impact — resolved static call edges only; dynamic dispatch may hide dependents";

/// One import statement is one site even when it resolves to both a module and
/// a symbol edge; every non-import edge remains an individual site.
const IMPACT_SITE_KEY: &str =
    "CASE WHEN e.kind = 'imports' THEN 'i:' || ss.id || ':' || e.site_line ELSE 'e:' || e.id END";

/// `aw imports` caveat (Amendment 19): outbound resolved imports only.
const IMPORTS_CAVEAT: &str =
    "aw: imports — resolved static imports only; external and unresolved imports are omitted";

/// `aw tests` caveat (Amendment 19): static candidates only.
const TESTS_CAVEAT: &str = "aw: test candidates — static resolved call edges only; test classification covers test paths, test-shaped names, Rust #[test]/#[cfg(test)], and TS describe/it scope, but fixtures, generated tests, and indirect tests may still be missed";
/// Forward search from every test is cheaper than a reverse graph walk when
/// the test set is bounded; larger sets keep the reverse BFS.
const FORWARD_TEST_LIMIT: usize = 20000;

/// One resolved candidate: (path, line, qualname).
mod scope;

use scope::*;
pub use scope::{path_scope_sql_pub, validate_path_scope};

mod analysis;
mod callgraph;
mod impact;

pub use analysis::{
    clones, complexity, dead, files, resolve_names, search, symbols, unresolved, unresolved_names,
};
pub use callgraph::{callees, callers, context, implementers, imports, path, test_impact};
pub use impact::impact;

fn total_callables(conn: &Connection) -> Result<usize> {
    if let Some(value) = db::meta_get(conn, "callable_symbol_count") {
        if let Ok(n) = value.parse::<usize>() {
            return Ok(n);
        }
    }
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM symbols WHERE kind IN ('function', 'method')",
        [],
        |r| r.get::<_, i64>(0),
    )? as usize)
}

fn has_test_candidates(conn: &Connection) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM symbols
         WHERE kind IN ('function', 'method')
           AND is_test = 1
         LIMIT 1",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn total_test_symbols(conn: &Connection) -> Result<usize> {
    if let Some(value) = db::meta_get(conn, "test_symbol_count") {
        if let Ok(n) = value.parse::<usize>() {
            return Ok(n);
        }
    }
    Ok(
        conn.query_row("SELECT COUNT(*) FROM symbols WHERE is_test = 1", [], |r| {
            r.get::<_, i64>(0)
        })? as usize,
    )
}

fn is_callable_kind(kind: &str) -> bool {
    matches!(kind, "function" | "method")
}

/// Dead-code candidates (Amendment 5): no resolved inbound calls edge and no
/// AMBIGUOUS edge naming the symbol (qualname or bare name). Pointer contract;
/// the caveat header is unconditional.
pub type Candidate = (String, i64, String);

/// One symbol row with metrics: (kind, qualname, line, signature,
/// cyclomatic, branches, nesting, effective_loc). Amendment 19.
type SymbolRow = (String, String, i64, String, i64, i64, i64, i64);

/// One BFS level's out-edges grouped per source: `(dst qualname, dst path,
/// site_line, dst id)` sorted by the pinned neighbor key (`path` is the
/// destination file). Amendment 13.
type LevelEdges = std::collections::HashMap<i64, Vec<(String, String, i64, i64)>>;

pub struct Pointer {
    pub file: String,
    pub line: i64,
    pub tag: &'static str, // "" or " [ambiguous]"
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

pub fn clamp_limit(limit: usize) -> Result<usize> {
    if limit == 0 {
        return Err(AwError::Usage("--limit must be >= 1".to_string()));
    }
    Ok(limit.min(MAX_LIMIT))
}

/// Resolve the query name to target symbol ids. Multiple distinct symbols ->
/// exit 2 with a `path:line qualname` candidate list on stdout (CUJ4).
fn target_ids(conn: &Connection, name: &str) -> Result<(Vec<i64>, Vec<Candidate>)> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.qualname, s.name, f.path, s.line_start, s.kind
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE s.name = ?1 OR s.qualname = ?1
         ORDER BY s.qualname, f.path",
    )?;
    let rows = stmt.query_map(rusqlite::params![name], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, String>(5)?,
        ))
    })?;
    let mut all: Vec<_> = rows.collect::<std::result::Result<_, _>>()?;
    // `Class.method` suffix form: the full qualname is `pkg.Class.method`, so
    // an exact name/qualname match misses it. Fall back to a suffix scan and
    // confirm the exact suffix in Rust (LIKE alone treats `_` and `%` as
    // wildcards). Multiple matches stay ambiguous through the caller.
    if all.is_empty() && name.contains('.') {
        let pattern = format!("%.{name}");
        let mut stmt = conn.prepare(
            "SELECT s.id, s.qualname, s.name, f.path, s.line_start, s.kind
             FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE s.qualname LIKE ?1
             ORDER BY s.qualname, f.path",
        )?;
        let rows = stmt.query_map([&pattern], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let suffix = format!(".{name}");
        all = rows
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|r| r.1.ends_with(&suffix))
            .collect();
    }
    // A module shares the bare name of classes/functions (`button` vs
    // `Button`); when a callable candidate exists, module symbols must not
    // create a false ambiguity (R7 F1).
    let has_callable = all.iter().any(|r| r.5 != "module");
    let mut ids = Vec::new();
    let mut cands = Vec::new();
    let mut seen_ids: std::collections::HashSet<i64> = Default::default();
    for row in all {
        let (id, qualname, sname, path, line, kind) = row;
        if has_callable && kind == "module" {
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
        let _ = sname;
    }
    Ok((ids, cands))
}

/// Cap, then staleness-check the post-cap displayed set only (cost bounded by
/// the displayed count): stat short-circuits, FNV-1a re-hash on mismatch.
/// Render a search QueryOutput (header, hits, cap footer) as stdout lines.
fn search_lines(out: QueryOutput) -> Result<Vec<String>> {
    // Every search exit path (exact, phrase, fuzzy) journals here, so an
    // exact-name hit is not recorded as an empty call (R7 review).
    crate::journal::record_result(out.pointers.len(), out.shown, out.total, out.stale, 0);
    let mut lines = Vec::with_capacity(out.pointers.len() + 2);
    if let Some(h) = &out.header {
        lines.push(h.clone());
    }
    for p in &out.pointers {
        lines.push(p.render());
    }
    if out.total > out.shown {
        lines.push(format!("showing {} of {}", out.shown, out.total));
        eprintln!("[aw] capped at {} results ({} total)", out.shown, out.total);
    }
    Ok(lines)
}

/// Bounded English morphology variants for a zero-hit token (R7).
fn derived_forms(tok: &str, out: &mut Vec<String>) {
    if tok.len() <= 4 {
        return;
    }
    if let Some(stem) = tok.strip_suffix("ies") {
        out.push(format!("{stem}y"));
        return;
    }
    for suffix in ["es", "s", "ed", "ing", "ion"] {
        if let Some(stem) = tok.strip_suffix(suffix) {
            if stem.len() >= 3 {
                out.push(stem.to_string());
                out.push(format!("{stem}e"));
            }
            return;
        }
    }
}

fn collect_output(
    conn: &Connection,
    root: &Path,
    rows: impl Iterator<Item = std::result::Result<Pointer, rusqlite::Error>>,
    limit: usize,
    empty_reason: &str,
    known_total: Option<usize>,
) -> Result<QueryOutput> {
    // Bounded materialization (R6f Important 5): callers pass a SQL LIMIT
    // plus a separate COUNT, so memory stays proportional to the displayed
    // set instead of the total hit count.
    let mut all: Vec<Pointer> = Vec::new();
    for r in rows {
        all.push(r?);
    }
    let total = known_total.unwrap_or(all.len());
    if total == 0 {
        return Err(AwError::NoResults(empty_reason.to_string()));
    }
    let shown = all.len().min(limit);
    let displayed: Vec<Pointer> = all.into_iter().take(shown).collect();
    let files: BTreeSet<&String> = displayed.iter().map(|p| &p.file).collect();
    let n_result_files = files.len();
    let mut changed = 0usize;
    for f in &files {
        if file_changed(conn, root, f) {
            changed += 1;
        }
    }
    let header = if changed > 0 {
        Some(format!(
            "aw: {changed} of {n_result_files} result files changed since build — run aw build"
        ))
    } else {
        None
    };
    let out = QueryOutput {
        header,
        pointers: displayed,
        shown,
        total,
        stale: changed > 0,
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
    if out.total > out.shown {
        text.push_str(&format!("showing {} of {}\n", out.shown, out.total));
    }
    text
}

pub fn print_output(out: &QueryOutput) {
    print!("{}", render_output(out));
    if out.total > out.shown {
        eprintln!("[aw] capped at {} results ({} total)", out.shown, out.total);
    }
}

/// Staleness: (mtime, size) stat short-circuit; FNV-1a re-hash on mismatch.
pub fn file_changed(conn: &Connection, root: &Path, rel: &str) -> bool {
    let row = conn.query_row(
        "SELECT mtime, size, content_hash FROM files WHERE path = ?1",
        [rel],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        },
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
