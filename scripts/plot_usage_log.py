#!/usr/bin/env python3
"""Plot Codex per-account usage log usage_pct metrics as a browser-viewable graph.

X-axis behavior:
- `percent=A->B` lines are treated as evenly spaced anchor points.
- `usage_pct[...]` points between two anchors are placed proportionally in time.
"""

from __future__ import annotations

import argparse
import bisect
import json
import os
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


TIMESTAMP_RE = re.compile(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z)\b")
PERCENT_RE = re.compile(r"\bpercent=(\d+)(?:->(\d+))?\b")
USAGE_RE = re.compile(r"usage_pct\[([^\]]+)\]=([0-9.+\-eE/]+)%")

PALETTE = [
    "#1f77b4",
    "#d62728",
    "#2ca02c",
    "#ff7f0e",
    "#9467bd",
    "#17becf",
    "#8c564b",
    "#e377c2",
    "#7f7f7f",
    "#bcbd22",
]


@dataclass
class BaselineEvent:
    ts: datetime
    label: str


@dataclass
class PercentEvent:
    ts: datetime
    value: float


@dataclass
class UsageEvent:
    ts: datetime
    metrics: dict[str, float]


def parse_utc(ts: str) -> datetime:
    return datetime.strptime(ts, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)


def parse_usage_line(
    line: str,
) -> tuple[datetime | None, BaselineEvent | None, PercentEvent | None, UsageEvent | None]:
    ts_match = TIMESTAMP_RE.search(line)
    ts = parse_utc(ts_match.group(1)) if ts_match else None

    baseline = None
    percent_event = None
    if ts is not None:
        percent_match = PERCENT_RE.search(line)
        if percent_match:
            percent_value = float(percent_match.group(2) or percent_match.group(1))
            percent_event = PercentEvent(ts=ts, value=percent_value)
        if percent_match and percent_match.group(2) is not None:
            baseline = BaselineEvent(
                ts=ts,
                label=percent_match.group(2),
            )

    usage = None
    if ts is not None:
        usage_match = USAGE_RE.search(line)
        if usage_match:
            metric_keys = usage_match.group(1).split("/")
            metric_values = usage_match.group(2).split("/")
            if len(metric_keys) == len(metric_values):
                metrics: dict[str, float] = {}
                for key, value in zip(metric_keys, metric_values):
                    key = key.strip()
                    try:
                        metrics[key] = float(value)
                    except ValueError:
                        continue
                if metrics:
                    usage = UsageEvent(ts=ts, metrics=metrics)
    return ts, baseline, percent_event, usage


def load_events(
    log_path: Path,
) -> tuple[list[BaselineEvent], list[PercentEvent], list[UsageEvent], list[str]]:
    baselines: list[BaselineEvent] = []
    percent_events: list[PercentEvent] = []
    usage_events: list[UsageEvent] = []
    metric_order: list[str] = []
    metric_seen: set[str] = set()

    with log_path.open("r", encoding="utf-8", errors="replace") as handle:
        for raw_line in handle:
            line = raw_line.strip()
            if not line:
                continue
            _, baseline, percent_event, usage = parse_usage_line(line)
            if baseline is not None:
                baselines.append(baseline)
            if percent_event is not None:
                percent_events.append(percent_event)
            if usage is not None:
                usage_events.append(usage)
                for key in usage.metrics:
                    if key not in metric_seen:
                        metric_seen.add(key)
                        metric_order.append(key)
    baselines.sort(key=lambda event: event.ts)
    percent_events.sort(key=lambda event: event.ts)
    usage_events.sort(key=lambda event: event.ts)
    return baselines, percent_events, usage_events, metric_order


