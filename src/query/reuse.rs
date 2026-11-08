//! Anti-reinvention detection engine (D1-D4).
//! Referencing RepoReuse (arXiv:2609.35357) and codebase-minimalism.
//! Sub-20ms calibrated static duplicate detectors.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::error::Result;
use crate::extract::{split_ident, EdgeSiteKind};
use crate::query::{clamp_limit, QueryCtx};

/// Standard prelude/utility constructors and methods excluded from callee footprint.
const STOPLIST: &[&str] = &[
    // Rust prelude & stdlib
    "Ok",
    "Err",
    "Some",
    "None",
    "Vec::new",
    "String::new",
    "Default::default",
    "unwrap",
    "clone",
    "to_string",
    "format",
    "push",
    "len",
    "is_empty",
    "as_str",
    "as_ref",
    "into",
    "from",
    "map",
    "unwrap_or",
    "unwrap_or_default",
    "and_then",
    "contains",
    "get",
    "insert",
    "open",
    "new",
    "collect",
    "iter",
    "into_iter",
    "eprintln",
    "println",
    // TypeScript / JavaScript
    "Promise.all",
    "Promise.resolve",
    "console",
    "log",
    "error",
    "warn",
    "string",
    "number",
    "boolean",
    "any",
    "void",
    "Promise",
    "Array",
    "Object",
    // Python
    "def",
    "self",
    "cls",
    "int",
    "str",
    "float",
    "bool",
    "list",
    "dict",
    "set",
    "tuple",
    "print",
    "range",
    "isinstance",
];

/// Raw external clients targeted by D3 abstraction bypass.
const RAW_CLIENTS: &[&str] = &[
    "Connection::open",
    "sqlite3_open",
    "requests.get",
    "requests.post",
    "axios",
    "axios.get",
    "axios.post",
    "fetch",
    "reqwest::get",
    "ureq::get",
];

#[derive(Debug, Clone)]
pub struct ReuseCandidate {
    pub detector: &'static str,
    pub symbol: String,
    pub file: String,
    pub line: i64,
    pub canonical: String,
    pub canonical_file: String,
    pub canonical_line: i64,
    pub canonical_callers: i64,
    pub overlap_pct: u64,
    pub shared_items: Vec<String>,
}

impl ReuseCandidate {
    pub fn hint_line(&self) -> String {
        // The caller tail every detector renders the same way.
        let tail = format!(
            "([callers={}] in {}:{})",
            self.canonical_callers, self.canonical_file, self.canonical_line
        );
        match self.detector {
            // D1/D2 share the overlap template and differ only in the noun.
            "D1" | "D2" => format!(
                "symbol '{}' shares {}% {} with existing '{}' {}",
                self.symbol,
                self.overlap_pct,
                if self.detector == "D1" {
                    "callee footprint"
                } else {
                    "interface token overlap"
                },
                self.canonical,
                tail
            ),
            "D3" => format!(
                "symbol '{}' bypasses canonical client '{}' to call raw external client {}",
                self.symbol, self.canonical, tail
            ),
            "D4" => format!(
                "symbol '{}' is a pass-through wrapper around existing '{}' {}",
                self.symbol, self.canonical, tail
            ),
            _ => format!(
                "symbol '{}' potential duplication of existing '{}' {}",
                self.symbol, self.canonical, tail
            ),
        }
    }

    pub fn format_text(&self) -> String {
        let mut line = self.hint_line();
        if !self.shared_items.is_empty() {
            let label = match self.detector {
                "D1" => "shared callees",
                "D2" => "shared tokens",
                "D3" => "client bypass",
                _ => "details",
            };
            line.push_str(&format!(
                "\n  {}: [{}]",
                label,
                self.shared_items.join(", ")
            ));
        }
        line
    }
}

