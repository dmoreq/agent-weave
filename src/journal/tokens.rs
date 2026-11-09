//! Token-savings accounting: pricing model, per-tool savings arms, and the
//! number and percent formatting helpers for summary rows.

pub(crate) const PRICE_PER_TOKEN: f64 = 0.000003;

/// Estimated tokens one rendered graph/context row replaces: a pointer the
/// agent would otherwise re-derive by reading the callee/caller source.
const GRAPH_ROW_TOKENS: usize = 350;
/// Estimated tokens one rendered search/symbol row replaces.
const SEARCH_ROW_TOKENS: usize = 300;
/// Estimated tokens one reuse hit replaces (candidate row plus its trimmed
/// code snippet).
const REUSE_RESULT_TOKENS: usize = 800;
/// Estimated tokens one reuse report replaces beyond its rows (the ranked
/// comparison prose the agent would otherwise write itself).
const REUSE_CALL_TOKENS: usize = 1200;
/// Estimated tokens one rendered symbol row replaces.
const SYMBOL_ROW_TOKENS: usize = 150;
/// Estimated tokens one successful build report replaces (the incremental
/// summary the agent would otherwise re-derive by re-running the build).
const BUILD_CALL_TOKENS: usize = 500;
/// Per-call overhead shared by every priced query arm: the result envelope,
/// staleness header, and pointer identity suffixes.
const QUERY_CALL_TOKENS: usize = 100;

/// Journaled tool names deliberately priced at zero: diagnostics, inventory
/// reports, and memory housekeeping replace no source-file read, so they
/// contribute no token-savings estimate. A new journaled tool must either
/// gain a pricing arm in `estimated_tokens_saved` or be listed here — pinned
/// by `service::tests::estimated_tokens_saved_covers_every_journaled_tool`.
pub const UNPRICED_TOOLS: &[&str] = &[
    "doctor",
    "unresolved",
    "stats",
    "usage",
    "mem_write",
    "mem_query",
    "mem_touch",
    "mem_get",
    "mem_delete",
    "mem_gc",
    "mem_export",
];

/// Estimated tokens one tool call saves the agent versus reading the same
/// result out of source files. The single coefficient table for BOTH telemetry
/// surfaces: `aw usage` aggregates journaled calls with it, and the MCP KPI
/// footer prices the single call it just rendered (subtracting that call's own
/// payload) — so the two surfaces cannot report contradictory numbers.
pub fn estimated_tokens_saved(
    tool: &str,
    shown: usize,
    result_count: usize,
    calls: usize,
) -> usize {
    match tool {
        "context" | "callers" | "callees" | "path" | "impact" | "imports" => {
            shown * GRAPH_ROW_TOKENS + calls * QUERY_CALL_TOKENS
        }
        "search" => shown * SEARCH_ROW_TOKENS + calls * QUERY_CALL_TOKENS,
        "reuse" => result_count * REUSE_RESULT_TOKENS + calls * REUSE_CALL_TOKENS,
        "symbols" => shown * SYMBOL_ROW_TOKENS,
        "build" => calls * BUILD_CALL_TOKENS,
        // Deliberately unpriced journaled tools (see UNPRICED_TOOLS).
        t if UNPRICED_TOOLS.contains(&t) => 0,
        // Legacy names from older journal generations, no longer dispatchable.
        _ => 0,
    }
}

pub(super) fn format_number_with_commas(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let len = s.len();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(c);
    }
    result
}

pub(super) fn percent(n: usize, d: usize) -> String {
    if d == 0 {
        "-".to_string()
    } else {
        format!("{:.1}%", 100.0 * n as f64 / d as f64)
    }
}

pub(super) fn percentile(lat: &[u64], q: f64) -> String {
    if lat.is_empty() {
        return "-".to_string();
    }
    let idx = ((lat.len() as f64 * q).ceil() as usize)
        .saturating_sub(1)
        .min(lat.len() - 1);
    format!("{:.1}ms", lat[idx] as f64 / 1000.0)
}
