//! Terminal ANSI layout rendering for the multi-tab TUI monitor.

use std::io::{self, Write};

use serde_json::Value;

use super::state::{classify_usp_pillar, format_timestamp, truncate_chars, MonitorTab, TuiState};
use crate::error::ExitClass;

pub fn render_screen<W: Write>(
    w: &mut W,
    state: &TuiState,
    cols: usize,
    rows: usize,
) -> io::Result<()> {
    write!(w, "\x1b[H")?;
    let mut lines_rendered = 0;

    lines_rendered += render_header(w, state, cols)?;

    let content_height = rows.saturating_sub(lines_rendered + 2);
    lines_rendered += match state.active_tab {
        MonitorTab::Graph => render_tab_graph(w, state, cols, content_height)?,
        MonitorTab::Reuse => render_tab_reuse(w, state, cols, content_height)?,
        MonitorTab::Memory => render_tab_memory(w, state, cols, content_height)?,
        MonitorTab::Stream => render_tab_stream(w, state, cols, content_height)?,
    };

    while lines_rendered + 1 < rows {
        writeln!(w, "\x1b[2K")?;
        lines_rendered += 1;
    }

    render_footer(w, state, cols)?;
    w.flush()
}

fn render_header<W: Write>(w: &mut W, state: &TuiState, cols: usize) -> io::Result<usize> {
    let mode_str = if state.errors_only {
        " | Mode: CRITICAL ERRORS"
    } else {
        ""
    };
    let filter_str = match &state.repo_filter {
        Some(f) if f != "ALL" => format!(" | Filter: {f}"),
        _ => String::new(),
    };

    let title = format!(
        "aw monitor v0.5.3 | Repos: {}{}{}",
        state.total_repos, filter_str, mode_str
    );
    writeln!(w, "\x1b[2K\x1b[1;37m{title}\x1b[0m")?;

    let tabs = [
        MonitorTab::Graph,
        MonitorTab::Reuse,
        MonitorTab::Memory,
        MonitorTab::Stream,
    ];
    write!(w, "\x1b[2K")?;
    for t in tabs {
        if state.active_tab == t {
            write!(w, "\x1b[7;1m [ {} ] \x1b[0m  ", t.title())?;
        } else {
            write!(w, "\x1b[90m [ {} ] \x1b[0m  ", t.title())?;
        }
    }
    writeln!(w)?;

    let hr_len = cols.clamp(80, 120);
    writeln!(w, "\x1b[2K\x1b[90m{}\x1b[0m", "─".repeat(hr_len))?;
    Ok(3)
}

fn render_footer<W: Write>(w: &mut W, _state: &TuiState, _cols: usize) -> io::Result<usize> {
    writeln!(
        w,
        "\x1b[2K\x1b[90mKeys: \x1b[37m1-4\x1b[90m Tabs | \x1b[37mr\x1b[90m Refresh | \x1b[37mq\x1b[90m Exit\x1b[0m"
    )?;
    Ok(1)
}

