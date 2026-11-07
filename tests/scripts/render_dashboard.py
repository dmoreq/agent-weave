#!/usr/bin/env python3
"""Interactive Telemetry & Index Dashboard Generator for `aw` (`agent-weave`).

Self-contained, zero-dependency script reading `.aw/usage.log` and `.aw/index.db`
and producing a responsive, single-file HTML dashboard with hero metrics,
SVG tool distribution, latency stats, and anti-reinvention effectiveness.
"""

from __future__ import annotations

import argparse
import html
import json
import math
import os
from pathlib import Path
import sqlite3
import sys
import time
from typing import Any, Dict, List, Optional, Tuple


def parse_usage_log(root: Path) -> List[Dict[str, Any]]:
    """Load journal entries from .aw/usage.log and .aw/usage.log.1."""
    entries: List[Dict[str, Any]] = []
    aw_dir = root / ".aw"
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


def inspect_index_db(root: Path) -> Dict[str, Any]:
    """Inspect .aw/index.db for indexed file and symbol counts."""
    db_path = root / ".aw" / "index.db"
    stats: Dict[str, Any] = {
        "exists": False,
        "size_bytes": 0,
        "files": 0,
        "symbols": 0,
        "refs": 0,
        "languages": {},
    }
    if not db_path.is_file():
        return stats

    stats["exists"] = True
    stats["size_bytes"] = db_path.stat().st_size

    try:
        conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
        cur = conn.cursor()
        cur.execute("SELECT COUNT(*) FROM files")
        stats["files"] = cur.fetchone()[0]

        cur.execute("SELECT COUNT(*) FROM symbols")
        stats["symbols"] = cur.fetchone()[0]

        # Check edges count from edges table
        cur.execute("SELECT name FROM sqlite_master WHERE type='table' AND name='edges'")
        if cur.fetchone():
            cur.execute("SELECT COUNT(*) FROM edges")
            stats["edges"] = cur.fetchone()[0]

        # Language breakdown from files table
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
    """Calculate aggregated telemetry metrics."""
    total_calls = len(entries)
    if total_calls == 0:
        return {
            "total_calls": 0,
            "error_count": 0,
            "reliability_pct": 100.0,
            "p50_ms": 0.0,
            "p95_ms": 0.0,
            "avg_ms": 0.0,
            "max_ms": 0.0,
            "saved_tokens": 0,
            "saved_dollars": 0.0,
            "reuse_hits": 0,
            "tool_aggs": {},
            "recent_queries": [],
        }

    errors = 0
    stale_reads = 0
    reuse_hits = 0
    saved_tokens = 0
    latencies: List[float] = []
    tool_aggs: Dict[str, Dict[str, Any]] = {}

    for e in entries:
        tool = e.get("tool", "unknown")
        exit_code = e.get("exit_code", 0)
        shown = e.get("shown", e.get("result_count", 0))
        result_count = e.get("result_count", 0)
        stale = bool(e.get("stale", False))
        lat_us = e.get("latency_us")

        if tool not in tool_aggs:
            tool_aggs[tool] = {
                "calls": 0,
                "errors": 0,
                "shown": 0,
                "latencies": [],
            }

        tool_aggs[tool]["calls"] += 1
        tool_aggs[tool]["shown"] += shown

        if exit_code != 0:
            errors += 1
            tool_aggs[tool]["errors"] += 1

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

    reliability = 100.0 * (total_calls - errors) / total_calls if total_calls else 100.0
    saved_dollars = saved_tokens * 0.000003

    # Most recent 25 queries
    recent = list(reversed(entries[-25:]))

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
        "recent_queries": recent,
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
        color = "#4f46e5" if errors == 0 else "#e11d48"

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
    metrics: Dict[str, Any], index_stats: Dict[str, Any], repo_name: str
) -> str:
    """Render single-file HTML document."""
    p50 = f"{metrics['p50_ms']:.1f}ms"
    p95 = f"{metrics['p95_ms']:.1f}ms"
    tokens_str = f"{metrics['saved_tokens']:,}"
    dollars_str = f"${metrics['saved_dollars']:.2f}"
    gen_time = time.strftime("%Y-%m-%d %H:%M:%S UTC", time.gmtime())
    db_size = format_bytes(index_stats.get("size_bytes", 0))

    barchart_svg = render_svg_barchart(metrics.get("tool_aggs", {}))

    # Recent queries rows
    recent_rows = []
    for q in metrics.get("recent_queries", []):
        ts = q.get("ts", 0)
        time_str = time.strftime("%H:%M:%S", time.localtime(ts)) if ts else "-"
        tool = html.escape(str(q.get("tool", "-")))
        shown = q.get("shown", q.get("result_count", 0))
        lat_us = q.get("latency_us", 0)
        lat_str = f"{lat_us / 1000.0:.1f}ms" if lat_us else "-"
        status = "ok" if q.get("exit_code", 0) == 0 else f"err {q.get('exit_code')}"
        status_class = "status-ok" if q.get("exit_code", 0) == 0 else "status-err"

        recent_rows.append(
            f"<tr><td>{time_str}</td><td><code>{tool}</code></td><td>{shown}</td><td>{lat_str}</td><td><span class='badge {status_class}'>{status}</span></td></tr>"
        )
    recent_tbody = "\n".join(recent_rows) if recent_rows else "<tr><td colspan='5'>No queries logged</td></tr>"

    sorted_langs = sorted(index_stats.get("languages", {}).items(), key=lambda kv: -kv[1])
    lang_str = ", ".join(f"{cnt} {ext}" for ext, cnt in sorted_langs)
    lang_meta = f" ({lang_str})" if lang_str else ""
    edges_count = index_stats.get("edges", 0)
    edges_meta = f", {edges_count} edges" if edges_count else ""

    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>aw telemetry — {html.escape(repo_name)}</title>