struct SymbolData {
    id: i64,
    name: String,
    qualname: String,
    file_id: i64,
    file_path: String,
    line_start: i64,
    line_end: i64,
    resolved_callees: HashSet<String>,
    all_callees: HashSet<String>,
    callee_ids: HashSet<i64>,
    inbound_callers: i64,
    tokens: HashSet<String>,
    type_tokens: HashSet<String>,
}

/// Detect up to 3 non-failing advisory lines for modified symbols during incremental build.
pub fn detect_hints(conn: &Connection, new_names: &HashSet<String>) -> Result<Vec<String>> {
    if new_names.is_empty() {
        return Ok(Vec::new());
    }
    let (candidates, _) = analyze_corpus(conn, None, Some(new_names), 3)?;
    let mut hints = Vec::new();
    for c in candidates.iter().take(3) {
        hints.push(format!("aw: reuse-hint: {}", c.hint_line()));
    }
    Ok(hints)
}

/// Run `aw reuse` across the whole repository or specified paths.
pub fn run_reuse(
    ctx: &QueryCtx<'_>,
    paths: &[String],
    limit: usize,
) -> Result<crate::query::LineQuery> {
    let QueryCtx { conn, .. } = *ctx;
    let limit = clamp_limit(limit)?;
    let path_filter = if paths.is_empty() { None } else { Some(paths) };
    let (candidates, total) = analyze_corpus(conn, path_filter, None, limit)?;

    if candidates.is_empty() {
        crate::journal::record_result(crate::journal::OutcomeCounts::of(0, 0), false, 0);
        return Ok(crate::query::LineQuery {
            lines: vec!["0 reuse candidates found".to_string()],
            stderr: String::new(),
            shown: 0,
        });
    }

    let mut out = vec![format!(
        "aw: reuse — {} duplication candidate(s) found",
        candidates.len()
    )];
    for c in &candidates {
        for l in c.format_text().lines() {
            out.push(l.to_string());
        }
    }
    let shown = candidates.len();
    crate::journal::record_result(crate::journal::OutcomeCounts::of(shown, total), false, 0);
    Ok(crate::query::LineQuery {
        lines: out,
        stderr: String::new(),
        shown,
    })
}

pub(crate) fn analyze_corpus(
    conn: &Connection,
    path_filter: Option<&[String]>,
    target_names: Option<&HashSet<String>>,
    limit: usize,
) -> Result<(Vec<ReuseCandidate>, usize)> {
    let mut symbols = load_symbols(conn)?;
    if symbols.len() < 2 {
        return Ok((Vec::new(), 0));
    }

    load_callees(conn, &mut symbols)?;
    let stopset = build_stoplist(&symbols);
    for sym in symbols.values_mut() {
        sym.all_callees.retain(|c| !stopset.contains(c.as_str()));
        sym.resolved_callees
            .retain(|c| !stopset.contains(c.as_str()));
    }

    let targets = select_targets(&symbols, path_filter, target_names);
    let mut candidates = Vec::new();
    let mut seen_pairs = HashSet::new();

    for target in &targets {
        for detect in DETECTORS {
            if let Some(c) = detect(target, &symbols) {
                accept_candidate(c, &mut seen_pairs, &mut candidates);
                break;
            }
        }
    }

    candidates.sort_by(|a, b| {
        b.overlap_pct
            .cmp(&a.overlap_pct)
            .then_with(|| b.canonical_callers.cmp(&a.canonical_callers))
            .then_with(|| a.symbol.cmp(&b.symbol))
    });
    let total = candidates.len();
    candidates.truncate(limit);
    Ok((candidates, total))
}

/// One reuse detector: resolve a target against the corpus to a candidate.
type Detector = fn(&SymbolData, &HashMap<i64, SymbolData>) -> Option<ReuseCandidate>;

/// The detector cascade in priority order; a target stops at the first
/// detector that produces a candidate.
const DETECTORS: &[Detector] = &[find_best_d1, find_best_d2, check_d3, check_d4];

