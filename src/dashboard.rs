//! Interactive telemetry, anti-reinvention, and memory dashboard module.
//! Zero-dependency native Rust HTTP 1.1 + SSE server and ANSI query monitor.

pub mod data;
pub mod monitor;
pub mod server;

use std::path::Path;

use crate::error::Result;

pub fn run_dashboard(
    root: Option<&Path>,
    host: &str,
    port: u16,
    open: bool,
    local: bool,
) -> Result<()> {
    server::serve_dashboard(root, host, port, open, local)
}

pub fn run_monitor(
    root: Option<&Path>,
    repo_filter: Option<&str>,
    errors_only: bool,
    local: bool,
) -> Result<()> {
    monitor::stream_monitor(root, repo_filter, errors_only, local)
}

#[cfg(test)]
mod tip_tests {
    //! The hover-help copy lives in `app.js` and the hooks in `index.html`.
    //! Neither file is compiled, so a key typo would ship a blank tooltip.
    const HTML: &str = include_str!("dashboard/assets/index.html");
    const JS: &str = include_str!("dashboard/assets/app.js");
    const ATTR: &str = "data-tip=\"";

    fn html_keys() -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut rest = HTML;
        while let Some(pos) = rest.find(ATTR) {
            let tail = &rest[pos + ATTR.len()..];
            let end = tail.find('"').unwrap_or(0);
            out.push(tail[..end].to_string());
            rest = &tail[end..];
        }
        out.sort();
        out.dedup();
        out
    }

    fn copy_keys() -> Vec<String> {
        let start = JS.find("const TIP = {").expect("TIP map missing");
        let block = &JS[start..];
        let end = block.find("};").unwrap_or(block.len());
        let mut out: Vec<String> = Vec::new();
        for line in block[..end].lines() {
            let Some(open) = line.find('\'') else {
                continue;
            };
            let Some(close) = line[open + 1..].find('\'') else {
                continue;
            };
            out.push(line[open + 1..open + 1 + close].to_string());
        }
        out.sort();
        out.dedup();
        out
    }

    #[test]
    fn every_hook_has_copy() {
        for key in html_keys() {
            let needle = format!("'{key}':");
            assert!(JS.contains(&needle), "no tooltip copy for {key}");
        }
    }

    #[test]
    fn graph_tool_list_matches_server_classification() {
        let start = JS
            .find("const GRAPH_TOOLS = [")
            .expect("GRAPH_TOOLS literal");
        let end = JS[start..].find("];").expect("array end");
        let mut js_tools: Vec<&str> = JS[start..start + end]
            .split('\'')
            .skip(1)
            .step_by(2)
            .collect();
        js_tools.sort_unstable();
        let expected = [
            "build", "callees", "callers", "context", "impact", "imports", "path", "symbols",
        ];
        assert_eq!(
            js_tools, expected,
            "client GRAPH_TOOLS drifted from the server list"
        );
        for tool in expected {
            assert!(
                crate::dashboard::data::classify(tool).graph,
                "{tool} must be graph"
            );
        }
        assert!(!crate::dashboard::data::classify("stats").graph);
        assert!(!crate::dashboard::data::classify("reuse").graph);
    }

    #[test]
    fn client_has_no_aggregate_mirror_or_token_model() {
        assert!(
            !JS.contains("estimateEventTokens"),
            "token estimator must stay deleted"
        );
        assert!(
            !JS.contains("patchSummaryState"),
            "aggregate mirror must stay deleted"
        );
        assert!(
            !JS.contains("PRICE_PER_TOKEN"),
            "price constant is server-side only"
        );
    }

    #[test]
    fn switch_tab_has_an_allowlist_guard() {
        assert!(
            JS.contains("TAB_KEYS.includes(tab)"),
            "switchTab must reject an unknown key instead of hiding every panel"
        );
        assert!(
            !JS.contains("switchTab('graph')"),
            "the legacy 'graph' tab id must be gone"
        );
    }

    #[test]
    fn every_copy_is_hooked() {
        for key in copy_keys() {
            let needle = format!("{ATTR}{key}\"");
            // Dynamic rows build their hooks in app.js template strings, so a
            // key may be attached from either asset.
            assert!(
                HTML.contains(&needle) || JS.contains(&needle),
                "unused tooltip copy: {key}"
            );
        }
    }
}

#[cfg(test)]
mod client_contract_tests {
    //! Static contracts over the embedded client bundle. No JS harness exists,
    //! so the invariants the browser depends on are pinned here.
    const HTML: &str = include_str!("dashboard/assets/index.html");
    const JS: &str = include_str!("dashboard/assets/app.js");