<style>
  :root {{
    --bg: #0f172a;
    --surface: #1e293b;
    --border: #334155;
    --text: #f8fafc;
    --text-muted: #94a3b8;
    --primary: #6366f1;
    --success: #10b981;
    --danger: #ef4444;
    --accent: #38bdf8;
    --font: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    --font-mono: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace;
  }}
  @media (prefers-color-scheme: light) {{
    :root {{
      --bg: #f8fafc;
      --surface: #ffffff;
      --border: #e2e8f0;
      --text: #0f172a;
      --text-muted: #64748b;
      --primary: #4f46e5;
      --success: #059669;
      --danger: #dc2626;
      --accent: #0284c7;
    }}
  }}
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{
    font-family: var(--font);
    background: var(--bg);
    color: var(--text);
    padding: 2rem 1.5rem;
    line-height: 1.5;
  }}
  .container {{ max-width: 1080px; margin: 0 auto; }}
  header {{
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 2rem;
    padding-bottom: 1rem;
    border-bottom: 1px solid var(--border);
  }}
  h1 {{ font-size: 1.5rem; font-weight: 700; letter-spacing: -0.02em; }}
  .header-meta {{ font-size: 0.85rem; color: var(--text-muted); font-family: var(--font-mono); }}
  
  .hero-grid {{
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(210px, 1fr));
    gap: 1rem;
    margin-bottom: 2rem;
  }}
  .card {{
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 1.25rem;
  }}
  .card-label {{ font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.05em; color: var(--text-muted); margin-bottom: 0.5rem; }}
  .card-value {{ font-size: 1.75rem; font-weight: 700; font-family: var(--font-mono); color: var(--text); }}
  .card-sub {{ font-size: 0.8rem; color: var(--text-muted); margin-top: 0.25rem; }}
  .highlight {{ color: var(--primary); }}
  .success {{ color: var(--success); }}

  .section-grid {{
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1.5rem;
    margin-bottom: 2rem;
  }}
  @media (max-width: 768px) {{ .section-grid {{ grid-template-columns: 1fr; }} }}
  
  h2 {{ font-size: 1.1rem; font-weight: 600; margin-bottom: 1rem; }}
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
  .badge {{
    display: inline-block;
    padding: 0.15rem 0.4rem;
    border-radius: 4px;
    font-size: 0.7rem;
    font-weight: 600;
  }}
  .status-ok {{ background: rgba(16, 185, 129, 0.15); color: var(--success); }}
  .status-err {{ background: rgba(239, 68, 68, 0.15); color: var(--danger); }}
  .empty-state {{ color: var(--text-muted); font-style: italic; padding: 1rem 0; }}
</style>
</head>
<body>
<div class="container">
  <header>
    <div>
      <h1>⚡ aw telemetry dashboard</h1>
      <div class="header-meta">Repository: {html.escape(repo_name)} &bull; Index: {index_stats.get("files", 0)} files{lang_meta}, {index_stats.get("symbols", 0)} symbols{edges_meta} ({db_size})</div>
    </div>
    <div class="header-meta">Generated: {gen_time}</div>
  </header>

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
      <h2>Tool Distribution</h2>
      {barchart_svg}
    </div>

    <div class="card">
      <h2>Recent Query Activity</h2>
      <table>
        <thead>
          <tr>
            <th>Time</th>
            <th>Tool</th>
            <th>Items</th>
            <th>Latency</th>
            <th>Status</th>
          </tr>
        </thead>
        <tbody>
          {recent_tbody}
        </tbody>
      </table>
    </div>
  </div>
</div>
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
        default=Path("."),
        help="Repository root directory containing .aw/ (default: current directory)",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=None,
        help="Output HTML dashboard path (default: <root>/.aw/dashboard.html)",
    )
    args = parser.parse_args()

    root = args.root.resolve()
    out_path = args.out or (root / ".aw" / "dashboard.html")

    entries = parse_usage_log(root)
    metrics = calculate_metrics(entries)
    index_stats = inspect_index_db(root)
    repo_name = root.name or "repository"

    html_content = render_html_dashboard(metrics, index_stats, repo_name)

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(html_content, encoding="utf-8")
    print(f"Rendered dashboard to {out_path} ({len(entries)} journaled queries)")


if __name__ == "__main__":
    main()