fn render_tab_graph<W: Write>(
    w: &mut W,
    state: &TuiState,
    cols: usize,
    max_lines: usize,
) -> io::Result<usize> {
    let mut count = 0;
    let s = &state.summary;

    let total_calls = s.get("total_calls").and_then(Value::as_u64).unwrap_or(0);
    let graph_calls = s.get("graph_calls").and_then(Value::as_u64).unwrap_or(0);
    let graph_pct = if total_calls > 0 {
        100.0 * (graph_calls as f64) / (total_calls as f64)
    } else {
        0.0
    };
    let rel_pct = s
        .get("graph_reliability_pct")
        .and_then(Value::as_f64)
        .unwrap_or(100.0);
    let graph_errors = s.get("graph_errors").and_then(Value::as_u64).unwrap_or(0);
    let p50 = s.get("graph_p50_ms").and_then(Value::as_f64).unwrap_or(0.0);
    let p95 = s.get("graph_p95_ms").and_then(Value::as_f64).unwrap_or(0.0);
    let saved_tokens = s
        .get("graph_saved_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let saved_dollars = s
        .get("graph_saved_dollars")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);

    writeln!(
        w,
        "\x1b[2K\x1b[1;36m[USP 1: DETERMINISTIC CODE GRAPH & NAVIGATION]\x1b[0m"
    )?;
    writeln!(
        w,
        "\x1b[2K  Calls:       \x1b[1;32m{graph_calls}\x1b[0m calls ({graph_pct:.1}% of corpus)   Reliability: \x1b[1;32m{rel_pct:.1}%\x1b[0m ok ({graph_errors} errors)"
    )?;
    writeln!(
        w,
        "\x1b[2K  Speed:       p50=\x1b[32m{p50:.1}ms\x1b[0m · p95=\x1b[32m{p95:.1}ms\x1b[0m"
    )?;
    writeln!(
        w,
        "\x1b[2K  Tokens:      ~\x1b[1;32m{saved_tokens}\x1b[0m tokens saved (~${saved_dollars:.2})"
    )?;
    writeln!(w, "\x1b[2K")?;
    count += 5;

    writeln!(
        w,
        "\x1b[2K\x1b[1m{:<12} {:>8} {:>9} {:>9} {:>14}  {:<24}\x1b[0m",
        "TOOL", "CALLS", "OK %", "p50", "SAVED TOKENS", "CAPABILITY"
    )?;
    writeln!(
        w,
        "\x1b[2K\x1b[90m{}\x1b[0m",
        "─".repeat(cols.clamp(74, 80))
    )?;
    count += 2;

    let graph_tools = [
        ("callers", "Upstream caller tree"),
        ("callees", "Downstream callee tree"),
        ("path", "Reachable call path route"),
        ("context", "Surrounding scope context"),
        ("impact", "Blast radius impact analysis"),
        ("symbols", "Symbol definition search"),
        ("imports", "Import dependency mapping"),
        ("build", "Incremental graph index build"),
    ];

    let tool_aggs = s.get("tool_aggs").and_then(Value::as_object);
    for (tool, desc) in graph_tools {
        if count + 2 >= max_lines {
            break;
        }
        let (calls, ok_pct, p50_ms, saved) = if let Some(tag) = tool_aggs.and_then(|m| m.get(tool))
        {
            let c = tag.get("calls").and_then(Value::as_u64).unwrap_or(0);
            let rel = tag
                .get("reliability_pct")
                .and_then(Value::as_f64)
                .unwrap_or(100.0);
            let p = tag.get("p50_ms").and_then(Value::as_f64).unwrap_or(0.0);
            let s_tok = tag.get("saved_tokens").and_then(Value::as_u64).unwrap_or(0);
            (c, rel, p, s_tok)
        } else {
            (0, 100.0, 0.0, 0)
        };
        writeln!(
            w,
            "\x1b[2K{:<12} {:>8} {:>8.1}% {:>8.1}ms {:>14}  \x1b[90m{:<24}\x1b[0m",
            tool, calls, ok_pct, p50_ms, saved, desc
        )?;
        count += 1;
    }

    Ok(count)
}

