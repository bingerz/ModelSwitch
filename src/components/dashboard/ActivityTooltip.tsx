import { useTranslation } from "react-i18next";
import type { DispatchLog } from "../../lib/api";
import { formatCost, formatNumber, latencyColor } from "./helpers";

export const VIEW_WIDTH = 760;
export const VIEW_HEIGHT = 200;

function formatLatency(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

/** Hover tooltip rendered as an absolutely-positioned div above the SVG. */
export function ActivityTooltip({
  log,
  x,
  y,
  viewWidth = VIEW_WIDTH,
}: {
  log: DispatchLog;
  x: number;
  y: number;
  viewWidth?: number;
}) {
  const { t } = useTranslation();
  const tokens = (log.input_tokens ?? 0) + (log.output_tokens ?? 0);
  const time = new Date(log.timestamp).toLocaleTimeString();
  // Convert SVG coords to percentage for responsive positioning.
  const leftPct = (x / viewWidth) * 100;
  const topPct = (y / VIEW_HEIGHT) * 100;

  return (
    <div
      className="dsh-activity-tooltip"
      style={{
        left: `${leftPct}%`,
        top: `${topPct}%`,
        transform: "translate(-50%, calc(-100% - 10px))",
      }}
    >
      <div className="dsh-activity-tooltip-row dsh-activity-tooltip-title">
        {log.channel_name}
      </div>
      <div className="dsh-activity-tooltip-row">
        <span className="dsh-activity-tooltip-k">{t("logs.tooltipModel")}</span>
        <span className="dsh-activity-tooltip-v">{log.request_model}</span>
      </div>
      <div className="dsh-activity-tooltip-row">
        <span className="dsh-activity-tooltip-k">{t("logs.tooltipLatency")}</span>
        <span
          className="dsh-activity-tooltip-v"
          style={{ color: latencyColor(log.latency_ms) }}
        >
          {formatLatency(log.latency_ms)}
        </span>
      </div>
      <div className="dsh-activity-tooltip-row">
        <span className="dsh-activity-tooltip-k">{t("logs.tooltipStatus")}</span>
        <span
          className="dsh-activity-tooltip-v"
          style={{
            color: log.success ? "var(--color-success)" : "var(--color-danger)",
          }}
        >
          {log.success ? t("common.success") : t("common.failed")}
        </span>
      </div>
      <div className="dsh-activity-tooltip-row">
        <span className="dsh-activity-tooltip-k">{t("logs.tooltipRetries")}</span>
        <span className="dsh-activity-tooltip-v">{log.retry_count}</span>
      </div>
      <div className="dsh-activity-tooltip-row">
        <span className="dsh-activity-tooltip-k">{t("logs.tooltipTokens")}</span>
        <span className="dsh-activity-tooltip-v">{formatNumber(tokens)}</span>
      </div>
      <div className="dsh-activity-tooltip-row">
        <span className="dsh-activity-tooltip-k">{t("logs.tooltipCost")}</span>
        <span className="dsh-activity-tooltip-v">
          {formatCost(log.estimated_cost ?? 0)}
        </span>
      </div>
      <div className="dsh-activity-tooltip-row dsh-activity-tooltip-time">
        {time}
      </div>
    </div>
  );
}

export function LegendSwatch({
  color,
  label,
  shape = "bar",
}: {
  color: string;
  label: string;
  shape?: "bar" | "dot";
}) {
  return (
    <span className="dsh-activity-legend-item">
      <span
        className={`dsh-activity-legend-swatch ${
          shape === "dot" ? "dsh-activity-legend-dot" : ""
        }`}
        style={{ background: color }}
      />
      {label}
    </span>
  );
}