def map_ts_to_anchor_x(ts_value: float, anchor_times: list[float]) -> float:
    first_delta = max(anchor_times[1] - anchor_times[0], 1.0)
    last_delta = max(anchor_times[-1] - anchor_times[-2], 1.0)
    idx = bisect.bisect_right(anchor_times, ts_value) - 1
    if idx < 0:
        return (ts_value - anchor_times[0]) / first_delta
    if idx >= len(anchor_times) - 1:
        return (len(anchor_times) - 1) + (ts_value - anchor_times[-1]) / last_delta
    start_t = anchor_times[idx]
    end_t = anchor_times[idx + 1]
    duration = max(end_t - start_t, 1.0)
    return idx + (ts_value - start_t) / duration


def compute_x_from_baselines(
    usage_events: list[UsageEvent], baselines: list[BaselineEvent], percent_events: list[PercentEvent]
) -> tuple[list[dict[str, Any]], list[dict[str, Any]], list[dict[str, Any]]]:
    anchor_times = [event.ts.timestamp() for event in baselines]
    anchor_labels = [event.label for event in baselines]
    anchor_data = [{"x": float(index), "label": label} for index, label in enumerate(anchor_labels)]

    if not usage_events and not percent_events:
        return [], anchor_data, []

    if len(anchor_times) < 2:
        first_candidates = [event.ts.timestamp() for event in usage_events]
        first_candidates.extend(event.ts.timestamp() for event in percent_events)
        first_ts = min(first_candidates) if first_candidates else 0.0
        usage_points = [
            {
                "x": (event.ts.timestamp() - first_ts) / 60.0,
                "ts": event.ts.isoformat(),
                "metrics": event.metrics,
            }
            for event in usage_events
        ]
        baseline_points = [
            {"x": (event.ts.timestamp() - first_ts) / 60.0, "value": event.value}
            for event in percent_events
        ]
        return usage_points, anchor_data, baseline_points

    points: list[dict[str, Any]] = []
    for event in usage_events:
        x = map_ts_to_anchor_x(event.ts.timestamp(), anchor_times)
        points.append({"x": x, "ts": event.ts.isoformat(), "metrics": event.metrics})
    baseline_points = [
        {"x": map_ts_to_anchor_x(event.ts.timestamp(), anchor_times), "value": event.value}
        for event in percent_events
    ]
    return points, anchor_data, baseline_points


