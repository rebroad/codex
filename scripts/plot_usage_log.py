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
      cursor: pointer;
      user-select: none;
      padding: 2px 6px;
      border-radius: 6px;
      border: 1px solid transparent;
    }}
    .legend-item:hover {{
      border-color: #d1d5db;
      background: #f9fafb;
    }}
    .legend-item.off {{
      opacity: 0.45;
      text-decoration: line-through;
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
    <div class="meta">Source: <code>{source_path}</code>. Y-axis: percentage values. X-axis anchored to evenly spaced <code>percent=A->B</code> events. Zoom (X+Y): left-click in plot area to zoom in 25%, right-click or shift+left-click to zoom out 25%. Pan when zoomed: left-drag in plot area, or use mouse wheel/trackpad scroll.</div>
    <canvas id="plot"></canvas>
    <div class="legend" id="legend"></div>
  </div>
  <script>
    const data = {json.dumps(payload)};
    const canvas = document.getElementById("plot");
    const legend = document.getElementById("legend");
    const ctx = canvas.getContext("2d");
    const margin = {{ left: 64, right: 16, top: 16, bottom: 52 }};

    const dpr = window.devicePixelRatio || 1;
    const rect = () => canvas.getBoundingClientRect();
    const allXs = [];
    const allYs = [];
    allXs.push(...data.points.map((p) => p.x));
    allXs.push(...data.anchors.map((a) => a.x));
    allXs.push(...data.baseline.map((b) => b.x));
    allYs.push(...data.baseline.map((b) => b.value));
    for (const point of data.points) {{
      for (const metric of data.metrics) {{
        if (Object.prototype.hasOwnProperty.call(point.metrics, metric)) {{
          allYs.push(point.metrics[metric]);
        }}
      }}
    }}
    if (!allXs.length) allXs.push(0, 1);
    if (!allYs.length) allYs.push(0, 100);

    const globalXMin = Math.min(...allXs);
    const globalXMax = Math.max(...allXs);
    const globalYMin = Math.min(0, ...allYs);
    const globalYMax = Math.max(100, ...allYs);
    const globalSpan = Math.max(globalXMax - globalXMin, 1e-9);
    const globalYSpan = Math.max(globalYMax - globalYMin, 1e-9);
    const minViewSpan = Math.max(globalSpan * 0.01, 1e-6);
    const minViewYSpan = Math.max(globalYSpan * 0.01, 1e-6);

    let viewXMin = globalXMin;
    let viewXMax = globalXMax;
    let viewYMin = globalYMin;
    let viewYMax = globalYMax;
    let lastPlotRect = null;
    let dragState = null;
    const seriesVisible = {{ baseline: true }};
    for (const metric of data.metrics) {{
      seriesVisible[metric] = true;
    }}

    function valueToX(v, xmin, xmax, left, width) {{
      if (xmax === xmin) return left + width / 2;
      return left + ((v - xmin) / (xmax - xmin)) * width;
    }}
    function valueToY(v, ymin, ymax, top, height) {{
      if (ymax === ymin) return top + height / 2;
      return top + (1 - ((v - ymin) / (ymax - ymin))) * height;
    }}

    function draw() {{
      const r = rect();
      canvas.width = Math.floor(r.width * dpr);
      canvas.height = Math.floor(r.height * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, r.width, r.height);

      const w = r.width - margin.left - margin.right;
      const h = r.height - margin.top - margin.bottom;
      lastPlotRect = {{ left: margin.left, top: margin.top, width: w, height: h }};

      const xmin = viewXMin;
      const xmax = viewXMax;
      const ymin = viewYMin;
      const ymax = viewYMax;

      ctx.strokeStyle = "#e5e7eb";
      ctx.fillStyle = "#6b7280";
      ctx.lineWidth = 1;
      ctx.font = "12px ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial, sans-serif";

      const ySpan = Math.max(ymax - ymin, 1e-9);
      let yStep = 10;
      if (ySpan > 0) {{
        const rawStep = ySpan / 8;
        const power = Math.pow(10, Math.floor(Math.log10(rawStep)));
        const scaled = rawStep / power;
        if (scaled <= 1) yStep = 1 * power;
        else if (scaled <= 2) yStep = 2 * power;
        else if (scaled <= 5) yStep = 5 * power;
        else yStep = 10 * power;
      }}
      const yStart = Math.ceil(ymin / yStep) * yStep;
      for (let y = yStart; y <= ymax + yStep * 0.5; y += yStep) {{
        const py = valueToY(y, ymin, ymax, margin.top, h);
        ctx.beginPath();
        ctx.moveTo(margin.left, py);
        ctx.lineTo(margin.left + w, py);
        ctx.stroke();
        const label = Number.isInteger(y) ? String(y) : y.toFixed(2).replace(/\\.?0+$/, "");
        ctx.fillText(label, 8, py + 4);
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

      if (seriesVisible.baseline && data.baseline.length) {{
        const baseline = [...data.baseline].sort((a, b) => a.x - b.x);
        ctx.strokeStyle = "#9ca3af";
        ctx.lineWidth = 2;
        ctx.beginPath();
        const first = baseline[0];
        let prevX = valueToX(first.x, xmin, xmax, margin.left, w);
        let prevY = valueToY(first.value, ymin, ymax, margin.top, h);
        ctx.moveTo(prevX, prevY);
        for (let i = 1; i < baseline.length; i += 1) {{
          const next = baseline[i];
          const nextX = valueToX(next.x, xmin, xmax, margin.left, w);
          const nextY = valueToY(next.value, ymin, ymax, margin.top, h);
          ctx.lineTo(nextX, prevY);
          ctx.lineTo(nextX, nextY);
          prevX = nextX;
          prevY = nextY;
        }}
        ctx.stroke();
      }}

      for (const metric of data.metrics) {{
        if (!seriesVisible[metric]) continue;
        const points = data.points.filter((p) => Object.prototype.hasOwnProperty.call(p.metrics, metric));
        if (!points.length) continue;
        ctx.strokeStyle = data.colors[metric];
        ctx.lineWidth = 2;
        ctx.beginPath();
        points.forEach((p, i) => {{
          const px = valueToX(p.x, xmin, xmax, margin.left, w);
          const py = valueToY(p.metrics[metric], ymin, ymax, margin.top, h);
          if (i === 0) ctx.moveTo(px, py);
          else ctx.lineTo(px, py);
        }});
        ctx.stroke();
      }}
    }}

    function setAxisView(anchor, anchorRatio, nextSpan, globalMin, globalMax, minSpan) {{
      const axisSpan = Math.max(globalMax - globalMin, 1e-9);
      const clampedSpan = Math.min(axisSpan, Math.max(minSpan, nextSpan));
      const safeRatio = Math.min(1, Math.max(0, anchorRatio));
      let nextMin = anchor - safeRatio * clampedSpan;
      let nextMax = nextMin + clampedSpan;
      if (nextMin < globalMin) {{
        nextMax += globalMin - nextMin;
        nextMin = globalMin;
      }}
      if (nextMax > globalMax) {{
        nextMin -= nextMax - globalMax;
        nextMax = globalMax;
      }}
      if (nextMin < globalMin) nextMin = globalMin;
      if (nextMax > globalMax) nextMax = globalMax;
      return [nextMin, nextMax];
    }}

    function setViewFromAnchor(anchorX, anchorRatioX, nextXSpan, anchorY, anchorRatioY, nextYSpan) {{
      const [nextXMin, nextXMax] = setAxisView(
        anchorX,
        anchorRatioX,
        nextXSpan,
        globalXMin,
        globalXMax,
        minViewSpan
      );
      const [nextYMin, nextYMax] = setAxisView(
        anchorY,
        anchorRatioY,
        nextYSpan,
        globalYMin,
        globalYMax,
        minViewYSpan
      );
      viewXMin = nextXMin;
      viewXMax = nextXMax;
      viewYMin = nextYMin;
      viewYMax = nextYMax;
    }}

    function setCenteredView(centerX, nextXSpan, centerY, nextYSpan) {{
      setViewFromAnchor(centerX, 0.5, nextXSpan, centerY, 0.5, nextYSpan);
    }}

    function shiftView(deltaX, deltaY) {{
      const xSpan = Math.max(viewXMax - viewXMin, minViewSpan);
      const ySpan = Math.max(viewYMax - viewYMin, minViewYSpan);
      const xCenter = (viewXMin + viewXMax) / 2 + deltaX;
      const yCenter = (viewYMin + viewYMax) / 2 + deltaY;
      setCenteredView(xCenter, xSpan, yCenter, ySpan);
    }}

    function zoomAtPixel(px, py, zoomFactor) {{
      if (!lastPlotRect || globalSpan <= 1e-9 || globalYSpan <= 1e-9) return;
      if (px < lastPlotRect.left || px > lastPlotRect.left + lastPlotRect.width) return;
      if (py < lastPlotRect.top || py > lastPlotRect.top + lastPlotRect.height) return;

      const currentXSpan = Math.max(viewXMax - viewXMin, minViewSpan);
      const currentYSpan = Math.max(viewYMax - viewYMin, minViewYSpan);
      const relativeX = (px - lastPlotRect.left) / Math.max(lastPlotRect.width, 1);
      const relativeY = (py - lastPlotRect.top) / Math.max(lastPlotRect.height, 1);
      const anchorX = viewXMin + relativeX * currentXSpan;
      const anchorY = viewYMin + (1 - relativeY) * currentYSpan;
      const nextXSpan = currentXSpan * zoomFactor;
      const nextYSpan = currentYSpan * zoomFactor;
      setViewFromAnchor(anchorX, relativeX, nextXSpan, anchorY, 1 - relativeY, nextYSpan);
      draw();
    }}

    canvas.addEventListener("contextmenu", (event) => {{
      event.preventDefault();
    }});

    canvas.addEventListener("mouseleave", () => {{
      dragState = null;
      canvas.style.cursor = "";
    }});

    canvas.addEventListener("mousedown", (event) => {{
      if (!lastPlotRect) return;
      const inPlot =
        event.offsetX >= lastPlotRect.left &&
        event.offsetX <= lastPlotRect.left + lastPlotRect.width &&
        event.offsetY >= lastPlotRect.top &&
        event.offsetY <= lastPlotRect.top + lastPlotRect.height;
      if (!inPlot) return;

      if (event.button === 2) {{
        zoomAtPixel(event.offsetX, event.offsetY, 1.25);
        return;
      }}
      if (event.button !== 0) return;

      dragState = {{
        startX: event.offsetX,
        startY: event.offsetY,
        lastX: event.offsetX,
        lastY: event.offsetY,
        moved: false,
        shiftKey: event.shiftKey,
      }};
      canvas.style.cursor = "grabbing";
    }});

    canvas.addEventListener("mousemove", (event) => {{
      if (!dragState || !lastPlotRect) return;
      const dxPx = event.offsetX - dragState.lastX;
      const dyPx = event.offsetY - dragState.lastY;
      dragState.lastX = event.offsetX;
      dragState.lastY = event.offsetY;
      const movedX = Math.abs(event.offsetX - dragState.startX);
      const movedY = Math.abs(event.offsetY - dragState.startY);
      if (movedX > 3 || movedY > 3) {{
        dragState.moved = true;
      }}
      if (!dragState.moved) return;

      const xSpan = Math.max(viewXMax - viewXMin, minViewSpan);
      const ySpan = Math.max(viewYMax - viewYMin, minViewYSpan);
      const deltaX = -(dxPx / Math.max(lastPlotRect.width, 1)) * xSpan;
      const deltaY = (dyPx / Math.max(lastPlotRect.height, 1)) * ySpan;
      shiftView(deltaX, deltaY);
      draw();
    }});

    addEventListener("mouseup", (event) => {{
      if (!dragState) return;
      const clickState = dragState;
      dragState = null;
      canvas.style.cursor = "";
      if (event.button !== 0) return;
      if (!clickState.moved) {{
        const factor = clickState.shiftKey ? 1.25 : 0.75;
        zoomAtPixel(clickState.startX, clickState.startY, factor);
      }}
    }});

    canvas.addEventListener("wheel", (event) => {{
      if (!lastPlotRect) return;
      const inPlot =
        event.offsetX >= lastPlotRect.left &&
        event.offsetX <= lastPlotRect.left + lastPlotRect.width &&
        event.offsetY >= lastPlotRect.top &&
        event.offsetY <= lastPlotRect.top + lastPlotRect.height;
      if (!inPlot) return;
      event.preventDefault();

      const xSpan = Math.max(viewXMax - viewXMin, minViewSpan);
      const ySpan = Math.max(viewYMax - viewYMin, minViewYSpan);
      const horizontalPixels = event.deltaX + (event.shiftKey ? event.deltaY : 0);
      const verticalPixels = event.shiftKey ? 0 : event.deltaY;
      const deltaX = (horizontalPixels / Math.max(lastPlotRect.width, 1)) * xSpan;
      const deltaY = -(verticalPixels / Math.max(lastPlotRect.height, 1)) * ySpan;
      shiftView(deltaX, deltaY);
      draw();
    }}, {{ passive: false }});

    canvas.addEventListener("mouseenter", () => {{
      if (!dragState) {{
        canvas.style.cursor = "grab";
      }}
    }});

    function buildLegend() {{
      legend.innerHTML = "";
      const baselineItem = document.createElement("span");
      baselineItem.className = "legend-item";
      if (!seriesVisible.baseline) baselineItem.classList.add("off");
      const baselineSwatch = document.createElement("span");
      baselineSwatch.className = "swatch";
      baselineSwatch.style.backgroundColor = "#9ca3af";
      const baselineLabel = document.createElement("span");
      baselineLabel.textContent = "baseline";
      baselineItem.append(baselineSwatch, baselineLabel);
      baselineItem.addEventListener("click", () => {{
        seriesVisible.baseline = !seriesVisible.baseline;
        buildLegend();
        draw();
      }});
      legend.appendChild(baselineItem);
      for (const metric of data.metrics) {{
        const item = document.createElement("span");
        item.className = "legend-item";
        if (!seriesVisible[metric]) item.classList.add("off");
        const swatch = document.createElement("span");
        swatch.className = "swatch";
        swatch.style.backgroundColor = data.colors[metric];
        const label = document.createElement("span");
        label.textContent = metric;
        item.append(swatch, label);
        item.addEventListener("click", () => {{
          seriesVisible[metric] = !seriesVisible[metric];
          buildLegend();
          draw();
        }});
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
    usage_log_dir = os.environ.get("CODEX_USAGE_LOG_DIR")
    if usage_log_dir:
        return Path(usage_log_dir).expanduser() / filename
    env_codex_home = os.environ.get("CODEX_HOME")
    candidates = [
        Path.home() / ".codex" / "log" / filename,
    ]
    if env_codex_home:
        candidates.append(Path(env_codex_home).expanduser() / "log" / filename)
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
        "positional_email",
        nargs="?",
        help="Account email (positional shorthand for --email).",
    )
    parser.add_argument(
        "--email",
        dest="option_email",
        help="Account email to load from usage-<email>.log in CODEX_USAGE_LOG_DIR or ~/.codex/log.",
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

    email = args.option_email or args.positional_email
    if args.input:
        input_path = Path(args.input).expanduser()
    elif email:
        input_path = resolve_usage_log_path(email)
    else:
        print("error: pass either --input, --email, or positional email", file=sys.stderr)
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
