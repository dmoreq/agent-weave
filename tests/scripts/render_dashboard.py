#!/usr/bin/env python3
"""Interactive Telemetry & Index Dashboard Generator for `aw` (`agent-weave`).

Self-contained, zero-dependency script reading `.aw/usage.log` (local or global)
and `.aw/index.db`, producing a responsive, single-file HTML dashboard with hero metrics,
SVG tool distribution, multi-repo worktree breakdown, error & bug triage, and
interactive filtering.
"""

from __future__ import annotations

import argparse
import html
import json
import math
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import time
from typing import Any, Dict, List, Optional, Tuple


def resolve_global_aw_dir() -> Path:
    """Resolve global ~/.aw directory, honoring AW_GLOBAL_DIR."""
    val = os.environ.get("AW_GLOBAL_DIR")
    if val and val.strip():
        return Path(val.strip())
    home = os.environ.get("HOME") or os.environ.get("USERPROFILE") or "."
    return Path(home) / ".aw"


def parse_usage_log_from_dir(aw_dir: Path) -> List[Dict[str, Any]]:
    """Load journal entries from usage.log.1 and usage.log in aw_dir."""
    entries: List[Dict[str, Any]] = []
    for filename in ["usage.log.1", "usage.log"]:
        log_file = aw_dir / filename
        if not log_file.is_file():
            continue
        try:
            with open(log_file, "r", encoding="utf-8", errors="replace") as f:
                for line in f:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        entries.append(json.loads(line))
                    except json.JSONDecodeError:
                        continue
        except OSError:
            continue
    return entries


def parse_usage_log(root: Path) -> List[Dict[str, Any]]:
    """Load journal entries from root/.aw/."""
    return parse_usage_log_from_dir(root / ".aw")


def inspect_index_db(root: Optional[Path]) -> Dict[str, Any]:
    """Inspect .aw/index.db for indexed file and symbol counts."""
    stats: Dict[str, Any] = {
        "exists": False,
        "size_bytes": 0,
        "files": 0,
        "symbols": 0,
        "edges": 0,
        "languages": {},
    }
    if not root:
        return stats

    db_path = root / ".aw" / "index.db"
    if not db_path.is_file():
        return stats

    stats["exists"] = True
    try:
        stats["size_bytes"] = db_path.stat().st_size
        conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
        cur = conn.cursor()
        cur.execute("SELECT COUNT(*) FROM files")
        stats["files"] = cur.fetchone()[0]

        cur.execute("SELECT COUNT(*) FROM symbols")
        stats["symbols"] = cur.fetchone()[0]

        cur.execute("SELECT name FROM sqlite_master WHERE type='table' AND name='edges'")
        if cur.fetchone():
            cur.execute("SELECT COUNT(*) FROM edges")
            stats["edges"] = cur.fetchone()[0]

        cur.execute("SELECT path FROM files")
        for (fpath,) in cur.fetchall():
            ext = Path(fpath).suffix
            if ext:
                stats["languages"][ext] = stats["languages"].get(ext, 0) + 1
        conn.close()
    except (sqlite3.Error, OSError):
        pass

    return stats


