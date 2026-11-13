//! Phase 4 opt-in coverage detectors. All results are advisory, capped at
//! Medium confidence, and never included in the default scan.

use std::collections::{HashMap, HashSet};

use super::corpus::{load_callees, load_symbols, SymbolData};
use super::tokens::{build_stoplist, tokenize_name_sig};
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
    for sym in symbols.values_mut() {
        sym.all_callees
            .retain(|c| !stopset.contains(c.as_str()) || is_raw_client(c.as_str()));
        sym.resolved_callees
            .retain(|c| !stopset.contains(c.as_str()));
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
    });
    Ok(pool)
}

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
    let mut methods: HashMap<String, HashSet<String>> = HashMap::new();
    for (_, _, qualname, _, _, _, _) in &classes {
        let mut stmt = conn.prepare(
            "SELECT name FROM symbols WHERE kind = 'method' AND qualname LIKE ?1 ESCAPE '\\'",
        )?;
        let prefix = format!("{qualname}.%");
        let names = stmt.query_map([prefix], |r| r.get::<_, String>(0))?;
        methods.insert(
            qualname.clone(),
            names.collect::<rusqlite::Result<HashSet<_>>>()?,
        );
    }
    let mut pool = Vec::new();
    for (i, left) in classes.iter().enumerate() {
        for right in classes.iter().skip(i + 1) {
            if left.1 != right.1 || left.4 == right.4 {
                continue;
            }
            let left_methods = methods.get(&left.2).cloned().unwrap_or_default();
            let right_methods = methods.get(&right.2).cloned().unwrap_or_default();
            let shared: Vec<String> = left_methods.intersection(&right_methods).cloned().collect();
            if shared.is_empty() && left.1 != right.1 {
                continue;
            }
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
        }
    }
    Ok(pool)
}

fn unresolved_candidates(symbols: &HashMap<i64, SymbolData>) -> Result<Vec<ReuseCandidate>> {
    let mut by_name: HashMap<&str, &SymbolData> = HashMap::new();
    for symbol in symbols.values() {
        by_name.entry(symbol.name.as_str()).or_insert(symbol);
    }
    let mut pool = Vec::new();
    for target in symbols.values() {
        for callee in target.all_callees.difference(&target.resolved_callees) {
            let Some(canonical) = by_name.get(callee.as_str()).copied() else {
                continue;
            };
            if canonical.id == target.id || canonical.file_id == target.file_id {
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
                confidence: ReuseConfidence::Medium,
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
        let (name_tokens, _, _) = tokenize_name_sig(&name, &signature);
        if name_tokens.is_empty() {
            continue;
        }
        let mut best: Option<(&SymbolData, usize)> = None;
        for candidate in production.values() {
            let shared = name_tokens.intersection(&candidate.name_tokens).count();
            if shared >= 2 && best.is_none_or(|(_, n)| shared > n) {
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