def build_html(
    points: list[dict[str, Any]],
    anchors: list[dict[str, Any]],
    baseline_points: list[dict[str, Any]],
    metric_order: list[str],
    source_path: Path,
) -> str:
    colors = {metric: PALETTE[index % len(PALETTE)] for index, metric in enumerate(metric_order)}

    payload = {
        "points": points,
        "anchors": anchors,
        "baseline": baseline_points,
        "metrics": metric_order,
        "colors": colors,
        "source": str(source_path),
    }

    return f"""<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Codex Usage Graph</title>
  <style>
    :root {{
      --bg: #ffffff;
      --fg: #111111;
      --grid: #e5e7eb;
      --muted: #6b7280;
    }}
    html, body {{
      margin: 0;
      width: 100%;
      height: 100%;
    }}
    body {{
      font-family: ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial, sans-serif;
      color: var(--fg);
      background: var(--bg);
    }}
    .wrap {{
      box-sizing: border-box;
      width: 100vw;
      height: 100vh;
      padding: 12px;
      display: flex;
      flex-direction: column;
    }}
    h2 {{
      margin: 0 0 8px;
    }}
    .meta {{
      margin-bottom: 8px;
      color: var(--muted);
      font-size: 13px;
    }}
    canvas {{
      width: 100% !important;
      height: 100%;
      min-height: 0;
      border: 1px solid #d1d5db;
      border-radius: 8px;
      background: #fff;
      display: block;
      flex: 1 1 auto;
    }}
    .legend {{
      display: flex;
      gap: 12px;
      flex-wrap: wrap;
      margin: 10px 0 0;
      font-size: 13px;
    }}
    .legend-item {{
      display: inline-flex;
      align-items: center;
      gap: 6px;
      color: #111827;
    }}
    .swatch {{
      width: 14px;
      height: 3px;
      border-radius: 2px;
      display: inline-block;
    }}
  </style>
</head>
<body>
  <div class="wrap">
    <h2>Usage Percentages</h2>
    <div class="meta">Source: <code>{source_path}</code>. Y-axis: 0-100%. X-axis anchored to evenly spaced <code>percent=A->B</code> events.</div>
    <canvas id="plot"></canvas>
    <div class="legend" id="legend"></div>
  </div>
  <script>
    const data = {json.dumps(payload)};
    const canvas = document.getElementById("plot");
    const legend = document.getElementById("legend");
    const ctx = canvas.getContext("2d");

    const dpr = window.devicePixelRatio || 1;
    const rect = () => canvas.getBoundingClientRect();

    function valueToX(v, xmin, xmax, left, width) {{
      if (xmax === xmin) return left + width / 2;
      return left + ((v - xmin) / (xmax - xmin)) * width;
    }}
    function valueToY(v, top, height) {{
      return top + (1 - (v / 100)) * height;
    }}

    function draw() {{
      const r = rect();
      canvas.width = Math.floor(r.width * dpr);
      canvas.height = Math.floor(r.height * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, r.width, r.height);

      const margin = {{ left: 64, right: 16, top: 16, bottom: 52 }};
      const w = r.width - margin.left - margin.right;
      const h = r.height - margin.top - margin.bottom;

      const xs = data.points.map((p) => p.x);
      if (data.anchors.length) xs.push(...data.anchors.map((a) => a.x));
      if (data.baseline.length) xs.push(...data.baseline.map((b) => b.x));
      const xmin = xs.length ? Math.min(...xs) : 0;
      const xmax = xs.length ? Math.max(...xs) : 1;

      ctx.strokeStyle = "#e5e7eb";
      ctx.fillStyle = "#6b7280";
      ctx.lineWidth = 1;
      ctx.font = "12px ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial, sans-serif";

      for (let y = 0; y <= 100; y += 10) {{
        const py = valueToY(y, margin.top, h);
        ctx.beginPath();
        ctx.moveTo(margin.left, py);
        ctx.lineTo(margin.left + w, py);
        ctx.stroke();
        ctx.fillText(String(y), 8, py + 4);
      }}

      ctx.strokeStyle = "#f3f4f6";
      for (const anchor of data.anchors) {{
        const px = valueToX(anchor.x, xmin, xmax, margin.left, w);
        ctx.beginPath();
        ctx.moveTo(px, margin.top);
        ctx.lineTo(px, margin.top + h);
        ctx.stroke();
      }}

      ctx.strokeStyle = "#111827";
      ctx.beginPath();
      ctx.moveTo(margin.left, margin.top + h);
      ctx.lineTo(margin.left + w, margin.top + h);
      ctx.stroke();

      ctx.fillStyle = "#374151";
      for (const anchor of data.anchors) {{
        const px = valueToX(anchor.x, xmin, xmax, margin.left, w);
        ctx.fillText(anchor.label, px - 16, margin.top + h + 16);
      }}

      if (data.baseline.length) {{
        const baseline = [...data.baseline].sort((a, b) => a.x - b.x);
        ctx.strokeStyle = "#9ca3af";
        ctx.lineWidth = 2;
        ctx.beginPath();
        const first = baseline[0];
        let prevX = valueToX(first.x, xmin, xmax, margin.left, w);
        let prevY = valueToY(first.value, margin.top, h);
        ctx.moveTo(prevX, prevY);
        for (let i = 1; i < baseline.length; i += 1) {{
          const next = baseline[i];
          const nextX = valueToX(next.x, xmin, xmax, margin.left, w);
          const nextY = valueToY(next.value, margin.top, h);
          ctx.lineTo(nextX, prevY);
          ctx.lineTo(nextX, nextY);
          prevX = nextX;
          prevY = nextY;
        }}
        ctx.stroke();
      }}

      for (const metric of data.metrics) {{
        const points = data.points.filter((p) => Object.prototype.hasOwnProperty.call(p.metrics, metric));
        if (!points.length) continue;
        ctx.strokeStyle = data.colors[metric];
        ctx.lineWidth = 2;
        ctx.beginPath();
        points.forEach((p, i) => {{
          const px = valueToX(p.x, xmin, xmax, margin.left, w);
          const py = valueToY(p.metrics[metric], margin.top, h);
          if (i === 0) ctx.moveTo(px, py);
          else ctx.lineTo(px, py);
        }});
        ctx.stroke();
      }}
    }}

    function buildLegend() {{
      legend.innerHTML = "";
      const baselineItem = document.createElement("span");
      baselineItem.className = "legend-item";
      const baselineSwatch = document.createElement("span");
      baselineSwatch.className = "swatch";
      baselineSwatch.style.backgroundColor = "#9ca3af";
      const baselineLabel = document.createElement("span");
      baselineLabel.textContent = "baseline";
      baselineItem.append(baselineSwatch, baselineLabel);
      legend.appendChild(baselineItem);
      for (const metric of data.metrics) {{
        const item = document.createElement("span");
        item.className = "legend-item";
        const swatch = document.createElement("span");
        swatch.className = "swatch";
        swatch.style.backgroundColor = data.colors[metric];
        const label = document.createElement("span");
        label.textContent = metric;
        item.append(swatch, label);
        legend.appendChild(item);
      }}
    }}

    buildLegend();
    draw();
    addEventListener("resize", draw);
  </script>
</body>
</html>
"""


