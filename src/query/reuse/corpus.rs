//! Corpus loading and target selection: symbol rows, callee folding (resolved
//! and unresolved), and the deterministic target filter.

use super::tokens::tokenize_name_sig;
use super::*;
use std::collections::HashMap;

use crate::extract::EdgeSiteKind;

pub(super) struct SymbolData {
    pub(super) id: i64,
    pub(super) name: String,
    pub(super) qualname: String,
    pub(super) file_id: i64,
    pub(super) file_path: String,
    pub(super) line_start: i64,
    pub(super) line_end: i64,
    pub(super) resolved_callees: HashSet<String>,
    pub(super) all_callees: HashSet<String>,
    pub(super) callee_ids: HashSet<i64>,
    /// Weighted 1-hop (1.0) and 2-hop (0.5) callee footprint, populated only
    /// when `reuse --deep-callee` asks for the transitive signal.
    pub(super) deep_weights: HashMap<String, f64>,
    pub(super) inbound_callers: i64,
    pub(super) tokens: HashSet<String>,
    pub(super) type_tokens: HashSet<String>,
    /// Tokens from the symbol's own name, separate from its signature. Two
    /// functions that only share a parameter shape must not read as twins.
    pub(super) name_tokens: HashSet<String>,
}

pub(super) fn load_symbols(conn: &Connection) -> Result<HashMap<i64, SymbolData>> {
    // Caller counts are folded from the call edges `load_resolved_callees`
    // already reads: the previous per-row `CALLER_COUNT_SQL` subquery repeated
    // 5,000 correlated counts on large corpora.
    let mut stmt = conn.prepare(
        "SELECT s.id, s.name, s.qualname, s.file_id, f.path, s.line_start, s.line_end, s.signature
         FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE s.is_test = 0 AND s.kind IN ('function', 'method')",
    )?;

    let mut map = HashMap::new();
    let rows = stmt.query_map([], |r| {
        let name: String = r.get(1)?;
        let signature: String = r.get(7)?;
        let (tokens, type_tokens, name_tokens) = tokenize_name_sig(&name, &signature);
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
            deep_weights: HashMap::new(),
            inbound_callers: 0,
            tokens,
            type_tokens,
            name_tokens,
        })
    })?;

    for row in rows {
        let sym = row?;
        map.insert(sym.id, sym);
    }
    Ok(map)
}

pub(super) fn load_callees(
    conn: &Connection,
    symbols: &mut HashMap<i64, SymbolData>,
) -> Result<()> {
    load_resolved_callees(conn, symbols)?;
    map_unresolved_callees(conn, symbols)
}

/// Build the weighted two-hop footprint after the stoplist filter: direct
/// resolved callees weigh 1.0, callees of those callees weigh 0.5. A callee
/// reachable both ways keeps the direct 1.0 weight.
pub(super) fn load_deep_weights(symbols: &mut HashMap<i64, SymbolData>) {
    let computed: HashMap<i64, HashMap<String, f64>> = symbols
        .iter()
        .map(|(&id, sym)| {
            let mut weights: HashMap<String, f64> = sym
                .resolved_callees
                .iter()
                .map(|callee| (callee.clone(), 1.0))
                .collect();
            for callee_id in &sym.callee_ids {
                if let Some(callee) = symbols.get(callee_id) {
                    for second in &callee.resolved_callees {
                        weights.entry(second.clone()).or_insert(0.5);
                    }
                }
            }
            (id, weights)
        })
        .collect();
    for (id, weights) in computed {
        if let Some(sym) = symbols.get_mut(&id) {
            sym.deep_weights = weights;
        }
    }
}

fn callee_name(dst_name: &str) -> String {
    if is_raw_client(dst_name) {
        dst_name.to_string()
    } else {
        dst_name.rsplit('.').next().unwrap_or(dst_name).to_string()
    }
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
        if let Some(did) = dst_id {
            if let Some(dst) = symbols.get_mut(&did) {
                dst.inbound_callers += 1;
            }
        }
        if let Some(sym) = symbols.get_mut(&src_id) {
            let callee = callee_name(&dst_name);
            sym.all_callees.insert(callee.clone());
            if let Some(did) = dst_id {
                sym.callee_ids.insert(did);
                sym.resolved_callees.insert(callee);
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
    let mut unres_rows = unres_stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;

    // Peek one row before building the span index: a corpus with no unresolved
    // calls (the common case in a fully resolved repo) must not pay the
    // per-file span sort. Streaming keeps the previous bounded memory profile.
    let Some(first) = unres_rows.next() else {
        return Ok(());
    };
    let spans = span_index(symbols);
    for r in std::iter::once(first).chain(unres_rows) {
        let (file_id, site_line, dst_name) = r?;
        let callee = callee_name(&dst_name);
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
            sym.all_callees.insert(callee);
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

pub(super) fn select_targets<'a>(
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
