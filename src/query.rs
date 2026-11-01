//! Query surface: callers/callees/impact with qualname disambiguation,
//! result-set-scoped staleness, caps, tier tags, and empty-result reason lines
//! (response contract; CUJ2/CUJ3/CUJ4/CUJ7).

use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::Connection;

use crate::db;
use crate::error::{KgError, Result};
use crate::hash::fnv1a64;

pub const DEFAULT_LIMIT: usize = 50;
pub const MAX_LIMIT: usize = 10_000;

/// Permanent honesty caveat for `kg dead` (Amendment 5): candidates, not proof.
const DEAD_CAVEAT: &str =
    "kg: dead-code candidates — dynamic dispatch, entry points, and external callers are not proven absent";

/// Permanent honesty caveat for `kg clones` (Amendment 12d): candidates, not proof.
const CLONES_CAVEAT: &str =
    "kg: clone candidates — normalized-text equality, not proof of semantic duplication";

/// One resolved candidate: (path, line, qualname).
pub type Candidate = (String, i64, String);

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
            format!("{}:{}{} {}", self.file, self.line, self.tag.trim_start(), self.detail)
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
        return Err(KgError::Usage("--limit must be >= 1".to_string()));
    }
    Ok(limit.min(MAX_LIMIT))
}

/// Resolve the query name to target symbol ids. Multiple distinct symbols ->
/// exit 2 with a `path:line qualname` candidate list on stdout (CUJ4).
fn target_ids(conn: &Connection, name: &str) -> Result<(Vec<i64>, Vec<Candidate>)> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.qualname, s.name, f.path, s.line_start
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE s.name = ?1 OR s.qualname = ?1
         ORDER BY s.qualname, f.path",
    )?;
    let rows = stmt.query_map(rusqlite::params![name], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?))
    })?;
    let mut ids = Vec::new();
    let mut cands = Vec::new();
    for row in rows {
        let (id, qualname, sname, path, line) = row?;
        // Dedupe by id: a symbol whose bare name equals its qualname matches
        // both WHERE branches and must not create a false ambiguity.
        if !ids.contains(&id) {
            ids.push(id);
            cands.push((path, line, qualname));
        }
        let _ = sname;
    }
    Ok((ids, cands))
}

pub fn callers(conn: &Connection, root: &Path, name: &str, limit: usize) -> Result<QueryOutput> {
    let limit = clamp_limit(limit)?;
    let (ids, cands) = target_ids(conn, name)?;
    if ids.is_empty() {
        return Err(KgError::NoResults(format!("symbol '{name}' not in index")));
    }
    if cands.len() > 1 {
        // CUJ4: no merged-silent result; name the Class.method form. The
        // candidate list travels IN the error so non-CLI surfaces render it.
        return Err(KgError::Ambiguous {
            name: name.to_string(),
            candidates: cands,
        });
    }
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT e.site_line, e.confidence, f.path
         FROM edges e
         JOIN symbols s ON s.id = e.src_id
         JOIN files f ON f.id = s.file_id
         WHERE e.dst_id IN ({placeholders}) AND e.kind = 'calls'
         ORDER BY f.path, e.site_line"
    );
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<rusqlite::types::Value> = ids.iter().map(|i| rusqlite::types::Value::from(*i)).collect();
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        let site: i64 = r.get(0)?;
        let conf: String = r.get(1)?;
        let path: String = r.get(2)?;
        Ok(Pointer {
            file: path,
            line: site,
            tag: if conf == "AMBIGUOUS" { " [ambiguous]" } else { "" },
            detail: String::new(),
        })
    })?;
    collect_output(
        conn,
        root,
        name,
        rows,
        limit,
        "0 callers found (dynamic dispatch possible — grep fallback recommended)",
    )
}