def calculate_metrics(entries: List[Dict[str, Any]]) -> Dict[str, Any]:
    """Calculate aggregated telemetry metrics, per-repo breakdown, and errors."""
    total_calls = len(entries)
    if total_calls == 0:
        return {
            "total_calls": 0,
            "error_count": 0,
            "stale_reads": 0,
            "reliability_pct": 100.0,
            "p50_ms": 0.0,
            "p95_ms": 0.0,
            "avg_ms": 0.0,
            "max_ms": 0.0,
            "saved_tokens": 0,
            "saved_dollars": 0.0,
            "reuse_hits": 0,
            "tool_aggs": {},
            "repo_aggs": {},
            "recent_queries": [],
            "recent_errors": [],
        }

    errors = 0
    stale_reads = 0
    reuse_hits = 0
    saved_tokens = 0
    latencies: List[float] = []
    tool_aggs: Dict[str, Dict[str, Any]] = {}
    repo_aggs: Dict[str, Dict[str, Any]] = {}
    all_errors: List[Dict[str, Any]] = []

    for e in entries:
        tool = str(e.get("tool", "unknown"))
        exit_code = int(e.get("exit_code", 0))
        shown = int(e.get("shown", e.get("result_count", 0)))
        result_count = int(e.get("result_count", 0))
        stale = bool(e.get("stale", False))
        lat_us = e.get("latency_us")
        ts = e.get("ts")
        repo_name = str(e.get("repo_name") or "workspace")
        is_worktree = bool(e.get("worktree", False))

        if tool not in tool_aggs:
            tool_aggs[tool] = {
                "calls": 0,
                "errors": 0,
                "shown": 0,
                "latencies": [],
            }

        tool_aggs[tool]["calls"] += 1
        tool_aggs[tool]["shown"] += shown

        if repo_name not in repo_aggs:
            repo_aggs[repo_name] = {
                "calls": 0,
                "errors": 0,
                "worktree": is_worktree,
                "last_ts": ts,
            }

        repo_aggs[repo_name]["calls"] += 1
        if ts and (repo_aggs[repo_name]["last_ts"] is None or ts > repo_aggs[repo_name]["last_ts"]):
            repo_aggs[repo_name]["last_ts"] = ts

        if exit_code != 0:
            errors += 1
            tool_aggs[tool]["errors"] += 1
            repo_aggs[repo_name]["errors"] += 1
            all_errors.append(e)

        if stale:
            stale_reads += 1

        if lat_us is not None and lat_us >= 0:
            ms = lat_us / 1000.0
            latencies.append(ms)
            tool_aggs[tool]["latencies"].append(ms)

        # Token savings model
        if exit_code == 0:
            if tool in ("context", "callers", "callees", "path", "impact", "imports"):
                saved_tokens += shown * 350 + 100
            elif tool == "search":
                saved_tokens += shown * 300 + 100
            elif tool == "reuse":
                reuse_hits += max(result_count, shown)
                saved_tokens += result_count * 800 + 1200
            elif tool == "symbols":
                saved_tokens += shown * 150
            elif tool == "build":
                saved_tokens += 500

    latencies.sort()
    n = len(latencies)
    p50 = latencies[int(math.ceil(0.50 * n)) - 1] if n else 0.0
    p95 = latencies[int(math.ceil(0.95 * n)) - 1] if n else 0.0
    avg = sum(latencies) / n if n else 0.0
    max_lat = latencies[-1] if n else 0.0

    for t, data in tool_aggs.items():
        t_lats = sorted(data["latencies"])
        tn = len(t_lats)
        data["p50"] = t_lats[int(math.ceil(0.50 * tn)) - 1] if tn else 0.0

    for r_name, r_data in repo_aggs.items():
        calls = r_data["calls"]
        errs = r_data["errors"]
        r_data["reliability_pct"] = 100.0 * (calls - errs) / calls if calls else 100.0

    reliability = 100.0 * (total_calls - errors) / total_calls if total_calls else 100.0
    saved_dollars = saved_tokens * 0.000003

    recent = list(reversed(entries[-40:]))
    recent_errors = list(reversed(all_errors[-25:]))

    return {
        "total_calls": total_calls,
        "error_count": errors,
        "stale_reads": stale_reads,
        "reliability_pct": reliability,
        "p50_ms": p50,
        "p95_ms": p95,
        "avg_ms": avg,
        "max_ms": max_lat,
        "saved_tokens": saved_tokens,
        "saved_dollars": saved_dollars,
        "reuse_hits": reuse_hits,
        "tool_aggs": tool_aggs,
        "repo_aggs": repo_aggs,
        "recent_queries": recent,
        "recent_errors": recent_errors,
    }


def format_bytes(n: int) -> str:
    """Format bytes as KB or MB."""
    if n < 1024:
        return f"{n} B"
    elif n < 1024 * 1024:
        return f"{n / 1024:.1f} KB"
    else:
        return f"{n / (1024 * 1024):.2f} MB"


