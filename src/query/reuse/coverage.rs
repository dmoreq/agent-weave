//! Phase 4 opt-in coverage detectors. All results are advisory, capped at
//! Medium confidence, and never included in the default scan.

use std::collections::{HashMap, HashSet};

use super::corpus::{load_callees, load_symbols, SymbolData};
use super::guards::is_more_canonical;
use super::tokens::{build_name_stoplist, build_stoplist, tokenize_name_sig};
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CoverageKind {
    Class,
    Unresolved,
    Tests,
}

impl CoverageKind {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "class" => Some(Self::Class),
            "unresolved" => Some(Self::Unresolved),
            "tests" => Some(Self::Tests),
            _ => None,
        }
    }
}

pub(super) fn analyze(conn: &Connection, kinds: &[CoverageKind]) -> Result<Vec<ReuseCandidate>> {
    let mut pool = Vec::new();
    if kinds.contains(&CoverageKind::Class) {
        pool.extend(class_candidates(conn)?);
    }
    let mut symbols = load_symbols(conn)?;
    load_callees(conn, &mut symbols)?;
    let stopset = build_stoplist(&symbols);
    let name_stopset = build_name_stoplist(&symbols);
    for sym in symbols.values_mut() {
        sym.all_callees
            .retain(|c| !stopset.contains(c.as_str()) || is_raw_client(c.as_str()));
        sym.unresolved_callees
            .retain(|c| !stopset.contains(c.as_str()) || is_raw_client(c.as_str()));
        sym.resolved_callees
            .retain(|c| !stopset.contains(c.as_str()));
        sym.name_tokens
            .retain(|token| !name_stopset.contains(token));
    }
    if kinds.contains(&CoverageKind::Unresolved) {
        pool.extend(unresolved_candidates(&symbols)?);
    }
    if kinds.contains(&CoverageKind::Tests) {
        pool.extend(test_candidates(conn, &symbols)?);
    }
    pool.sort_by(|a, b| {
        b.confidence
            .rank()
            .cmp(&a.confidence.rank())
            .then_with(|| a.symbol.cmp(&b.symbol))
            .then_with(|| a.canonical.cmp(&b.canonical))
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.canonical_file.cmp(&b.canonical_file))
            .then_with(|| a.canonical_line.cmp(&b.canonical_line))
    });
    Ok(pool)
}

/// D5 is pairwise; cap the materialized pool so thousands of same-named
/// classes cannot exhaust memory before finalize truncates the output.
const MAX_CLASS_CANDIDATES: usize = 10_000;

fn class_candidates(conn: &Connection) -> Result<Vec<ReuseCandidate>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, qualname, file_id, file_path, line_start, line_end
         FROM symbols WHERE kind = 'class' AND is_test = 0
         ORDER BY qualname, file_path",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, i64>(6)?,
        ))
    })?;
    let classes: Vec<_> = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut methods: HashMap<(i64, String), HashSet<String>> = HashMap::new();
    for (_, _, qualname, file_id, _, _, _) in &classes {
        // Scope methods to the defining file: two files can produce the same
        // qualname (`a/b.py` and `a.b.py` both map to `a.b`), and a
        // qualname-only query would merge their unrelated method sets.
        let mut stmt = conn.prepare(
            "SELECT name FROM symbols
             WHERE kind = 'method' AND file_id = ?1 AND qualname LIKE ?2 ESCAPE '\\'",
        )?;
        let prefix = format!("{}.%", escape_like(qualname));
        let names = stmt.query_map(rusqlite::params![file_id, prefix], |r| {
            r.get::<_, String>(0)
        })?;
        methods.insert(
            (*file_id, qualname.clone()),
            names.collect::<rusqlite::Result<HashSet<_>>>()?,
        );
    }
    let mut pool = Vec::new();
    'classes: for (i, left) in classes.iter().enumerate() {
        for right in classes.iter().skip(i + 1) {
            if left.1 != right.1
                || left.4 == right.4
                || language_family(&left.4) != language_family(&right.4)
            {
                continue;
            }
            let left_methods = methods
                .get(&(left.3, left.2.clone()))
                .cloned()
                .unwrap_or_default();
            let right_methods = methods
                .get(&(right.3, right.2.clone()))
                .cloned()
                .unwrap_or_default();
            let mut shared: Vec<String> =
                left_methods.intersection(&right_methods).cloned().collect();
            if shared.is_empty() {
                continue;
            }
            shared.sort();
            let confidence = if shared.len() >= 2 {
                ReuseConfidence::Medium
            } else {
                ReuseConfidence::Low
            };
            pool.push(ReuseCandidate {
                detector: DetectorId::D5,
                symbol: left.2.clone(),
                file: left.4.clone(),
                line: left.5,
                canonical: right.2.clone(),
                canonical_file: right.4.clone(),
                canonical_line: right.5,
                canonical_callers: 0,
                overlap_pct: None,
                shared_items: shared,
                raw_client: None,
                confidence,
                reasons: vec![ReasonCode::CoverageClass],
                freshness: Freshness::Fresh,
            });
            if pool.len() >= MAX_CLASS_CANDIDATES {
                break 'classes;
            }
        }
    }
    Ok(pool)
}