fn render_tab_reuse<W: Write>(
    w: &mut W,
    state: &TuiState,
    cols: usize,
    max_lines: usize,
) -> io::Result<usize> {
    let mut count = 0;
    let s = &state.reuse_summary;

    let total_cands = s
        .get("total_candidates")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let sample_cands = s
        .get("sample_candidates")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let prev_loc = s
        .get("preventable_loc")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let prewrite_count = s.get("prewrite_count").and_then(Value::as_u64).unwrap_or(0);
    let avg_callers = s.get("avg_callers").and_then(Value::as_f64).unwrap_or(0.0);

    let d_counts = s.get("detector_counts");
    let d1 = d_counts
        .and_then(|c| c.get("D1"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let d2 = d_counts
        .and_then(|c| c.get("D2"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let d3 = d_counts
        .and_then(|c| c.get("D3"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let d4 = d_counts
        .and_then(|c| c.get("D4"))
        .and_then(Value::as_u64)
        .unwrap_or(0);

    writeln!(
        w,
        "\x1b[2K\x1b[1;32m[USP 2: REAL-TIME ANTI-REINVENTION ENGINE]\x1b[0m"
    )?;
    writeln!(
        w,
        "\x1b[2K  Duplicate Candidates: \x1b[1;32m{total_cands}\x1b[0m twins        Preventable Excess: ~\x1b[1;32m{prev_loc}\x1b[0m LOC"
    )?;
    writeln!(
        w,
        "\x1b[2K  Pre-Write Searches:   \x1b[1;32m{prewrite_count}\x1b[0m intercepted     Authoritative Ratio: \x1b[1;32m{avg_callers:.1}\x1b[0m callers/canonical"
    )?;
    writeln!(
        w,
        "\x1b[2K  Detectors: D1 Callee Overlap: {d1} · D2 Signature: {d2} · D3 Bypass: {d3} · D4 Wrapper: {d4}"
    )?;
    writeln!(w, "\x1b[2K")?;
    count += 5;

    let sub_title = if total_cands > sample_cands {
        format!("DUPLICATE TWINS (Sample top {sample_cands} of {total_cands})")
    } else {
        "DUPLICATE TWINS".to_string()
    };
    writeln!(w, "\x1b[2K\x1b[1m{sub_title}\x1b[0m")?;
    writeln!(
        w,
        "\x1b[2K\x1b[1m{:<8} {:<4} {:<28} {:<28} {:>7}\x1b[0m",
        "REPO", "DET", "DUPLICATE SYMBOL", "CANONICAL TARGET", "SAVING"
    )?;
    writeln!(
        w,
        "\x1b[2K\x1b[90m{}\x1b[0m",
        "─".repeat(cols.clamp(77, 80))
    )?;
    count += 3;

    for cand in &state.reuse_candidates {
        if count + 2 >= max_lines {
            break;
        }
        let repo = cand.get("repo").and_then(Value::as_str).unwrap_or("-");
        let det = cand.get("detector").and_then(Value::as_str).unwrap_or("-");
        let sym = cand.get("symbol").and_then(Value::as_str).unwrap_or("-");
        let can = cand.get("canonical").and_then(Value::as_str).unwrap_or("-");
        let loc = cand.get("loc_saving").and_then(Value::as_i64).unwrap_or(0);

        let r_trunc = truncate_chars(repo, 7);
        let s_trunc = truncate_chars(sym, 27);
        let c_trunc = truncate_chars(can, 27);

        writeln!(
            w,
            "\x1b[2K{:<8} {:<4} {:<28} {:<28} {:>5} L",
            r_trunc, det, s_trunc, c_trunc, loc
        )?;
        count += 1;
    }

    Ok(count)
}

fn render_tab_memory<W: Write>(
    w: &mut W,
    state: &TuiState,
    cols: usize,
    max_lines: usize,
) -> io::Result<usize> {
    let mut count = 0;
    let s = &state.summary;

    let current_notes: Vec<&Value> = state
        .memories
        .iter()
        .filter(|m| m.get("is_current").and_then(Value::as_bool).unwrap_or(true))
        .collect();

    let gotchas = current_notes
        .iter()
        .filter(|m| m.get("kind").and_then(Value::as_str) == Some("gotcha"))
        .count();
    let decisions = current_notes
        .iter()
        .filter(|m| m.get("kind").and_then(Value::as_str) == Some("decision"))
        .count();

    let tool_aggs = s.get("tool_aggs").and_then(Value::as_object);
    let reads = tool_aggs
        .and_then(|m| m.get("mem_query").or_else(|| m.get("mem_get")))
        .and_then(|a| a.get("calls"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let writes = tool_aggs
        .and_then(|m| m.get("mem_write").or_else(|| m.get("mem_touch")))
        .and_then(|a| a.get("calls"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let rw_ratio = if writes > 0 {
        format!("{:.1}x", reads as f64 / writes as f64)
    } else if reads > 0 {
        format!("{:.1}x", reads as f64)
    } else {
        "0.0x".to_string()
    };

    let session_recall_pct = s
        .get("session_recall_pct")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);

    writeln!(
        w,
        "\x1b[2K\x1b[1;35m[USP 3: ZERO-TOKEN PERSISTENT PROJECT MEMORY]\x1b[0m"
    )?;
    writeln!(
        w,
        "\x1b[2K  Read:Write Ratio:    \x1b[1;32m{rw_ratio}\x1b[0m ({reads} reads · {writes} writes)   Session Recall: \x1b[1;32m{session_recall_pct:.1}%\x1b[0m"
    )?;
    writeln!(
        w,
        "\x1b[2K  Guarded Hotspots:    \x1b[1;32m{}\x1b[0m active notes ({gotchas} gotchas · {decisions} decisions)",
        current_notes.len()
    )?;
    writeln!(w, "\x1b[2K")?;
    count += 4;

    writeln!(
        w,
        "\x1b[2K\x1b[1m{:<8} {:<30} {:<10} {:>5} {:>8}  {:<12}\x1b[0m",
        "STORE", "KEY", "KIND", "REV", "RECALLS", "STATUS"
    )?;
    writeln!(
        w,
        "\x1b[2K\x1b[90m{}\x1b[0m",
        "─".repeat(cols.clamp(75, 80))
    )?;
    count += 2;

    for note in &current_notes {
        if count + 2 >= max_lines {
            break;
        }
        let store = note.get("store").and_then(Value::as_str).unwrap_or("-");
        let key = note.get("key").and_then(Value::as_str).unwrap_or("-");
        let kind = note.get("kind").and_then(Value::as_str).unwrap_or("-");
        let rev = note.get("rev").and_then(Value::as_u64).unwrap_or(1);
        let recalls = note
            .get("recall_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let status = note
            .get("ref_status")
            .and_then(Value::as_str)
            .unwrap_or("valid");

        let k_trunc = truncate_chars(key, 29);
        let (status_str, status_color) = match status {
            "valid" | "none" => ("fresh", "\x1b[32m"),
            "modified" => ("modified", "\x1b[33m"),
            _ => ("broken", "\x1b[31;1m"),
        };

        writeln!(
            w,
            "\x1b[2K{:<8} {:<30} {:<10} {:>4}r {:>8}  {}{:<12}\x1b[0m",
            store, k_trunc, kind, rev, recalls, status_color, status_str
        )?;
        count += 1;
    }

    Ok(count)
}

fn render_tab_stream<W: Write>(
    w: &mut W,
    state: &TuiState,
    _cols: usize,
    max_lines: usize,
) -> io::Result<usize> {
    let mut count = 0;
    writeln!(
        w,
        "\x1b[2K\x1b[1m{:<8} {:<9} {:<12} {:<12} {:<11} {:<9} {:<8} ARGUMENTS\x1b[0m",
        "TIME", "USP", "TOOL", "REPO", "AGENT", "LATENCY", "STATUS"
    )?;
    writeln!(w, "\x1b[2K\x1b[90m{}\x1b[0m", "─".repeat(96))?;
    count += 2;

    for val in &state.recent_events {
        if count + 1 >= max_lines {
            break;
        }
        render_event_row(w, val)?;
        count += 1;
    }

    Ok(count)
}

pub fn render_event_row<W: Write>(w: &mut W, val: &Value) -> io::Result<()> {
    let ts = val.get("ts").and_then(Value::as_i64).unwrap_or(0);
    let time_str = format_timestamp(ts);
    let tool = val.get("tool").and_then(Value::as_str).unwrap_or("-");
    let (usp_tag, usp_color) = classify_usp_pillar(tool);

    let repo_name = val
        .get("repo_name")
        .and_then(Value::as_str)
        .unwrap_or("workspace");
    let r_trunc = truncate_chars(repo_name, 11);

    let agent_name = val
        .get("agent")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("user");
    let a_trunc = truncate_chars(agent_name, 10);

    let lat_str = val
        .get("latency_us")
        .and_then(Value::as_f64)
        .map(|us| format!("{:.1}ms", us / 1000.0))
        .unwrap_or_else(|| "-".to_string());

    let exit_code = val.get("exit_code").and_then(Value::as_i64).unwrap_or(0);
    let (status_str, status_color) = match ExitClass::of(exit_code) {
        ExitClass::Ok => ("OK", "\x1b[32m"),
        ExitClass::Empty => ("EMPTY", "\x1b[33m"),
        ExitClass::Ambiguous => ("AMBIG", "\x1b[35m"),
        ExitClass::Error => ("ERR", "\x1b[31;1m"),
    };

    let args_val = val.get("args").cloned().unwrap_or(Value::Null);
    let args_str = serde_json::to_string(&args_val).unwrap_or_default();
    let max_args_len = 32;
    let short_args = if args_str.len() > max_args_len {
        format!("{}...", truncate_chars(&args_str, max_args_len - 3))
    } else {
        args_str
    };

    writeln!(
        w,
        "\x1b[2K{time_str:<8} {usp_color}[{usp_tag:<6}]\x1b[0m {tool:<12} {r_trunc:<12} {a_trunc:<11} {lat_str:<9} {status_color}{status_str:<8}\x1b[0m {short_args}"
    )
}