pub fn callees(
    conn: &Connection,
    root: &Path,
    name: &str,
    limit: usize,
    depth: usize,
) -> Result<QueryOutput> {
    let limit = clamp_limit(limit)?;
    let depth = depth.clamp(1, 10);
    let (ids, cands) = target_ids(conn, name)?;
    if ids.is_empty() {
        return Err(KgError::NoResults(format!("symbol '{name}' not in index")));
    }
    if cands.len() > 1 {
        return Err(KgError::Ambiguous {
            name: name.to_string(),
            candidates: cands,
        });
    }
    // Callee pointer: the callee's definition line. AMBIGUOUS edges have no
    // dst symbol; they surface as the call site itself with the ambiguous tag.
    // depth > 1: recursive closure over call edges from the reached set.
    let (sql, params): (String, Vec<rusqlite::types::Value>) = if depth == 1 {
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        (
            format!(
                "SELECT COALESCE(ds.line_start, e.site_line) AS line, e.confidence,
                COALESCE(df.path, sf.path) AS path
         FROM edges e
         JOIN symbols s ON s.id = e.src_id JOIN files sf ON sf.id = s.file_id
         LEFT JOIN symbols ds ON ds.id = e.dst_id
         LEFT JOIN files df ON df.id = ds.file_id
         WHERE e.src_id IN ({placeholders}) AND e.kind = 'calls'
         ORDER BY path, line"
            ),
            ids.iter().map(|i| rusqlite::types::Value::from(*i)).collect(),
        )
    } else {
        let seed = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        (
            format!(
                "WITH RECURSIVE reach(sym, d) AS (
             SELECT id, 0 FROM symbols WHERE id IN ({seed})
             UNION
             SELECT e.dst_id, r.d + 1 FROM edges e JOIN reach r ON e.src_id = r.sym
             WHERE e.kind = 'calls' AND e.dst_id IS NOT NULL AND r.d < ?{d}
         )
         SELECT DISTINCT COALESCE(ds.line_start, e.site_line) AS line, e.confidence,
                COALESCE(df.path, sf.path) AS path
         FROM reach r
         JOIN edges e ON e.src_id = r.sym AND e.kind = 'calls'
         JOIN symbols ss ON ss.id = e.src_id JOIN files sf ON sf.id = ss.file_id
         LEFT JOIN symbols ds ON ds.id = e.dst_id
         LEFT JOIN files df ON df.id = ds.file_id
         ORDER BY path, line",
                d = 1 + ids.len()
            ),
            ids.iter()
                .map(|i| rusqlite::types::Value::from(*i))
                .chain(std::iter::once(rusqlite::types::Value::from(
                    depth as i64,
                )))
                .collect(),
        )
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        let line: i64 = r.get(0)?;
        let conf: String = r.get(1)?;
        let path: String = r.get(2)?;
        Ok(Pointer {
            file: path,
            line,
            tag: if conf == "AMBIGUOUS" { " [ambiguous]" } else { "" },
            detail: String::new(),
        })
    })?;
    collect_output(conn, root, name, rows, limit, "0 callees found")
}

pub fn impact(conn: &Connection, root: &Path, file: &str, limit: usize) -> Result<QueryOutput> {
    let limit = clamp_limit(limit)?;
    let rel = file.trim_start_matches("./").to_string();
    let exists: bool = conn
        .query_row("SELECT COUNT(*) FROM files WHERE path = ?1", [&rel], |r| r.get(0))
        .map(|c: i64| c > 0)?;
    if !exists {
        return Err(KgError::NoResults(format!("path '{file}' not indexed")));
    }
    // Direction semantics (CLI help): files that call INTO <file>.
    let sql = "SELECT e.site_line, e.confidence, sf.path
         FROM edges e
         JOIN symbols ds ON ds.id = e.dst_id JOIN files df ON df.id = ds.file_id
         JOIN symbols ss ON ss.id = e.src_id JOIN files sf ON sf.id = ss.file_id
         WHERE df.path = ?1 AND e.kind = 'calls'
         ORDER BY sf.path, e.site_line";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([&rel], |r| {
        let site: i64 = r.get(0)?;
        let conf: String = r.get(1)?;
        let path: String = r.get(2)?;
        Ok(Pointer {
            file: path,
            line: site,
            tag: if conf == "AMBIGUOUS" { " [ambiguous]" } else { "" },
            detail: String::new(),
        })
    })?;
    collect_output(conn, root, &rel, rows, limit, "0 impacting files found")
}

