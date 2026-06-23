import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { DispatchLog } from "../../lib/api";
import { latencyColor } from "./helpers";
import { ActivityTooltip, LegendSwatch, VIEW_HEIGHT } from "./ActivityTooltip";
import { Inbox } from "./icons";

export interface ActivityChartProps {
  logs: DispatchLog[];
}

const PAD_TOP = 16;
const PAD_BOTTOM = 28;
const PAD_LEFT = 40;
const PAD_RIGHT = 12;
const CHART_H = VIEW_HEIGHT - PAD_TOP - PAD_BOTTOM;

interface HoverState {
  index: number;
  x: number;
  y: number;
}

function formatLatency(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

function formatAxisTime(ts: string, now: Date): string {
  const t = new Date(ts).getTime();
  const diff = Math.max(0, now.getTime() - t);
  const min = Math.floor(diff / 60000);
  if (min < 1) return "now";
  if (min < 60) return `${min}m`;
  const hr = Math.floor(min / 60);
  return `${hr}h`;
}

export function ActivityChart({ logs }: ActivityChartProps) {
  const { t } = useTranslation();
  const [hover, setHover] = useState<HoverState | null>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const [viewWidth, setViewWidth] = useState(760);

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const w = entries[0].contentRect.width;
      if (w > 0) setViewWidth(w);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const chartW = viewWidth - PAD_LEFT - PAD_RIGHT;

  const { bars, maxLatency, now, successCount, failureCount } = useMemo(() => {
    const sorted = [...logs].sort(
      (a, b) => new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime()
    );
    const max = Math.max(...sorted.map((l) => l.latency_ms), 1, 1000);
    return {
      bars: sorted,
      maxLatency: max,
      now: new Date(),
      successCount: sorted.filter((l) => l.success).length,
      failureCount: sorted.filter((l) => !l.success).length,
    };
  }, [logs]);

  if (logs.length === 0) {
    return (
      <div className="dsh-card dsh-activity-empty">
        <span className="dsh-activity-empty-icon">
          <Inbox size={28} />
        </span>
        <div className="dsh-activity-empty-title">{t("dashboard.noActivity")}</div>
        <div className="dsh-activity-empty-desc">
          {t("dashboard.noActivityHint")}
        </div>
      </div>
    );
  }

  const count = bars.length;
  const slotW = chartW / count;
  const barWidth = Math.max(2, Math.min(slotW * 0.7, 12));

  const yTicks = [
    { value: 0, label: "0" },
    { value: maxLatency / 2, label: formatLatency(maxLatency / 2) },
    { value: maxLatency, label: formatLatency(maxLatency) },
  ];

  const xLabelCount = Math.min(4, count);
  const xLabels: { x: number; text: string }[] = [];
  for (let i = 0; i < xLabelCount; i++) {
    const idx = Math.floor((i / Math.max(1, xLabelCount - 1)) * (count - 1));
    if (bars[idx]) {
      const x = PAD_LEFT + idx * slotW + slotW / 2;
      xLabels.push({ x, text: formatAxisTime(bars[idx].timestamp, now) });
    }
  }

  return (
    <div className="dsh-card dsh-activity-card">
      <div className="dsh-activity-header">
        <div className="dsh-activity-title-group">
          <span className="dsh-activity-title">{t("dashboard.recentActivity")}</span>
          <span className="dsh-activity-meta">
            {t("dashboard.successRate", { rate: successCount, count })} &middot; {t("dashboard.failedCount", { count: failureCount })}
          </span>
        </div>
        <div className="dsh-activity-legend">
          <LegendSwatch color="var(--color-success)" label={t("logs.legendUnder500")} />
          <LegendSwatch color="var(--color-warning)" label={t("logs.legend500to2s")} />
          <LegendSwatch color="#f97316" label={t("logs.legend2to5s")} />
          <LegendSwatch color="var(--color-danger)" label={t("logs.legendOver5s")} />
          <LegendSwatch color="var(--color-danger)" label={t("common.failed")} shape="dot" />
        </div>
      </div>
      <div className="dsh-activity-chart-wrap" ref={wrapRef}>
        <svg
          className="dsh-activity-svg"
          viewBox={`0 0 ${viewWidth} ${VIEW_HEIGHT}`}
          role="img"
          aria-label="Request activity timeline"
        >
          {/* Y gridlines + labels */}
          {yTicks.map((tick) => {
            const y = PAD_TOP + CHART_H - (tick.value / maxLatency) * CHART_H;
            return (
              <g key={tick.value}>
                <line
                  className="dsh-activity-gridline"
                  x1={PAD_LEFT}
                  y1={y}
                  x2={viewWidth - PAD_RIGHT}
                  y2={y}
                />
                <text
                  className="dsh-activity-axis-label"
                  x={PAD_LEFT - 6}
                  y={y + 3}
                  textAnchor="end"
                >
                  {tick.label}
                </text>
              </g>
            );
          })}

          {/* Bars (success) */}
          {bars.map((log, i) => {
            const x = PAD_LEFT + i * slotW + (slotW - barWidth) / 2;
            const height = Math.max((log.latency_ms / maxLatency) * CHART_H, 2);
            const y = PAD_TOP + CHART_H - height;
            if (!log.success) return null;
            return (
              <rect
                key={log.id}
                x={x}
                y={y}
                width={barWidth}
                height={height}
                rx={1.5}
                fill={latencyColor(log.latency_ms)}
                className="dsh-activity-bar"
                onMouseEnter={() => setHover({ index: i, x: x + barWidth / 2, y })}
                onMouseLeave={() => setHover(null)}
              />
            );
          })}

          {/* Failure markers */}
          {bars.map((log, i) => {
            if (log.success) return null;
            const x = PAD_LEFT + i * slotW + slotW / 2;
            const yTop = PAD_TOP + 6;
            return (
              <g key={log.id}>
                <line
                  x1={x}
                  x2={x}
                  y1={PAD_TOP + CHART_H}
                  y2={PAD_TOP + CHART_H - 4}
                  stroke="var(--color-danger)"
                  strokeWidth={1.5}
                  opacity={0.5}
                />
                <circle
                  cx={x}
                  cy={yTop}
                  r={3.5}
                  fill="var(--color-danger)"
                  className="dsh-activity-fail-dot"
                />
                {/* Invisible hover target */}
                <rect
                  key={`h-${log.id}`}
                  x={x - slotW / 2}
                  y={0}
                  width={slotW}
                  height={VIEW_HEIGHT}
                  fill="transparent"
                  onMouseEnter={() => setHover({ index: i, x, y: yTop })}
                  onMouseLeave={() => setHover(null)}
                />
              </g>
            );
          })}

          {/* X-axis labels */}
          {xLabels.map((lbl, i) => (
            <text
              key={i}
              className="dsh-activity-axis-label"
              x={lbl.x}
              y={VIEW_HEIGHT - 8}
              textAnchor="middle"
            >
              {lbl.text}
            </text>
          ))}
        </svg>

        {hover && bars[hover.index] && (
          <ActivityTooltip log={bars[hover.index]} x={hover.x} y={hover.y} viewWidth={viewWidth} />
        )}
      </div>
    </div>
  );
}