fn escape_like(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn is_generic_coverage_name(name: &str) -> bool {
    matches!(
        name,
        "default" | "new" | "call" | "apply" | "then" | "map" | "get" | "set" | "run"
    )
}

fn language_family(file: &str) -> String {
    let ext = file.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "py" | "pyi" => "python".to_string(),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => "tsjs".to_string(),
        "rs" => "rust".to_string(),
        other => other.to_ascii_lowercase(),
    }
}

fn unresolved_candidates(symbols: &HashMap<i64, SymbolData>) -> Result<Vec<ReuseCandidate>> {
    let mut by_name: HashMap<(&str, String), &SymbolData> = HashMap::new();
    for symbol in symbols.values() {
        let key = (symbol.name.as_str(), language_family(&symbol.file_path));
        by_name
            .entry(key)
            .and_modify(|current| {
                if is_more_canonical(symbol, current) {
                    *current = symbol;
                }
            })
            .or_insert(symbol);
    }
    let mut pool = Vec::new();
    for target in symbols.values() {
        let family = language_family(&target.file_path);
        for callee in &target.unresolved_callees {
            if is_generic_coverage_name(callee) {
                continue;
            }
            let Some(canonical) = by_name.get(&(callee.as_str(), family.clone())).copied() else {
                continue;
            };
            if canonical.id == target.id
                || canonical.file_id == target.file_id
                || language_family(&target.file_path) != language_family(&canonical.file_path)
            {
                continue;
            }
            pool.push(ReuseCandidate {
                detector: DetectorId::D1L,
                symbol: target.qualname.clone(),
                file: target.file_path.clone(),
                line: target.line_start,
                canonical: canonical.qualname.clone(),
                canonical_file: canonical.file_path.clone(),
                canonical_line: canonical.line_start,
                canonical_callers: canonical.inbound_callers,
                overlap_pct: None,
                shared_items: vec![callee.clone()],
                raw_client: None,
                confidence: ReuseConfidence::Low,
                reasons: vec![ReasonCode::CoverageUnresolved],
                freshness: Freshness::Fresh,
            });
        }
    }
    Ok(pool)
}

fn test_candidates(
    conn: &Connection,
    production: &HashMap<i64, SymbolData>,
) -> Result<Vec<ReuseCandidate>> {
    let mut stmt = conn.prepare(
        "SELECT name, qualname, file_path, line_start, signature
         FROM symbols WHERE is_test = 1 AND kind IN ('function', 'method')
         ORDER BY qualname, file_path",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, String>(4)?,
        ))
    })?;
    let tests: Vec<_> = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut pool = Vec::new();
    for (name, qualname, file, line, signature) in tests {
        let (all_tokens, _, name_tokens) = tokenize_name_sig(&name, &signature);
        let name_tokens = if name_tokens.is_empty() {
            all_tokens
        } else {
            name_tokens
        };
        if name_tokens.is_empty() {
            continue;
        }
        let mut best: Option<(&SymbolData, usize)> = None;
        for candidate in production.values() {
            if language_family(&file) != language_family(&candidate.file_path) {
                continue;
            }
            let shared = name_tokens.intersection(&candidate.name_tokens).count();
            if shared < 3 {
                continue;
            }
            let better = best.is_none_or(|(current, current_shared)| {
                shared > current_shared
                    || (shared == current_shared && is_more_canonical(candidate, current))
            });
            if better {
                best = Some((candidate, shared));
            }
        }
        let Some((canonical, shared)) = best else {
            continue;
        };
        pool.push(ReuseCandidate {
            detector: DetectorId::DT,
            symbol: qualname,
            file,
            line,
            canonical: canonical.qualname.clone(),
            canonical_file: canonical.file_path.clone(),
            canonical_line: canonical.line_start,
            canonical_callers: canonical.inbound_callers,
            overlap_pct: None,
            shared_items: vec![format!("{shared} shared tokens")],
            raw_client: None,
            confidence: ReuseConfidence::Low,
            reasons: vec![ReasonCode::CoverageTests],
            freshness: Freshness::Fresh,
        });
    }
    Ok(pool)
}