/// Dead-code candidates (Amendment 5): no resolved inbound calls edge and no
/// AMBIGUOUS edge naming the symbol (qualname or bare name). Pointer contract;
/// the caveat header is unconditional.
pub fn dead(conn: &Connection, root: &Path, limit: usize) -> Result<QueryOutput> {
    let limit = clamp_limit(limit)?;
    let sql = "SELECT s.qualname, f.path, s.line_start
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE s.kind IN ('function', 'method', 'class')
           AND s.id NOT IN (
               SELECT dst_id FROM edges WHERE kind = 'calls' AND dst_id IS NOT NULL)
           AND s.qualname NOT IN (
               SELECT dst_name FROM edges WHERE kind = 'calls' AND confidence = 'AMBIGUOUS')
           AND s.name NOT IN (
               SELECT dst_name FROM edges WHERE kind = 'calls' AND confidence = 'AMBIGUOUS')
         ORDER BY f.path, s.line_start";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |r| {
        let path: String = r.get(1)?;
        let line: i64 = r.get(2)?;
        Ok(Pointer { file: path, line, tag: "", detail: String::new() })
    })?;
    let mut out = collect_output(conn, root, "", rows, limit, "0 dead-code candidates found")?;
    out.header = Some(match out.header.take() {
        Some(stale) => format!("{DEAD_CAVEAT}\n{stale}"),
        None => DEAD_CAVEAT.to_string(),
    });
    Ok(out)
}