def render_svg_barchart(tool_aggs: Dict[str, Dict[str, Any]]) -> str:
    """Render a pure inline SVG bar chart for tool call frequency."""
    if not tool_aggs:
        return '<p class="empty-state">No tool call data available</p>'

    sorted_tools = sorted(tool_aggs.items(), key=lambda kv: kv[1]["calls"], reverse=True)
    max_calls = max((data["calls"] for _, data in sorted_tools), default=1)

    chart_height = max(180, len(sorted_tools) * 32 + 20)
    chart_width = 560
    bar_max_width = 340
    start_x = 120

    svg_lines = [
        f'<svg width="100%" height="{chart_height}" viewBox="0 0 {chart_width} {chart_height}" xmlns="http://www.w3.org/2000/svg" class="bar-chart">'
    ]

    for i, (tool, data) in enumerate(sorted_tools):
        y = 24 + i * 32
        calls = data["calls"]
        errors = data["errors"]
        bar_w = max(4, int((calls / max_calls) * bar_max_width))
        color = "#6366f1" if errors == 0 else "#ef4444"

        svg_lines.append(
            f'<text x="{start_x - 10}" y="{y + 14}" text-anchor="end" class="chart-label">{html.escape(tool)}</text>'
        )
        svg_lines.append(
            f'<rect x="{start_x}" y="{y}" width="{bar_w}" height="20" rx="4" fill="{color}" opacity="0.88"/>'
        )
        svg_lines.append(
            f'<text x="{start_x + bar_w + 8}" y="{y + 14}" class="chart-value">{calls} calls ({data.get("p50", 0.0):.1f}ms)</text>'
        )

    svg_lines.append("</svg>")
    return "\n".join(svg_lines)


def render_html_dashboard(
    metrics: Dict[str, Any],
    index_stats: Dict[str, Any],
    scope_name: str,
    is_global: bool = False,
) -> str:
    """Render single-file interactive HTML dashboard."""
    p50 = f"{metrics['p50_ms']:.1f}ms"
    p95 = f"{metrics['p95_ms']:.1f}ms"
    tokens_str = f"{metrics['saved_tokens']:,}"
    dollars_str = f"${metrics['saved_dollars']:.2f}"
    gen_time = time.strftime("%Y-%m-%d %H:%M:%S UTC", time.gmtime())

    barchart_svg = render_svg_barchart(metrics.get("tool_aggs", {}))
    repo_aggs = metrics.get("repo_aggs", {})
    repo_count = len(repo_aggs)

    # Repository Filter Options
    repo_options = ['<option value="ALL">All Repositories</option>']
    for r_name in sorted(repo_aggs.keys()):
        repo_options.append(f'<option value="{html.escape(r_name)}">{html.escape(r_name)}</option>')
    repo_options_html = "\n".join(repo_options)

    # Repositories Table Rows
    repo_rows = []
    sorted_repos = sorted(repo_aggs.items(), key=lambda kv: kv[1]["calls"], reverse=True)
    for r_name, r_data in sorted_repos:
        calls = r_data["calls"]
        errs = r_data["errors"]
        rel = r_data["reliability_pct"]
        wt = r_data["worktree"]
        last_ts = r_data["last_ts"]
        last_str = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(last_ts)) if last_ts else "-"
        wt_badge = '<span class="badge badge-wt">worktree</span>' if wt else '<span class="badge badge-repo">repo</span>'
        rel_class = "status-ok" if errs == 0 else "status-err"
        repo_rows.append(
            f'<tr data-repo="{html.escape(r_name)}">'
            f'<td><strong>{html.escape(r_name)}</strong> {wt_badge}</td>'
            f'<td>{calls}</td>'
            f'<td><span class="badge {rel_class}">{rel:.1f}% ({errs} err)</span></td>'
            f'<td class="text-muted">{last_str}</td>'
            f'</tr>'
        )
    repos_tbody = "\n".join(repo_rows) if repo_rows else "<tr><td colspan='4'>No repositories recorded</td></tr>"

    # Recent queries rows
    recent_rows = []
    for q in metrics.get("recent_queries", []):
        ts = q.get("ts", 0)
        time_str = time.strftime("%H:%M:%S", time.localtime(ts)) if ts else "-"
        tool = html.escape(str(q.get("tool", "-")))
        r_name = html.escape(str(q.get("repo_name") or "-"))
        shown = q.get("shown", q.get("result_count", 0))
        lat_us = q.get("latency_us", 0)
        lat_str = f"{lat_us / 1000.0:.1f}ms" if lat_us else "-"
        status = "ok" if q.get("exit_code", 0) == 0 else f"err {q.get('exit_code')}"
        status_class = "status-ok" if q.get("exit_code", 0) == 0 else "status-err"

        recent_rows.append(
            f'<tr class="query-row" data-repo="{r_name}">'
            f'<td>{time_str}</td>'
            f'<td><span class="repo-tag">{r_name}</span></td>'
            f'<td><code>{tool}</code></td>'
            f'<td>{shown}</td>'
            f'<td>{lat_str}</td>'
            f'<td><span class="badge {status_class}">{status}</span></td>'
            f'</tr>'
        )
    recent_tbody = "\n".join(recent_rows) if recent_rows else "<tr><td colspan='6'>No queries logged</td></tr>"

    # Recent errors rows
    error_rows = []
    for q in metrics.get("recent_errors", []):
        ts = q.get("ts", 0)
        time_str = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(ts)) if ts else "-"
        tool = html.escape(str(q.get("tool", "-")))
        r_name = html.escape(str(q.get("repo_name") or "-"))
        code = q.get("exit_code", 1)
        lat_us = q.get("latency_us", 0)
        lat_str = f"{lat_us / 1000.0:.1f}ms" if lat_us else "-"
        args_raw = json.dumps(q.get("args", {}), ensure_ascii=False)
        args_short = html.escape(args_raw[:100] + ("..." if len(args_raw) > 100 else ""))

        error_rows.append(
            f'<tr class="error-row" data-repo="{r_name}">'
            f'<td>{time_str}</td>'
            f'<td><span class="repo-tag">{r_name}</span></td>'
            f'<td><code>{tool}</code></td>'
            f'<td><span class="badge status-err">exit {code}</span></td>'
            f'<td>{lat_str}</td>'
            f'<td class="args-col" title="{html.escape(args_raw)}"><code>{args_short}</code></td>'
            f'</tr>'
        )
    errors_tbody = (
        "\n".join(error_rows)
        if error_rows
        else "<tr><td colspan='6' class='text-success' style='text-align: center; padding: 1.5rem;'>✓ Zero recorded errors — 100% clean operations</td></tr>"
    )

    header_title = "⚡ aw global telemetry dashboard" if is_global else f"⚡ aw telemetry dashboard — {html.escape(scope_name)}"
    sub_title = (
        f"Monitored across {repo_count} active repositories/worktrees &bull; Source: ~/.aw/usage.log"
        if is_global
        else f"Repository: {html.escape(scope_name)} &bull; Index: {index_stats.get('files', 0)} files, {index_stats.get('symbols', 0)} symbols"
    )

    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{header_title}</title>