/// Dedup the (symbol, canonical) pair and push. A repeated pair is dropped,
/// but the target still stops at its first detector hit.
fn accept_candidate(
    candidate: ReuseCandidate,
    seen_pairs: &mut HashSet<(String, String)>,
    out: &mut Vec<ReuseCandidate>,
) {
    if seen_pairs.insert((candidate.symbol.clone(), candidate.canonical.clone())) {
        out.push(candidate);
    }
}

fn tokenize_name_sig(name: &str, sig: &str) -> (HashSet<String>, HashSet<String>) {
    let mut all_tokens = HashSet::new();
    let mut type_tokens = HashSet::new();
    // One normalized insert shared by every scan: lowercase, keep the
    // three-plus-char non-stoplist tokens, and also file them into the
    // type-only set when they come from a typed position.
    let mut push_token = |tok: &str, typed: bool| {
        let lower = tok.to_ascii_lowercase();
        if keep_token(&lower) {
            if typed {
                type_tokens.insert(lower.clone());
            }
            all_tokens.insert(lower);
        }
    };

    for tok in split_ident(name).split_whitespace() {
        push_token(tok, false);
    }

    for part in sig.split(['(', ')', ',', '{', ';']) {
        // Parameter (`:`) and return (`->`) annotations share one loop.
        for (_, typed) in [part.split_once(':'), part.split_once("->")]
            .into_iter()
            .flatten()
        {
            for tok in split_ident(typed).split_whitespace() {
                push_token(tok, true);
            }
        }
    }

    for tok in split_ident(sig).split_whitespace() {
        push_token(tok, false);
    }

    (all_tokens, type_tokens)
}

/// Shared tokenizer guard: keep lowercase tokens of at least three characters
/// that the stoplist does not cover.
fn keep_token(lower: &str) -> bool {
    lower.len() >= 3 && !STOPLIST.contains(&lower)
}

fn load_symbols(conn: &Connection) -> Result<HashMap<i64, SymbolData>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT s.id, s.name, s.qualname, s.file_id, f.path, s.line_start, s.line_end, s.signature,
                {} AS callers
         FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE s.is_test = 0 AND s.kind IN ('function', 'method')",
        crate::db::CALLER_COUNT_SQL
    ))?;

    let mut map = HashMap::new();
    let rows = stmt.query_map([], |r| {
        let name: String = r.get(1)?;
        let signature: String = r.get(7)?;
        let (tokens, type_tokens) = tokenize_name_sig(&name, &signature);
        Ok(SymbolData {
            id: r.get(0)?,
            name,
            qualname: r.get(2)?,
            file_id: r.get(3)?,
            file_path: r.get(4)?,
            line_start: r.get(5)?,
            line_end: r.get(6)?,
            resolved_callees: HashSet::new(),
            all_callees: HashSet::new(),
            callee_ids: HashSet::new(),
            inbound_callers: r.get(8)?,
            tokens,
            type_tokens,
        })
    })?;

    for row in rows {
        let sym = row?;
        map.insert(sym.id, sym);
    }
    Ok(map)
}

fn load_callees(conn: &Connection, symbols: &mut HashMap<i64, SymbolData>) -> Result<()> {
    load_resolved_callees(conn, symbols)?;
    map_unresolved_callees(conn, symbols)
}

/// Fold the resolved `calls` edges into each source symbol's callee sets.
fn load_resolved_callees(conn: &Connection, symbols: &mut HashMap<i64, SymbolData>) -> Result<()> {
    let mut edge_stmt =
        conn.prepare("SELECT e.src_id, e.dst_id, e.dst_name FROM edges e WHERE e.kind = 'calls'")?;
    let rows = edge_stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;

    for row in rows {
        let (src_id, dst_id, dst_name) = row?;
        if let Some(sym) = symbols.get_mut(&src_id) {
            let bare = dst_name.rsplit('.').next().unwrap_or(&dst_name).to_string();
            sym.all_callees.insert(bare.clone());
            if let Some(did) = dst_id {
                sym.callee_ids.insert(did);
                sym.resolved_callees.insert(bare);
            }
        }
    }
    Ok(())
}