/// Unresolved-callee report (Amendment 5): files with counts, descending.
/// Report shape — cap, no staleness; header carries the index total.
pub fn unresolved(conn: &Connection, limit: usize) -> Result<Vec<String>> {
    let limit = clamp_limit(limit)?;
    let total: i64 = conn.query_row("SELECT COALESCE(SUM(unresolved), 0) FROM files", [], |r| r.get(0))?;
    if total == 0 {
        return Err(KgError::NoResults(
            "0 unresolved callees (full call-site coverage)".to_string(),
        ));
    }
    let mut stmt = conn.prepare(
        "SELECT path, unresolved FROM files WHERE unresolved > 0
         ORDER BY unresolved DESC, path",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let n_files = rows.len();
    let shown = n_files.min(limit);
    let mut out = vec![format!("kg: unresolved={total} callees in {n_files} files")];
    out.extend(
        rows.into_iter()
            .take(shown)
            .map(|(path, n)| format!("{path} unresolved={n}")),
    );
    if n_files > shown {
        out.push(format!("showing {} of {}", shown, n_files));
        eprintln!("[kg] capped at {} results ({} total)", shown, n_files);
    }
    crate::journal::record_result(n_files, shown, n_files, false, 0);
    Ok(out)
}

/// Symbols of one indexed file (Amendment 5): `path:line kind qualname` by
/// line; single-file staleness header. With `outline` (Amendment 12a) each
/// line gains the stored one-line signature; without it, byte-identical to
/// the pre-v4 contract.
pub fn symbols(
    conn: &Connection,
    root: &Path,
    file: &str,
    outline: bool,
    limit: usize,
) -> Result<Vec<String>> {
    let limit = clamp_limit(limit)?;
    let rel = file.trim_start_matches("./").to_string();
    let sql = if outline {
        "SELECT s.kind, s.qualname, s.line_start, s.signature
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE f.path = ?1
         ORDER BY s.line_start"
    } else {
        "SELECT s.kind, s.qualname, s.line_start, ''
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE f.path = ?1
         ORDER BY s.line_start"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map([&rel], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if rows.is_empty() {
        return Err(KgError::NoResults(format!("path '{file}' not indexed")));
    }
    let stale = file_changed(conn, root, &rel);
    let n_symbols = rows.len();
    let shown = n_symbols.min(limit);
    let data_lines: Vec<String> = rows
        .into_iter()
        .take(shown)
        .map(|(kind, qualname, line, signature)| {
            if signature.is_empty() {
                format!("{rel}:{line} {kind} {qualname}")
            } else {
                format!("{rel}:{line} {kind} {qualname} {signature}")
            }
        })
        .collect();
    // Payload capture (Amendment 10a): the `path:line kind qualname` data
    // lines only — never the staleness header or the cap footer.
    let built_at = db::meta_get(conn, "built_at")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let source_rev = db::meta_get(conn, "source_rev").unwrap_or_default();
    crate::journal::record_payload(
        &rel,
        &data_lines,
        stale,
        shown,
        n_symbols,
        built_at,
        &source_rev,
    );
    crate::journal::record_result(n_symbols, shown, n_symbols, stale, 0);
    let mut out = Vec::new();
    if stale {
        out.push("kg: 1 of 1 result files changed since build — run kg build".to_string());
    }
    out.extend(data_lines);
    if n_symbols > shown {
        out.push(format!("showing {} of {}", shown, n_symbols));
        eprintln!("[kg] capped at {} results ({} total)", shown, n_symbols);
    }
    Ok(out)
}

/// Symbol-level search (Amendment 12b): FTS5 `bm25()` over split identifier
/// tokens. Hits are symbol identities — the tier field position stays empty
/// (only call-edge pointers carry ` [ambiguous]`). Reuses `collect_output`
/// for cap, post-cap staleness, and payload capture.
pub fn search(conn: &Connection, root: &Path, terms: &str, limit: usize) -> Result<Vec<String>> {
    let limit = clamp_limit(limit)?;
    // OR of prefix terms, bm25-ranked (Amendment 12b): rows matching more
    // and rarer tokens rank higher — natural-language fit without brittle
    // all-token AND requirements.
    let expr = crate::extract::split_ident(terms)
        .split_whitespace()
        .map(|tok| format!("\"{}\"*", tok.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ");
    if expr.is_empty() {
        return Err(KgError::Usage("empty search terms".to_string()));
    }
    let mut stmt = conn.prepare(
        "SELECT f.path, s.line_start, s.qualname
         FROM symbols_fts ft
         JOIN symbols s ON s.id = ft.rowid
         JOIN files f ON f.id = s.file_id
         WHERE symbols_fts MATCH ?1
         ORDER BY bm25(symbols_fts)",
    )?;
    let rows = stmt.query_map([&expr], |r| {
        Ok(Pointer {
            file: r.get::<_, String>(0)?,
            line: r.get::<_, i64>(1)?,
            tag: "",
            detail: r.get::<_, String>(2)?,
        })
    })?;
    let out = collect_output(
        conn,
        root,
        terms,
        rows,
        limit,
        &format!("0 symbols matched '{terms}' — try fewer or broader terms"),
    )?;
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
        eprintln!("[kg] capped at {} results ({} total)", out.shown, out.total);
    }
    Ok(lines)
}

/// Clone candidates (Amendment 12d): merge adjacent matching shingle windows
/// per file, pair the merged regions, filter by `min_lines`, emit
/// `pathA:startA-endA pathB:startB-endB` under the standard cap contract.
/// Caveat line always first — candidates, never proof. NOT payload-journaled
/// (Amendment 12 deviation: pair lines are not single pointers, so the
/// `usage --verify` structural checks do not apply).
pub fn clones(conn: &Connection, root: &Path, min_lines: usize, limit: usize) -> Result<Vec<String>> {
    let limit = clamp_limit(limit)?;
    let min_lines = min_lines.max(crate::clones::WINDOW);
    let mut stmt = conn.prepare(
        "SELECT cg.shingle_hash, f.path, cg.line_start, cg.line_end
         FROM clone_groups cg JOIN files f ON f.id = cg.file_id
         ORDER BY cg.shingle_hash, f.path, cg.line_start",
    )?;
    let mut groups: std::collections::BTreeMap<i64, Vec<(String, i64, i64)>> = Default::default();
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    for r in rows {
        let (h, path, s, e) = r?;
        groups.entry(h).or_default().push((path, s, e));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut pairs: Vec<(String, i64, i64, String, i64, i64)> = Vec::new();
    for regions in groups.values() {
        // Merge overlapping / near-adjacent windows (gap < window) per file:
        // one long clone produces many overlapping windows, one region.
        let mut merged: Vec<(String, i64, i64)> = Vec::new();
        for (path, s, e) in regions {
            if let Some(last) = merged.last_mut() {
                if last.0 == *path && *s <= last.2 + 4 {
                    last.2 = (*e).max(last.2);
                    continue;
                }
            }
            merged.push((path.clone(), *s, *e));
        }
        if merged.len() < 2 {
            continue;
        }
        for i in 0..merged.len() {
            for j in (i + 1)..merged.len() {
                let (pa, sa, ea) = &merged[i];
                let (pb, sb, eb) = &merged[j];
                if (*ea - *sa + 1) < min_lines as i64 || (*eb - *sb + 1) < min_lines as i64 {
                    continue;
                }
                let (fa, sta, ena, fb, stb, enb) = if (pa.as_str(), *sa, *ea)
                    <= (pb.as_str(), *sb, *eb)
                {
                    (pa, *sa, *ea, pb, *sb, *eb)
                } else {
                    (pb, *sb, *eb, pa, *sa, *ea)
                };
                let key = (fa.clone(), sta, ena, fb.clone(), stb, enb);
                if seen.insert(key) {
                    pairs.push((fa.clone(), sta, ena, fb.clone(), stb, enb));
                }
            }
        }
    }
    // Post-merge: windows of one long clone live in different hash groups,
    // so the same file pair re-emits per window. Merge same-file-pair rows
    // whose spans overlap on BOTH sides into one region pair.
    pairs.sort();
    let mut merged_pairs: Vec<(String, i64, i64, String, i64, i64)> = Vec::new();
    for (fa, sa, ea, fb, sb, eb) in pairs {
        if let Some(last) = merged_pairs.last_mut() {
            if last.0 == fa && last.3 == fb && sa <= last.2 && sb <= last.5 {
                last.2 = last.2.max(ea);
                last.5 = last.5.max(eb);
                continue;
            }
        }
        merged_pairs.push((fa, sa, ea, fb, sb, eb));
    }
    let pairs = merged_pairs;
    if pairs.is_empty() {
        return Err(KgError::NoResults("0 clone candidates found".to_string()));
    }
    let shown = pairs.len().min(limit);
    let displayed = &pairs[..shown];
    let files: std::collections::BTreeSet<&String> = displayed
        .iter()
        .flat_map(|(a, _, _, b, _, _)| [a, b])
        .collect();
    let n_files = files.len();
    let mut changed = 0usize;
    for f in &files {
        if file_changed(conn, root, f) {
            changed += 1;
        }
    }
    let stale = changed > 0;
    crate::journal::record_result(shown, shown, pairs.len(), stale, 0);
    let mut out = vec![CLONES_CAVEAT.to_string()];
    if stale {
        out.push(format!(
            "kg: {changed} of {n_files} result files changed since build — run kg build"
        ));
    }
    for (pa, sa, ea, pb, sb, eb) in displayed {
        out.push(format!("{pa}:{sa}-{ea} {pb}:{sb}-{eb}"));
    }
    if pairs.len() > shown {
        out.push(format!("showing {} of {}", shown, pairs.len()));
        eprintln!("[kg] capped at {} results ({} total)", shown, pairs.len());
    }
    Ok(out)
}

/// Cap, then staleness-check the post-cap displayed set only (cost bounded by
/// the displayed count): stat short-circuits, FNV-1a re-hash on mismatch.
/// `name` is the query target recorded in the payload journal (Amendment 10).
fn collect_output(
    conn: &Connection,
    root: &Path,
    name: &str,
    rows: impl Iterator<Item = std::result::Result<Pointer, rusqlite::Error>>,
    limit: usize,
    empty_reason: &str,
) -> Result<QueryOutput> {
    let mut all: Vec<Pointer> = Vec::new();
    for r in rows {
        all.push(r?);
    }
    let total = all.len();
    if total == 0 {
        return Err(KgError::NoResults(empty_reason.to_string()));
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
            "kg: {changed} of {n_result_files} result files changed since build — run kg build"
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
    record_payload_capture(conn, name, &out);
    Ok(out)
}

/// Payload journal capture (Amendment 10a): the rendered displayed set,
/// byte-identical to stdout data lines, plus the index meta it came from.
/// Best-effort like every journal write; runs before the dispatch site's
/// `record_result`, which fills the same Pending fields consistently.
fn record_payload_capture(conn: &Connection, name: &str, out: &QueryOutput) {
    if out.pointers.is_empty() {
        return;
    }
    let built_at = db::meta_get(conn, "built_at")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let source_rev = db::meta_get(conn, "source_rev").unwrap_or_default();
    let rendered: Vec<String> = out.pointers.iter().map(|p| p.render()).collect();
    crate::journal::record_payload(
        name,
        &rendered,
        out.stale,
        out.shown,
        out.total,
        built_at,
        &source_rev,
    );
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
        eprintln!("[kg] capped at {} results ({} total)", out.shown, out.total);
    }
}

/// Staleness: (mtime, size) stat short-circuit; FNV-1a re-hash on mismatch.
pub fn file_changed(conn: &Connection, root: &Path, rel: &str) -> bool {
    let row = conn.query_row(
        "SELECT mtime, size, content_hash FROM files WHERE path = ?1",
        [rel],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)),
    );
    let Ok((stored_mtime, stored_size, stored_hash)) = row else { return false };
    let abs = root.join(rel);
    let Some(size) = db::stat_size(&abs) else { return true };
    if size != stored_size {
        return true;
    }
    let Some(mtime) = db::stat_mtime(&abs) else { return true };
    if mtime != stored_mtime {
        let Ok(bytes) = std::fs::read(&abs) else { return true };
        let hash = fnv1a64(&bytes) as i64;
        return hash != stored_hash;
    }
    false
}
