//! Terminal ANSI layout rendering for the multi-tab TUI monitor.
//! Calibrated for standard 80x24 terminals with zero line wrapping and zero scrolling.

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
    let safe_cols = cols.clamp(40, 78);
    let avail_lines = rows.saturating_sub(5).max(4);

    write!(w, "\x1b[H")?;

    render_header(w, state, safe_cols)?;

    match state.active_tab {
        MonitorTab::Graph => render_tab_graph(w, state, safe_cols, avail_lines)?,
        MonitorTab::Reuse => render_tab_reuse(w, state, safe_cols, avail_lines)?,
        MonitorTab::Memory => render_tab_memory(w, state, safe_cols, avail_lines)?,
        MonitorTab::Stream => render_tab_stream(w, state, safe_cols, avail_lines)?,
    };

    write!(w, "\x1b[J")?;
    render_footer(w, rows)?;
    w.flush()
}

fn render_header<W: Write>(w: &mut W, state: &TuiState, safe_cols: usize) -> io::Result<()> {
    let mode_str = if state.errors_only { " | ERRORS" } else { "" };
    let filter_str = match &state.repo_filter {
        Some(f) if f != "ALL" => format!(" | Stream repo: {f}"),
        _ => String::new(),
    };

    let title = format!(
        "eg monitor v{} | Repos: {}{}{}",
        env!("CARGO_PKG_VERSION"),
        state.total_repos,
        filter_str,
        mode_str
    );
    let title_trunc = truncate_chars(&title, safe_cols);
    write!(w, "\r\x1b[2K\x1b[1;37m{title_trunc}\x1b[0m\r\n")?;
    if state
        .summary
        .get("total_calls")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        == 0
    {
        let hint = format!(
            "no journal entries yet — watching {}",
            state.log_path.display()
        );
        let hint = truncate_chars(&hint, safe_cols);
        write!(w, "\r\x1b[2K\x1b[90m{hint}\x1b[0m\r\n")?;
    }

    write!(w, "\r\x1b[2K")?;
    let tabs = [
        MonitorTab::Graph,
        MonitorTab::Reuse,
        MonitorTab::Memory,
        MonitorTab::Stream,
    ];
    for t in tabs {
        if state.active_tab == t {
            write!(w, "\x1b[7;1m [ {} ] \x1b[0m ", t.title())?;
        } else {
            write!(w, "\x1b[90m [ {} ] \x1b[0m ", t.title())?;
        }
    }
    write!(w, "\r\n")?;

    write!(w, "\r\x1b[2K\x1b[90m{}\x1b[0m\r\n", "─".repeat(safe_cols))?;
    Ok(())
}

fn render_footer<W: Write>(w: &mut W, rows: usize) -> io::Result<()> {
    write!(
        w,
        "\x1b[{rows};1H\r\x1b[2K\x1b[90mKeys: \x1b[37m1-4/Arrows/Tab\x1b[90m Tabs | \x1b[37mr\x1b[90m Refresh | \x1b[37mq\x1b[90m Exit\x1b[0m"
    )
}

fn render_tab_graph<W: Write>(
    w: &mut W,
    state: &TuiState,
    safe_cols: usize,
    avail_lines: usize,
) -> io::Result<()> {
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

    write!(w, "\r\x1b[2K\x1b[1;36m[GRAPH]\x1b[0m\r\n")?;
    write!(
        w,
        "\r\x1b[2K  Calls: \x1b[1;32m{graph_calls}\x1b[0m ({graph_pct:.1}%)  Rel: \x1b[1;32m{rel_pct:.1}%\x1b[0m ok  Errors: \x1b[33m{graph_errors}\x1b[0m\r\n"
    )?;
    write!(
        w,
        "\r\x1b[2K  Speed: p50=\x1b[32m{p50:.1}ms\x1b[0m · p95=\x1b[32m{p95:.1}ms\x1b[0m  Tokens: ~\x1b[1;32m{saved_tokens}\x1b[0m (${saved_dollars:.2})\r\n"
    )?;
    write!(
        w,
        "\r\x1b[2K\x1b[1m{:<9} {:>6} {:>7} {:>7} {:>10}  {:<24}\x1b[0m\r\n",
        "TOOL", "CALLS", "OK %", "p50", "SAVED", "PURPOSE"
    )?;
    write!(w, "\r\x1b[2K\x1b[90m{}\x1b[0m\r\n", "─".repeat(safe_cols))?;

    let graph_tools = [
        ("callers", "Upstream caller tree"),
        ("callees", "Downstream callee tree"),
        ("path", "Reachable call path route"),
        ("context", "Surrounding scope context"),
        ("impact", "Blast radius impact"),
        ("symbols", "Symbol definition search"),
        ("imports", "Import dependency mapping"),
        ("build", "Incremental index build"),
    ];

    let tool_aggs = s.get("tool_aggs").and_then(Value::as_object);
    let max_rows = avail_lines.saturating_sub(5);
    for (idx, (tool, desc)) in graph_tools.iter().enumerate() {
        if idx >= max_rows {
            break;
        }
        let (calls, ok_str, p50_str, saved) =
            if let Some(tag) = tool_aggs.and_then(|m| m.get(*tool)) {
                let c = tag.get("calls").and_then(Value::as_u64).unwrap_or(0);
                let errors = tag.get("errors").and_then(Value::as_u64).unwrap_or(0);
                let ok = if c > 0 {
                    format!(
                        "{:.1}%",
                        100.0 * (c.saturating_sub(errors)) as f64 / c as f64
                    )
                } else {
                    "-".to_string()
                };
                let p = match tag.get("p50").and_then(Value::as_f64) {
                    Some(v) if c > 0 => format!("{v:.1}ms"),
                    _ => "-".to_string(),
                };
                let s_tok = tag.get("saved_tokens").and_then(Value::as_u64).unwrap_or(0);
                (c, ok, p, s_tok)
            } else {
                (0, "-".to_string(), "-".to_string(), 0)
            };
        write!(
            w,
            "\r\x1b[2K{:<9} {:>6} {:>7} {:>7} {:>10}  \x1b[90m{:<24}\x1b[0m\r\n",
            tool, calls, ok_str, p50_str, saved, desc
        )?;
    }

    Ok(())
}