<style>
  :root {{
    --bg: #0f172a;
    --surface: #1e293b;
    --surface-hover: #293548;
    --border: #334155;
    --text: #f8fafc;
    --text-muted: #94a3b8;
    --primary: #6366f1;
    --primary-light: #818cf8;
    --success: #10b981;
    --danger: #ef4444;
    --warning: #f59e0b;
    --accent: #38bdf8;
    --font: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    --font-mono: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace;
  }}
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{
    background: var(--bg);
    color: var(--text);
    font-family: var(--font);
    line-height: 1.5;
    padding: 2rem;
  }}
  .container {{ max-width: 1240px; margin: 0 auto; }}
  header {{
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    padding-bottom: 1.5rem;
    border-bottom: 1px solid var(--border);
    margin-bottom: 2rem;
    flex-wrap: wrap;
    gap: 1rem;
  }}
  h1 {{ font-size: 1.6rem; font-weight: 700; color: var(--text); letter-spacing: -0.02em; }}
  .header-meta {{ color: var(--text-muted); font-size: 0.9rem; margin-top: 0.25rem; }}
  
  .controls-bar {{
    display: flex;
    align-items: center;
    gap: 1rem;
    margin-bottom: 1.5rem;
    background: var(--surface);
    padding: 0.75rem 1.25rem;
    border-radius: 8px;
    border: 1px solid var(--border);
  }}
  .controls-bar label {{ font-size: 0.85rem; font-weight: 600; color: var(--text-muted); }}
  .select-filter {{
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    padding: 0.4rem 0.8rem;
    border-radius: 6px;
    font-family: var(--font-mono);
    font-size: 0.85rem;
    outline: none;
  }}
  .select-filter:focus {{ border-color: var(--primary); }}

  .hero-grid {{
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 1.25rem;
    margin-bottom: 2rem;
  }}
  .card {{
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 1.25rem;
    position: relative;
    overflow: hidden;
  }}
  .card-label {{
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-muted);
    margin-bottom: 0.5rem;
    font-weight: 600;
  }}
  .card-value {{
    font-size: 1.8rem;
    font-weight: 700;
    color: var(--text);
    font-family: var(--font-mono);
  }}
  .card-sub {{
    font-size: 0.8rem;
    color: var(--text-muted);
    margin-top: 0.25rem;
  }}
  .highlight {{ color: var(--accent); }}
  .success {{ color: var(--success); }}
  .danger {{ color: var(--danger); }}

  .section-grid {{
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1.5rem;
    margin-bottom: 2rem;
  }}
  @media (max-width: 900px) {{ .section-grid {{ grid-template-columns: 1fr; }} }}
  
  h2 {{ font-size: 1.1rem; font-weight: 600; margin-bottom: 1rem; display: flex; align-items: center; justify-content: space-between; }}
  .bar-chart {{ display: block; }}
  .chart-label {{ fill: var(--text-muted); font-family: var(--font-mono); font-size: 12px; }}
  .chart-value {{ fill: var(--text); font-family: var(--font-mono); font-size: 11px; }}

  table {{
    width: 100%;
    border-collapse: collapse;
    font-size: 0.85rem;
    font-family: var(--font-mono);
  }}
  th, td {{
    text-align: left;
    padding: 0.6rem 0.75rem;
    border-bottom: 1px solid var(--border);
  }}
  th {{ color: var(--text-muted); font-weight: 600; text-transform: uppercase; font-size: 0.7rem; }}
  tr:hover td {{ background: var(--surface-hover); }}
  
  .badge {{
    display: inline-block;
    padding: 0.15rem 0.45rem;
    border-radius: 4px;
    font-size: 0.7rem;
    font-weight: 600;
  }}
  .badge-repo {{ background: rgba(99, 102, 241, 0.15); color: var(--primary-light); }}
  .badge-wt {{ background: rgba(245, 158, 11, 0.15); color: var(--warning); }}
  .status-ok {{ background: rgba(16, 185, 129, 0.15); color: var(--success); }}
  .status-err {{ background: rgba(239, 68, 68, 0.15); color: var(--danger); }}
  .repo-tag {{ color: var(--primary-light); font-weight: 600; }}
  .text-muted {{ color: var(--text-muted); font-size: 0.8rem; }}
  .text-success {{ color: var(--success); font-weight: 600; }}
  .args-col {{ max-width: 280px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-muted); }}
  .full-width {{ margin-bottom: 2rem; }}