def open_with_viewimg(output_path: Path) -> None:
    viewimg = shutil.which("viewimg")
    if not viewimg:
        print("warning: `viewimg` not found in PATH; skipping auto-open", file=sys.stderr)
        return
    subprocess.run([viewimg, str(output_path)], check=False)


def resolve_usage_log_path(email: str) -> Path:
    filename = f"usage-{email}.log"
    env_codex_home = os.environ.get("CODEX_HOME")
    codex_home = (
        Path(env_codex_home).expanduser()
        if env_codex_home
        else Path.home() / ".codex"
    )
    candidates = [
        codex_home / "log" / filename,
        Path.home() / ".codex" / "log" / filename,
    ]
    for candidate in candidates:
        if candidate.exists():
            return candidate
    return candidates[0]


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Plot usage_pct metrics from a Codex per-account usage log. "
            "Baseline `percent=A->B` events are evenly spaced on X."
        )
    )
    parser.add_argument(
        "--email",
        help="Account email to load from usage-<email>.log in CODEX_HOME/log or ~/.codex/log.",
    )
    parser.add_argument(
        "--input",
        help="Path to usage log. Overrides --email-based lookup.",
    )
    parser.add_argument(
        "--output",
        default="/var/tmp/codex_usage_graph.html",
        help="Output HTML file path (default: /var/tmp/codex_usage_graph.html).",
    )
    parser.add_argument(
        "--no-open",
        action="store_true",
        help="Do not open output in browser via viewimg.",
    )
    args = parser.parse_args()

    if args.input:
        input_path = Path(args.input).expanduser()
    elif args.email:
        input_path = resolve_usage_log_path(args.email)
    else:
        print("error: pass either --input or --email", file=sys.stderr)
        return 1
    output_path = Path(args.output).expanduser()

    if not input_path.exists():
        print(f"error: input file not found: {input_path}", file=sys.stderr)
        return 1

    baselines, percent_events, usage_events, metric_order = load_events(input_path)
    if not usage_events:
        print(f"error: no usage_pct data found in {input_path}", file=sys.stderr)
        return 1

    points, anchors, baseline_points = compute_x_from_baselines(usage_events, baselines, percent_events)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(
        build_html(points, anchors, baseline_points, metric_order, input_path),
        encoding="utf-8",
    )
    print(f"wrote {output_path}")
    if not args.no_open:
        open_with_viewimg(output_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