fn render_tab_reuse<W: Write>(
    w: &mut W,
    state: &TuiState,
    safe_cols: usize,
    avail_lines: usize,
) -> io::Result<()> {
    let s = &state.reuse_summary;
    let total_cands = s
        .get("total_candidates")
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

    write!(w, "\r\x1b[2K\x1b[1;32m[ANTI-REINVENTION]\x1b[0m\r\n")?;
    write!(
        w,
        "\r\x1b[2K  Duplicates: \x1b[1;32m{total_cands}\x1b[0m twins  Preventable: ~\x1b[1;32m{prev_loc}\x1b[0m LOC\r\n"
    )?;
    write!(
        w,
        "\r\x1b[2K  Pre-writes: \x1b[1;32m{prewrite_count}\x1b[0m caught  Ratio: \x1b[1;32m{avg_callers:.1}\x1b[0m callers  D1:{d1} D2:{d2} D3:{d3} D4:{d4}\r\n"
    )?;
    write!(
        w,
        "\r\x1b[2K\x1b[1m{:<7} {:<4} {:<25} {:<25} {:>7}\x1b[0m\r\n",
        "REPO", "DET", "DUPLICATE SYMBOL", "CANONICAL TARGET", "SAVING"
    )?;
    write!(w, "\r\x1b[2K\x1b[90m{}\x1b[0m\r\n", "─".repeat(safe_cols))?;

    let max_rows = avail_lines.saturating_sub(5);
    for (idx, cand) in state.reuse_candidates.iter().enumerate() {
        if idx >= max_rows {
            break;
        }
        let repo = cand.get("repo").and_then(Value::as_str).unwrap_or("-");
        let det = cand.get("detector").and_then(Value::as_str).unwrap_or("-");
        let sym = cand.get("symbol").and_then(Value::as_str).unwrap_or("-");
        let can = cand.get("canonical").and_then(Value::as_str).unwrap_or("-");
        let loc = cand.get("loc_saving").and_then(Value::as_i64).unwrap_or(0);

        let r_trunc = truncate_chars(repo, 6);
        let s_trunc = truncate_chars(sym, 24);
        let c_trunc = truncate_chars(can, 24);

        write!(
            w,
            "\r\x1b[2K{:<7} {:<4} {:<25} {:<25} {:>5} L\r\n",
            r_trunc, det, s_trunc, c_trunc, loc
        )?;
    }

    Ok(())
}