</style>
</head>
<body>
<div class="container">
  <header>
    <div>
      <h1>{header_title}</h1>
      <div class="header-meta">{sub_title}</div>
    </div>
    <div class="header-meta">Generated: {gen_time}</div>
  </header>

  <div class="controls-bar">
    <label for="repo-filter">Filter Scope:</label>
    <select id="repo-filter" class="select-filter" onchange="filterDashboard(this.value)">
      {repo_options_html}
    </select>
  </div>

  <div class="hero-grid">
    <div class="card">
      <div class="card-label">Total Queries</div>
      <div class="card-value highlight">{metrics["total_calls"]}</div>
      <div class="card-sub">{metrics["reliability_pct"]:.1f}% reliability ({metrics["error_count"]} errors)</div>
    </div>
    <div class="card">
      <div class="card-label">Median Latency (p50)</div>
      <div class="card-value success">{p50}</div>
      <div class="card-sub">p95 = {p95} &bull; avg = {metrics["avg_ms"]:.1f}ms</div>
    </div>
    <div class="card">
      <div class="card-label">Est. Tokens Saved</div>
      <div class="card-value">~{tokens_str}</div>
      <div class="card-sub">~{dollars_str} avoided vs whole-file reads</div>
    </div>
    <div class="card">
      <div class="card-label">Anti-Reinvention</div>
      <div class="card-value highlight">{metrics["reuse_hits"]}</div>
      <div class="card-sub">Duplication candidates caught</div>
    </div>
  </div>

  <div class="section-grid">
    <div class="card">
      <h2>Active Repositories & Worktrees</h2>
      <table>
        <thead>
          <tr>
            <th>Repository</th>
            <th>Queries</th>
            <th>Reliability</th>
            <th>Last Active</th>
          </tr>
        </thead>
        <tbody id="repos-tbody">
          {repos_tbody}
        </tbody>
      </table>
    </div>

    <div class="card">
      <h2>Tool Distribution</h2>
      {barchart_svg}
    </div>
  </div>

  <div class="card full-width">
    <h2>Recent Query Activity</h2>
    <table>
      <thead>
        <tr>
          <th>Time</th>
          <th>Repository</th>
          <th>Tool</th>
          <th>Items</th>
          <th>Latency</th>
          <th>Status</th>
        </tr>
      </thead>
      <tbody id="recent-tbody">
        {recent_tbody}
      </tbody>
    </table>
  </div>

  <div class="card full-width">
    <h2>Bug & Error Inspection (Triage Log)</h2>
    <table>
      <thead>
        <tr>
          <th>Time</th>
          <th>Repository</th>
          <th>Tool</th>
          <th>Exit Code</th>
          <th>Latency</th>
          <th>Arguments</th>
        </tr>
      </thead>
      <tbody id="errors-tbody">
        {errors_tbody}
      </tbody>
    </table>
  </div>
