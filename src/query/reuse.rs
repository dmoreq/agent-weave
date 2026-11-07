//! Anti-reinvention detection engine (D1-D4).
//! Referencing RepoReuse (arXiv:2609.35357) and codebase-minimalism.
//! Sub-20ms calibrated static duplicate detectors.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::error::Result;
use crate::extract::split_ident;
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
        match self.detector {
            "D1" => format!(
                "symbol '{}' shares {}% callee footprint with existing '{}' ([callers={}] in {}:{})",
                self.symbol,
                self.overlap_pct,
                self.canonical,
                self.canonical_callers,
                self.canonical_file,
                self.canonical_line
            ),
            "D2" => format!(
                "symbol '{}' shares {}% interface token overlap with existing '{}' ([callers={}] in {}:{})",
                self.symbol,
                self.overlap_pct,
                self.canonical,
                self.canonical_callers,
                self.canonical_file,
                self.canonical_line
            ),
            "D3" => format!(
                "symbol '{}' bypasses canonical client '{}' to call raw external client ([callers={}] in {}:{})",
                self.symbol,
                self.canonical,
                self.canonical_callers,
                self.canonical_file,
                self.canonical_line
            ),
            "D4" => format!(
                "symbol '{}' is a pass-through wrapper around existing '{}' ([callers={}] in {}:{})",
                self.symbol,
                self.canonical,
                self.canonical_callers,
                self.canonical_file,
                self.canonical_line
            ),
            _ => format!(
                "symbol '{}' potential duplication of existing '{}' ([callers={}] in {}:{})",
                self.symbol,
                self.canonical,
                self.canonical_callers,
                self.canonical_file,
                self.canonical_line
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
    let candidates = analyze_corpus(conn, None, Some(new_names), 3)?;
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
    let candidates = analyze_corpus(conn, path_filter, None, limit)?;

    if candidates.is_empty() {
        return Ok((vec!["0 reuse candidates found".to_string()], String::new()));
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
    Ok((out, String::new()))
}

fn analyze_corpus(
    conn: &Connection,
    path_filter: Option<&[String]>,
    target_names: Option<&HashSet<String>>,
    limit: usize,
) -> Result<Vec<ReuseCandidate>> {
    let mut symbols = load_symbols(conn)?;
    if symbols.len() < 2 {
        return Ok(Vec::new());
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
        if let Some(c) = find_best_d1(target, &symbols) {
            if seen_pairs.insert((c.symbol.clone(), c.canonical.clone())) {
                candidates.push(c);
            }
            continue;
        }
        if let Some(c) = find_best_d2(target, &symbols) {
            if seen_pairs.insert((c.symbol.clone(), c.canonical.clone())) {
                candidates.push(c);
            }
            continue;
        }
        if let Some(c) = check_d3(target, &symbols) {
            if seen_pairs.insert((c.symbol.clone(), c.canonical.clone())) {
                candidates.push(c);
            }
            continue;
        }
        if let Some(c) = check_d4(target, &symbols) {
            if seen_pairs.insert((c.symbol.clone(), c.canonical.clone())) {
                candidates.push(c);
            }
        }
    }

    candidates.sort_by(|a, b| {
        b.overlap_pct
            .cmp(&a.overlap_pct)
            .then_with(|| b.canonical_callers.cmp(&a.canonical_callers))
            .then_with(|| a.symbol.cmp(&b.symbol))
    });
    candidates.truncate(limit);
    Ok(candidates)
}

fn tokenize_name_sig(name: &str, sig: &str) -> (HashSet<String>, HashSet<String>) {
    let mut all_tokens = HashSet::new();
    let mut type_tokens = HashSet::new();

    for tok in split_ident(name).split_whitespace() {
        let lower = tok.to_ascii_lowercase();
        if lower.len() >= 3 && !STOPLIST.contains(&lower.as_str()) {
            all_tokens.insert(lower);
        }
    }

    for part in sig.split(['(', ')', ',', '{', ';']) {
        if let Some((_, type_part)) = part.split_once(':') {
            for tok in split_ident(type_part).split_whitespace() {
                let lower = tok.to_ascii_lowercase();
                if lower.len() >= 3 && !STOPLIST.contains(&lower.as_str()) {
                    type_tokens.insert(lower.clone());
                    all_tokens.insert(lower);
                }
            }
        }
        if let Some((_, ret_part)) = part.split_once("->") {
            for tok in split_ident(ret_part).split_whitespace() {
                let lower = tok.to_ascii_lowercase();
                if lower.len() >= 3 && !STOPLIST.contains(&lower.as_str()) {
                    type_tokens.insert(lower.clone());
                    all_tokens.insert(lower);
                }
            }
        }
    }

    for tok in split_ident(sig).split_whitespace() {
        let lower = tok.to_ascii_lowercase();
        if lower.len() >= 3 && !STOPLIST.contains(&lower.as_str()) {
            all_tokens.insert(lower);
        }
    }

    (all_tokens, type_tokens)
}

fn load_symbols(conn: &Connection) -> Result<HashMap<i64, SymbolData>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.name, s.qualname, s.file_id, f.path, s.line_start, s.line_end, s.signature,
                (SELECT COUNT(*) FROM edges e WHERE e.dst_id = s.id AND e.kind = 'calls') AS callers
         FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE s.is_test = 0 AND s.kind IN ('function', 'method')",
    )?;

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

    let mut unres_stmt = conn.prepare(
        "SELECT u.file_id, u.site_line, u.dst_name FROM unresolved_calls u WHERE u.kind = 'call'",
    )?;
    let unres_rows = unres_stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;

    for r in unres_rows {
        let (file_id, site_line, dst_name) = r?;
        let bare = dst_name.rsplit('.').next().unwrap_or(&dst_name).to_string();
        // Map unresolved call to the innermost callable enclosing site_line (smallest span).
        let innermost = symbols
            .values_mut()
            .filter(|sym| {
                sym.file_id == file_id && site_line >= sym.line_start && site_line <= sym.line_end
            })
            .min_by_key(|sym| sym.line_end - sym.line_start);

        if let Some(sym) = innermost {
            sym.all_callees.insert(bare);
        }
    }
    Ok(())
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

/// Detector D1: Callee Footprint Containment.
/// Flag when (g, f) not in calls, |C(f)| >= 3, |resolved(g) ∩ resolved(f)| >= 2,
/// and |C(g) ∩ C(f)| / |C(f)| >= 0.80.
fn find_best_d1(target: &SymbolData, symbols: &HashMap<i64, SymbolData>) -> Option<ReuseCandidate> {
    let t_callees = &target.all_callees;
    if t_callees.len() < 3 || target.resolved_callees.len() < 2 {
        return None;
    }

    let mut best: Option<ReuseCandidate> = None;

    for candidate in symbols.values() {
        if candidate.id == target.id || candidate.file_id == target.file_id {
            continue;
        }
        let c_callees = &candidate.all_callees;
        if c_callees.len() < 3 || candidate.resolved_callees.len() < 2 {
            continue;
        }
        if !is_more_canonical(candidate, target) {
            continue;
        }
        if target.callee_ids.contains(&candidate.id)
            || candidate.callee_ids.contains(&target.id)
            || target.all_callees.contains(&candidate.name)
            || candidate.all_callees.contains(&target.name)
        {
            continue;
        }

        let resolved_shared = target
            .resolved_callees
            .intersection(&candidate.resolved_callees)
            .count();
        if resolved_shared < 2 {
            continue;
        }

        let shared: Vec<String> = t_callees.intersection(c_callees).cloned().collect();
        let ratio = shared.len() as f64 / c_callees.len() as f64;
        if ratio < 0.80 {
            continue;
        }

        let pct = (ratio * 100.0).round() as u64;
        if best.as_ref().is_none_or(|b| pct > b.overlap_pct) {
            let mut sorted_shared = shared;
            sorted_shared.sort();
            best = Some(ReuseCandidate {
                detector: "D1",
                symbol: target.qualname.clone(),
                file: target.file_path.clone(),
                line: target.line_start,
                canonical: candidate.qualname.clone(),
                canonical_file: candidate.file_path.clone(),
                canonical_line: candidate.line_start,
                canonical_callers: candidate.inbound_callers,
                overlap_pct: pct,
                shared_items: sorted_shared,
            });
        }
    }
    best
}

/// Detector D2: Interface & Signature Token Overlap.
fn find_best_d2(target: &SymbolData, symbols: &HashMap<i64, SymbolData>) -> Option<ReuseCandidate> {
    if target.tokens.len() < 2 {
        return None;
    }
    let t_tokens = &target.tokens;
    let t_types = &target.type_tokens;

    let mut best: Option<ReuseCandidate> = None;

    for candidate in symbols.values() {
        if candidate.id == target.id || candidate.file_id == target.file_id {
            continue;
        }
        if candidate.tokens.len() < 2 {
            continue;
        }
        if !is_more_canonical(candidate, target) {
            continue;
        }
        let c_tokens = &candidate.tokens;
        let shared: Vec<String> = t_tokens.intersection(c_tokens).cloned().collect();
        if shared.len() < 2 {
            continue;
        }

        let union_len = t_tokens.union(c_tokens).count();
        let jaccard = shared.len() as f64 / union_len as f64;

        let type_jaccard = if !t_types.is_empty() && !candidate.type_tokens.is_empty() {
            let shared_types = t_types.intersection(&candidate.type_tokens).count();
            let union_types = t_types.union(&candidate.type_tokens).count();
            shared_types as f64 / union_types as f64
        } else {
            0.0
        };

        if jaccard < 0.60 && !(jaccard >= 0.40 && type_jaccard >= 0.50) {
            continue;
        }

        let pct = (jaccard * 100.0).round() as u64;
        if best.as_ref().is_none_or(|b| pct > b.overlap_pct) {
            let mut sorted_shared = shared;
            sorted_shared.sort();
            best = Some(ReuseCandidate {
                detector: "D2",
                symbol: target.qualname.clone(),
                file: target.file_path.clone(),
                line: target.line_start,
                canonical: candidate.qualname.clone(),
                canonical_file: candidate.file_path.clone(),
                canonical_line: candidate.line_start,
                canonical_callers: candidate.inbound_callers,
                overlap_pct: pct,
                shared_items: sorted_shared,
            });
        }
    }
    best
}

/// Detector D3: Raw Client / Abstraction Bypass.
fn check_d3(target: &SymbolData, symbols: &HashMap<i64, SymbolData>) -> Option<ReuseCandidate> {
    let raw_client = target
        .all_callees
        .iter()
        .find(|c| RAW_CLIENTS.iter().any(|r| r.contains(c.as_str())))?;

    let mut best_wrapper: Option<&SymbolData> = None;
    for s in symbols.values() {
        if s.id != target.id
            && s.file_id != target.file_id
            && is_more_canonical(s, target)
            && s.all_callees
                .iter()
                .any(|c| RAW_CLIENTS.iter().any(|r| r.contains(c.as_str())))
            && best_wrapper.is_none_or(|bw| s.inbound_callers > bw.inbound_callers)
        {
            best_wrapper = Some(s);
        }
    }
    let canonical = best_wrapper?;
    if canonical.inbound_callers < 2 {
        return None;
    }

    Some(ReuseCandidate {
        detector: "D3",
        symbol: target.qualname.clone(),
        file: target.file_path.clone(),
        line: target.line_start,
        canonical: canonical.qualname.clone(),
        canonical_file: canonical.file_path.clone(),
        canonical_line: canonical.line_start,
        canonical_callers: canonical.inbound_callers,
        overlap_pct: 90,
        shared_items: vec![format!("raw_client={raw_client}")],
    })
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

    Some(ReuseCandidate {
        detector: "D4",
        symbol: target.qualname.clone(),
        file: target.file_path.clone(),
        line: target.line_start,
        canonical: callee.qualname.clone(),
        canonical_file: callee.file_path.clone(),
        canonical_line: callee.line_start,
        canonical_callers: callee.inbound_callers,
        overlap_pct: 100,
        shared_items: vec![callee.name.clone()],
    })
}