    #[test]
    fn no_inline_event_handlers_remain() {
        for bad in ["onclick=", "onchange=", "oninput=", "onsubmit="] {
            assert!(!HTML.contains(bad), "index.html still has {bad}");
            assert!(!JS.contains(bad), "app.js still emits {bad}");
        }
    }

    #[test]
    fn filters_survive_refetch_and_no_silent_catch_remains() {
        assert!(
            JS.contains("renderRecentQueries(data.graph_recent_queries || []);\n      applyTelemetryFilters();"),
            "the stream rebuild must re-apply active filters"
        );
        assert!(
            !JS.contains(".catch(() => {})"),
            "failed fetches must surface, not vanish"
        );
        assert!(
            JS.contains("showFetchError("),
            "per-panel error state expected"
        );
        assert!(
            JS.contains("reset: repository no longer active"),
            "invalid filter restore notice"
        );
    }

    #[test]
    fn live_feeds_show_only_the_newest_ten() {
        assert!(
            JS.contains("const FEED_LIMIT = 10;"),
            "feed limit constant missing"
        );
        assert!(
            JS.contains("graphQueries.slice(0, FEED_LIMIT)"),
            "recent-queries feed must cap at the newest ten"
        );
        assert!(
            JS.contains("prewriteSearches.slice(0, FEED_LIMIT)"),
            "prewrite feed must cap at the newest ten"
        );
        assert!(
            HTML.contains("id=\"recent-list-count\""),
            "recent counter missing"
        );
        assert!(
            HTML.contains("id=\"prewrite-list-count\""),
            "prewrite counter missing"
        );
    }

    #[test]
    fn command_arguments_read_as_plain_language() {
        assert!(
            JS.contains("function explainToolArgs("),
            "plain-language command helper missing"
        );
        assert!(
            JS.contains("explainToolArgs(q.tool, q.args) || argsShort"),
            "recent-queries feed must use the plain-language summary"
        );
        assert!(
            JS.contains("explainToolArgs(s.tool, s.tool === 'reuse' ? { paths: s.terms } : { terms: s.terms })"),
            "prewrite feed must use the plain-language summary"
        );
        assert!(JS.contains("Look for duplicate code in every file"));
    }

    #[test]
    fn reuse_cards_show_surfaced_usage_and_last_used() {
        assert!(
            JS.contains("function reuseUsageText("),
            "reuse usage helper missing"
        );
        assert!(
            JS.contains("last used '"),
            "reuse card must show the last-used time"
        );
        assert!(JS.contains("card.usage"), "usage tooltip hook missing");
        assert!(
            JS.contains("never surfaced in retained journal"),
            "zero-usage copy missing"
        );
    }

    #[test]
    fn list_views_are_collapsible_and_deterministic() {
        assert!(
            HTML.contains("id=\"mem-view\""),
            "memory view select missing"
        );
        assert!(
            HTML.contains("id=\"reuse-view\""),
            "reuse view select missing"
        );
        assert!(
            HTML.contains("<option value=\"recent3\" selected>Top 3</option>"),
            "memory default view must be Top 3"
        );
        assert!(
            HTML.contains("<option value=\"used3\" selected>Top 3 by callers</option>"),
            "reuse default view must be Top 3 by callers"
        );
        assert!(
            JS.contains("function viewMemories("),
            "memory ranking helper missing"
        );
        assert!(
            JS.contains("function viewReuseCandidates("),
            "reuse ranking helper missing"
        );
        assert!(JS.contains("memoryActivity"), "activity sort key missing");
        assert!(
            JS.contains("recalled ' + (m.recall_count || 0)"),
            "card must show the used count"
        );
        assert!(
            JS.contains("last used '"),
            "card must show the last-used time"
        );
        assert!(
            JS.contains("card.verified"),
            "card must show the verify time"
        );
        assert!(
            JS.contains("Showing ' + shown + ' of ' + total"),
            "list counter missing"
        );
    }

    #[test]
    fn touch_is_store_addressed_with_a_rev_precondition() {
        assert!(JS.contains("store_id: storeId"), "touch sends store_id");
        assert!(JS.contains("rev: rev"), "touch sends the displayed rev");
        assert!(
            JS.contains("data-store-id"),
            "touch buttons carry the store id"
        );
        assert!(
            JS.contains("event: reset") || JS.contains("addEventListener('reset'"),
            "named reset listener"
        );
    }
}