</div>

<script>
function filterDashboard(selectedRepo) {{
  const queryRows = document.querySelectorAll('#recent-tbody tr.query-row');
  queryRows.forEach(row => {{
    const repo = row.getAttribute('data-repo');
    row.style.display = (selectedRepo === 'ALL' || repo === selectedRepo) ? '' : 'none';
  }});

  const errorRows = document.querySelectorAll('#errors-tbody tr.error-row');
  errorRows.forEach(row => {{
    const repo = row.getAttribute('data-repo');
    row.style.display = (selectedRepo === 'ALL' || repo === selectedRepo) ? '' : 'none';
  }});

  const repoRows = document.querySelectorAll('#repos-tbody tr');
  repoRows.forEach(row => {{
    const repo = row.getAttribute('data-repo');
    row.style.display = (selectedRepo === 'ALL' || repo === selectedRepo) ? '' : 'none';
  }});
}}
</script>
</body>
</html>
"""


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate an interactive HTML telemetry dashboard for aw."
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=None,
        help="Repository root directory containing .aw/ (default: auto-detect)",
    )
    parser.add_argument(
        "--global",
        dest="is_global",
        action="store_true",
        help="Generate global dashboard across all repositories from ~/.aw/usage.log",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=None,
        help="Output HTML dashboard path (default: <root>/.aw/dashboard.html or ~/.aw/dashboard.html)",
    )
    parser.add_argument(
        "--open",
        action="store_true",
        help="Automatically open generated HTML dashboard in default web browser",
    )
    args = parser.parse_args()

    if args.is_global:
        aw_dir = resolve_global_aw_dir()
        out_path = args.out or (aw_dir / "dashboard.html")
        entries = parse_usage_log_from_dir(aw_dir)
        metrics = calculate_metrics(entries)
        index_stats = inspect_index_db(None)
        scope_name = "Global Multi-Repo"
        html_content = render_html_dashboard(metrics, index_stats, scope_name, is_global=True)
    else:
        root = (args.root or Path(".")).resolve()
        out_path = args.out or (root / ".aw" / "dashboard.html")
        entries = parse_usage_log(root)
        metrics = calculate_metrics(entries)
        index_stats = inspect_index_db(root)
        scope_name = root.name or "repository"
        html_content = render_html_dashboard(metrics, index_stats, scope_name, is_global=False)

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(html_content, encoding="utf-8")
    print(f"Rendered dashboard to {out_path} ({len(entries)} journaled queries)")

    if args.open:
        opener = "open" if sys.platform == "darwin" else "xdg-open"
        try:
            subprocess.run([opener, str(out_path)], check=False)
        except OSError:
            pass


if __name__ == "__main__":
    main()