/// Attribute each unresolved call row to the innermost callable enclosing its
/// site line, so its bare name still counts as a callee of that symbol.
fn map_unresolved_callees(conn: &Connection, symbols: &mut HashMap<i64, SymbolData>) -> Result<()> {
    let mut unres_stmt = conn.prepare(&format!(
        "SELECT u.file_id, u.site_line, u.dst_name FROM unresolved_calls u WHERE u.kind = '{}'",
        EdgeSiteKind::Call.as_str()
    ))?;
    let unres_rows = unres_stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;

    let spans = span_index(symbols);
    for r in unres_rows {
        let (file_id, site_line, dst_name) = r?;
        let bare = dst_name.rsplit('.').next().unwrap_or(&dst_name).to_string();
        // Map unresolved call to the innermost callable enclosing site_line
        // (smallest span): candidates end where line_start passes the site.
        let Some(candidates) = spans.get(&file_id) else {
            continue;
        };
        let prefix = candidates.partition_point(|&(start, _, _)| start <= site_line);
        let innermost = candidates[..prefix]
            .iter()
            .filter(|&&(_, end, _)| end >= site_line)
            .min_by_key(|&&(start, end, _)| end - start)
            .map(|&(_, _, id)| id);

        if let Some(sym) = innermost.and_then(|id| symbols.get_mut(&id)) {
            sym.all_callees.insert(bare);
        }
    }
    Ok(())
}

/// Per-file span index `(line_start, line_end, symbol id)` sorted by
/// line_start, so the innermost-enclosing-span search touches one file's
/// line_start prefix instead of scanning the whole symbol map per unresolved
/// row.
fn span_index(symbols: &HashMap<i64, SymbolData>) -> HashMap<i64, Vec<(i64, i64, i64)>> {
    let mut spans: HashMap<i64, Vec<(i64, i64, i64)>> = HashMap::new();
    for sym in symbols.values() {
        spans
            .entry(sym.file_id)
            .or_default()
            .push((sym.line_start, sym.line_end, sym.id));
    }
    for file_spans in spans.values_mut() {
        file_spans.sort_unstable();
    }
    spans
}

fn build_stoplist(symbols: &HashMap<i64, SymbolData>) -> HashSet<String> {
    let mut stopset: HashSet<String> = STOPLIST.iter().map(|&s| s.to_string()).collect();
    let mut freq: HashMap<String, usize> = HashMap::new();
    let total = symbols.len();

    for sym in symbols.values() {
        for callee in &sym.all_callees {
            *freq.entry(callee.clone()).or_default() += 1;
        }
    }

    let threshold = (total as f64 * 0.05).max(4.0) as usize;
    for (callee, count) in freq {
        if count >= threshold {
            stopset.insert(callee);
        }
    }
    stopset
}