fn render_tab_memory<W: Write>(
    w: &mut W,
    state: &TuiState,
    safe_cols: usize,
    avail_lines: usize,
) -> io::Result<()> {
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
    // Reads and writes are summed over every memory tool in the shared
    // classification table: an OR-picked single tool undercounts.
    let sum_calls = |classify: fn(&str) -> crate::dashboard::data::ToolClass,
                     want: fn(&crate::dashboard::data::ToolClass) -> bool| {
        tool_aggs
            .map(|m| {
                m.iter()
                    .filter(|(tool, _)| want(&classify(tool)))
                    .filter_map(|(_, agg)| agg.get("calls").and_then(Value::as_u64))
                    .sum::<u64>()
            })
            .unwrap_or(0)
    };
    let reads = sum_calls(crate::dashboard::data::classify, |c| c.mem_read);
    let writes = sum_calls(crate::dashboard::data::classify, |c| c.mem_write);
    let rw_ratio = if writes > 0 {
        format!("{:.1}x", reads as f64 / writes as f64)
    } else if reads > 0 {
        format!("{:.1}x", reads as f64)
    } else {
        "0.0x".to_string()
    };

    // No tagged session means the metric is unmeasurable, not zero.
    let has_sessions = s.get("session_count").and_then(Value::as_u64).unwrap_or(0) > 0;
    let recall = if has_sessions {
        format!(
            "{:.1}%",
            s.get("session_recall_pct")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        )
    } else {
        "—".to_string()
    };

    write!(w, "\r\x1b[2K\x1b[1;35m[MEMORY]\x1b[0m\r\n")?;
    write!(
        w,
        "\r\x1b[2K  R:W Ratio: \x1b[1;32m{rw_ratio}\x1b[0m ({reads}r/{writes}w)  Session Recall: \x1b[1;32m{recall}\x1b[0m\r\n"
    )?;
    write!(
        w,
        "\r\x1b[2K  Guarded: \x1b[1;32m{}\x1b[0m active notes ({gotchas} gotchas · {decisions} decisions)\r\n",
        current_notes.len()
    )?;
    write!(
        w,
        "\r\x1b[2K\x1b[1m{:<7} {:<28} {:<9} {:>4} {:>7}  {:<8}\x1b[0m\r\n",
        "STORE", "KEY", "KIND", "REV", "RECALLS", "STATUS"
    )?;
    write!(w, "\r\x1b[2K\x1b[90m{}\x1b[0m\r\n", "─".repeat(safe_cols))?;

    let max_rows = avail_lines.saturating_sub(5);
    for (idx, note) in current_notes.iter().enumerate() {
        if idx >= max_rows {
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

        let k_trunc = truncate_chars(key, 27);
        let store_trunc = truncate_chars(store, 7);
        let (status_str, status_color) = match status {
            "valid" => ("fresh", "\x1b[32m"),
            "none" => ("no-refs", "\x1b[90m"),
            "modified" => ("modified", "\x1b[33m"),
            "unverified" => ("unverif", "\x1b[90m"),
            _ => ("broken", "\x1b[31;1m"),
        };

        write!(
            w,
            "\r\x1b[2K{:<7} {:<28} {:<9} {:>3}r {:>7}  {}{:<8}\x1b[0m\r\n",
            store_trunc, k_trunc, kind, rev, recalls, status_color, status_str
        )?;
    }

    Ok(())
}

fn render_tab_stream<W: Write>(
    w: &mut W,
    state: &TuiState,
    safe_cols: usize,
    avail_lines: usize,
) -> io::Result<()> {
    write!(
        w,
        "\r\x1b[2K\x1b[1m{:<8} {:<8} {:<10} {:<10} {:<10} {:<7} {:<6} ARGS\x1b[0m\r\n",
        "UTC", "PILLAR", "TOOL", "REPO", "AGENT", "LAT", "STATUS"
    )?;
    write!(w, "\r\x1b[2K\x1b[90m{}\x1b[0m\r\n", "─".repeat(safe_cols))?;

    let max_rows = avail_lines.saturating_sub(2);
    for (idx, val) in state.recent_events.iter().enumerate() {
        if idx >= max_rows {
            break;
        }
        render_event_row(w, val, safe_cols)?;
    }

    Ok(())
}

pub fn render_event_row<W: Write>(w: &mut W, val: &Value, safe_cols: usize) -> io::Result<()> {
    let ts = val.get("ts").and_then(Value::as_i64).unwrap_or(0);
    let time_str = format_timestamp(ts);
    let tool = val.get("tool").and_then(Value::as_str).unwrap_or("-");
    let (usp_tag, usp_color) = classify_usp_pillar(tool);

    let repo_name = val
        .get("repo_name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            val.get("repo")
                .and_then(Value::as_str)
                .and_then(|p| std::path::Path::new(p).file_name().and_then(|n| n.to_str()))
        })
        .unwrap_or("-");
    let r_trunc = truncate_chars(repo_name, 9);

    let agent_name = val
        .get("agent")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("user");
    let a_trunc = truncate_chars(agent_name, 9);

    // Latency never exceeds the 7-column budget: seconds above 999 ms.
    let lat_str = val
        .get("latency_us")
        .and_then(Value::as_f64)
        .map(|us| {
            if us >= 1_000_000.0 {
                format!("{:.2}s", us / 1_000_000.0)
            } else {
                format!("{:.1}ms", us / 1000.0)
            }
        })
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
    // Prefix is 67 columns; keep one spare column so an 80-col terminal never wraps.
    let max_args_len = safe_cols.saturating_sub(68).max(4);
    let short_args = if args_str.len() > max_args_len {
        format!("{}...", truncate_chars(&args_str, max_args_len - 3))
    } else {
        args_str
    };

    write!(
        w,
        "\r\x1b[2K{time_str:<8} {usp_color}[{usp_tag:<6}]\x1b[0m {tool:<10} {r_trunc:<10} {a_trunc:<10} {lat_str:<7} {status_color}{status_str:<6}\x1b[0m {short_args}\r\n"
    )
}