fn select_targets<'a>(
    symbols: &'a HashMap<i64, SymbolData>,
    path_filter: Option<&[String]>,
    target_names: Option<&HashSet<String>>,
) -> Vec<&'a SymbolData> {
    symbols
        .values()
        .filter(|s| {
            if let Some(names) = target_names {
                if !names.contains(&s.qualname) && !names.contains(&s.name) {
                    return false;
                }
            }
            if let Some(paths) = path_filter {
                if !paths.iter().any(|p| s.file_path.starts_with(p)) {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// Deterministic ordering: candidate is more canonical if it has more callers,
/// or alphabetically earlier file/line/qualname as tie-breaker.
fn is_more_canonical(candidate: &SymbolData, target: &SymbolData) -> bool {
    if candidate.inbound_callers != target.inbound_callers {
        candidate.inbound_callers > target.inbound_callers
    } else {
        (
            candidate.file_path.as_str(),
            candidate.line_start,
            candidate.qualname.as_str(),
        ) < (
            target.file_path.as_str(),
            target.line_start,
            target.qualname.as_str(),
        )
    }
}

/// The identity block every detector builds from a matched (target,
/// candidate) pair; detectors only differ in the overlap signal above it.
/// `shared` is sorted into the candidate's display items.
fn make_candidate(
    detector: &'static str,
    target: &SymbolData,
    candidate: &SymbolData,
    overlap_pct: u64,
    shared: &[String],
) -> ReuseCandidate {
    let mut shared_items: Vec<String> = shared.to_vec();
    shared_items.sort();
    ReuseCandidate {
        detector,
        symbol: target.qualname.clone(),
        file: target.file_path.clone(),
        line: target.line_start,
        canonical: candidate.qualname.clone(),
        canonical_file: candidate.file_path.clone(),
        canonical_line: candidate.line_start,
        canonical_callers: candidate.inbound_callers,
        overlap_pct,
        shared_items,
    }
}

/// Shared D1/D2 scan frame: walk the corpus, drop the target's own id/file
/// and non-canonical candidates, let `signal` produce the `(pct, shared)`
/// overlap, and keep the best by overlap percentage. An equal percentage
/// breaks the tie by the canonical key (file_path, line_start, qualname
/// ascending) so the winner does not depend on HashMap iteration order.
fn best_by_overlap(
    detector: &'static str,
    target: &SymbolData,
    symbols: &HashMap<i64, SymbolData>,
    signal: impl Fn(&SymbolData) -> Option<(u64, Vec<String>)>,
) -> Option<ReuseCandidate> {
    let mut best: Option<(u64, &SymbolData, Vec<String>)> = None;
    for candidate in symbols.values() {
        if candidate.id == target.id {
            continue;
        }
        // If in the same file, drop enclosing/overlapping spans (e.g. methods in class or nested fns)
        if candidate.file_id == target.file_id
            && !(candidate.line_end < target.line_start || candidate.line_start > target.line_end)
        {
            continue;
        }
        if !is_more_canonical(candidate, target) {
            continue;
        }
        let Some((pct, shared)) = signal(candidate) else {
            continue;
        };
        let better = match &best {
            None => true,
            Some((best_pct, best_sym, _)) => {
                pct > *best_pct || (pct == *best_pct && is_more_canonical(candidate, best_sym))
            }
        };
        if better {
            best = Some((pct, candidate, shared));
        }
    }
    best.map(|(pct, candidate, shared)| make_candidate(detector, target, candidate, pct, &shared))
}

/// Detector D1: Callee Footprint Containment.
/// Flag when (g, f) not in calls, |C(f)| >= 3, |resolved(g) ∩ resolved(f)| >= 2,
/// and max(|C(g) ∩ C(f)| / |C(f)|, |C(g) ∩ C(f)| / |C(g)|) >= 0.80.
fn find_best_d1(target: &SymbolData, symbols: &HashMap<i64, SymbolData>) -> Option<ReuseCandidate> {
    if target.all_callees.len() < 3 || target.resolved_callees.len() < 2 {
        return None;
    }
    best_by_overlap("D1", target, symbols, |candidate| {
        let c_callees = &candidate.all_callees;
        if c_callees.len() < 3 || candidate.resolved_callees.len() < 2 {
            return None;
        }
        if target.callee_ids.contains(&candidate.id)
            || candidate.callee_ids.contains(&target.id)
            || target.all_callees.contains(&candidate.name)
            || candidate.all_callees.contains(&target.name)
        {
            return None;
        }
        if target
            .resolved_callees
            .intersection(&candidate.resolved_callees)
            .count()
            < 2
        {
            return None;
        }
        let shared: Vec<String> = target
            .all_callees
            .intersection(c_callees)
            .cloned()
            .collect();
        // Guard size balance: min callees must be at least 50% of max callees
        let min_len = target.all_callees.len().min(c_callees.len());
        let max_len = target.all_callees.len().max(c_callees.len());
        if min_len * 2 < max_len {
            return None;
        }
        let ratio_candidate = shared.len() as f64 / c_callees.len() as f64;
        let ratio_target = shared.len() as f64 / target.all_callees.len() as f64;
        let ratio = ratio_candidate.max(ratio_target);
        if ratio < 0.80 {
            return None;
        }
        Some(((ratio * 100.0).round() as u64, shared))
    })
}

/// Detector D2: Interface & Signature Token Overlap.
fn find_best_d2(target: &SymbolData, symbols: &HashMap<i64, SymbolData>) -> Option<ReuseCandidate> {
    if target.tokens.len() < 2 {
        return None;
    }
    best_by_overlap("D2", target, symbols, |candidate| {
        if candidate.tokens.len() < 2 {
            return None;
        }
        let shared: Vec<String> = target
            .tokens
            .intersection(&candidate.tokens)
            .cloned()
            .collect();
        if shared.len() < 2 {
            return None;
        }
        let union_len = target.tokens.union(&candidate.tokens).count();
        let jaccard = shared.len() as f64 / union_len as f64;
        let type_jaccard = if !target.type_tokens.is_empty() && !candidate.type_tokens.is_empty() {
            let shared_types = target
                .type_tokens
                .intersection(&candidate.type_tokens)
                .count();
            let union_types = target.type_tokens.union(&candidate.type_tokens).count();
            shared_types as f64 / union_types as f64
        } else {
            0.0
        };
        if jaccard < 0.60 && !(jaccard >= 0.40 && type_jaccard >= 0.50) {
            return None;
        }
        Some(((jaccard * 100.0).round() as u64, shared))
    })
}

/// Detector D3: Raw Client / Abstraction Bypass.
fn check_d3(target: &SymbolData, symbols: &HashMap<i64, SymbolData>) -> Option<ReuseCandidate> {
    let raw_client = target
        .all_callees
        .iter()
        .find(|c| RAW_CLIENTS.iter().any(|r| r.contains(c.as_str())))?;

    let mut best_wrapper: Option<&SymbolData> = None;
    for s in symbols.values() {
        let same_file_overlap = s.file_id == target.file_id
            && !(s.line_end < target.line_start || s.line_start > target.line_end);
        if s.id != target.id
            && !same_file_overlap
            && is_more_canonical(s, target)
            && s.all_callees
                .iter()
                .any(|c| RAW_CLIENTS.iter().any(|r| r.contains(c.as_str())))
            // A caller-count tie breaks by the canonical key so the winner
            // does not depend on HashMap iteration order.
            && best_wrapper.is_none_or(|bw| {
                s.inbound_callers > bw.inbound_callers
                    || (s.inbound_callers == bw.inbound_callers && is_more_canonical(s, bw))
            })
        {
            best_wrapper = Some(s);
        }
    }
    let canonical = best_wrapper?;
    if canonical.inbound_callers < 2 {
        return None;
    }

    Some(make_candidate(
        "D3",
        target,
        canonical,
        90,
        &[format!("raw_client={raw_client}")],
    ))
}

/// Detector D4: Guarded Pass-Through Wrapper.
fn check_d4(target: &SymbolData, symbols: &HashMap<i64, SymbolData>) -> Option<ReuseCandidate> {
    let lines = target.line_end - target.line_start;
    if lines > 4
        || target.callee_ids.len() != 1
        || target.all_callees.len() != 1
        || target.inbound_callers > 1
    {
        return None;
    }

    let callee_id = *target.callee_ids.iter().next()?;
    let callee = symbols.get(&callee_id)?;
    if callee.id == target.id || callee.file_id == target.file_id {
        return None;
    }

    Some(make_candidate(
        "D4",
        target,
        callee,
        100,
        std::slice::from_ref(&callee.name),
    ))
}
