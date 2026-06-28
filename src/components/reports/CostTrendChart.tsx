import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { UsageReportRow } from "../../lib/api/types";
import { formatCents, formatNumber, formatTokens } from "../../lib/format";

type Metric = "cost" | "requests" | "tokens";

interface AggregatedDay {
  date: string;
  cost: number;
  requests: number;
  tokens: number;
}

interface CostTrendChartProps {
  rows: UsageReportRow[];
  avgDailyCostCents: number;
}

const PADDING_LEFT = 60;
const PADDING_RIGHT = 20;
const PADDING_TOP = 20;
const PADDING_BOTTOM = 40;
const VB_W = 800;
const VB_H = 280;
const CHART_W = VB_W - PADDING_LEFT - PADDING_RIGHT;
const CHART_H = VB_H - PADDING_TOP - PADDING_BOTTOM;
const BOTTOM_Y = PADDING_TOP + CHART_H;

function aggregateByDate(rows: UsageReportRow[]): AggregatedDay[] {
  const map = new Map<string, AggregatedDay>();
  for (const row of rows) {
    const prev = map.get(row.date);
    if (prev) {
      prev.cost += row.estimated_cost_cents;
      prev.requests += row.requests;
      prev.tokens += row.total_tokens;
    } else {
      map.set(row.date, {
        date: row.date,
        cost: row.estimated_cost_cents,
        requests: row.requests,
        tokens: row.total_tokens,
      });
    }
  }
  return [...map.values()].sort((a, b) => (a.date < b.date ? -1 : a.date > b.date ? 1 : 0));
}

function shortDate(iso: string): string {
  const [, m, d] = iso.split("-");
  return `${parseInt(m, 10)}/${parseInt(d, 10)}`;
}

function daysInCurrentMonth(): number {
  const now = new Date();
  return new Date(now.getFullYear(), now.getMonth() + 1, 0).getDate();
}

function formatMetricValue(v: number, metric: Metric): string {
  switch (metric) {
    case "cost":
      return formatCents(v);
    case "requests":
      return formatNumber(v);
    case "tokens":
      return formatTokens(v);
  }
}

const METRICS: { key: Metric; i18nKey: string }[] = [
  { key: "cost", i18nKey: "chartMetricCost" },
  { key: "requests", i18nKey: "chartMetricRequests" },
  { key: "tokens", i18nKey: "chartMetricTokens" },
];

const N_GRIDLINES = 5;
const N_X_LABELS = 6;

export function CostTrendChart({ rows, avgDailyCostCents }: CostTrendChartProps) {
  const { t } = useTranslation();
  const [metric, setMetric] = useState<Metric>("cost");

  const days = useMemo(() => aggregateByDate(rows), [rows]);

  if (days.length === 0) return null;

  const values = useMemo(
    () => days.map((d) => (metric === "cost" ? d.cost : metric === "requests" ? d.requests : d.tokens)),
    [days, metric],
  );

  const maxValue = Math.max(...values, 1);

  const points = useMemo(
    () =>
      values.map((v, i) => {
        const x = PADDING_LEFT + (days.length > 1 ? (i / (days.length - 1)) * CHART_W : CHART_W / 2);
        const y = PADDING_TOP + CHART_H - (v / maxValue) * CHART_H;
        return { x, y, value: v, date: days[i].date };
      }),
    [values, days, maxValue],
  );

  const fillPolygon = useMemo(
    () =>
      `${points[0].x},${BOTTOM_Y} ${points.map((p) => `${p.x},${p.y}`).join(" ")} ${points[points.length - 1].x},${BOTTOM_Y}`,
    [points],
  );

  const linePoints = useMemo(() => points.map((p) => `${p.x},${p.y}`).join(" "), [points]);

  const gridlines = useMemo(
    () =>
      Array.from({ length: N_GRIDLINES }, (_, i) => {
        const value = (maxValue / (N_GRIDLINES - 1)) * i;
        const y = PADDING_TOP + CHART_H - (value / maxValue) * CHART_H;
        return { y, label: formatMetricValue(value, metric) };
      }),
    [maxValue, metric],
  );

  const xLabels = useMemo(() => {
    if (days.length === 0) return [];
    const count = Math.min(N_X_LABELS, days.length);
    const indices = Array.from({ length: count }, (_, i) =>
      count === 1 ? 0 : Math.round((i / (count - 1)) * (days.length - 1)),
    );
    return indices.map((i) => ({
      x: points[i].x,
      label: shortDate(days[i].date),
    }));
  }, [days, points]);

  const projectedMonthly = avgDailyCostCents * 30;
  const periodTotal = days.reduce((s, d) => s + d.cost, 0);
  const now = new Date();
  const daysRemaining = daysInCurrentMonth() - now.getDate();

  return (
    <div className="cost-trend-chart">
      <div className="chart-metric-toggle">
        {METRICS.map((m) => (
          <button
            key={m.key}
            className={`chart-metric-btn${metric === m.key ? " active" : ""}`}
            onClick={() => setMetric(m.key)}
          >
            {t(`reports.${m.i18nKey}`)}
          </button>
        ))}
      </div>

      <svg
        className="chart-svg"
        viewBox={`0 0 ${VB_W} ${VB_H}`}
        preserveAspectRatio="xMidYMid meet"
      >
        <defs>
          <linearGradient id="cost-trend-fill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="var(--color-accent)" stopOpacity="0.15" />
            <stop offset="100%" stopColor="var(--color-accent)" stopOpacity="0.02" />
          </linearGradient>
        </defs>

        {/* Y-axis gridlines and labels */}
        {gridlines.map((g, i) => (
          <g key={i}>
            <line
              x1={PADDING_LEFT}
              y1={g.y}
              x2={VB_W - PADDING_RIGHT}
              y2={g.y}
              stroke="var(--color-border)"
              strokeWidth="1"
            />
            <text
              x={PADDING_LEFT - 8}
              y={g.y + 4}
              textAnchor="end"
              fill="var(--color-text-secondary)"
              fontSize="11"
            >
              {g.label}
            </text>
          </g>
        ))}

        {/* Area fill under the line */}
        <polygon points={fillPolygon} fill="url(#cost-trend-fill)" />

        {/* Line */}
        <polyline
          points={linePoints}
          fill="none"
          stroke="var(--color-accent)"
          strokeWidth="2"
          strokeLinejoin="round"
          strokeLinecap="round"
        />

        {/* Data point dots */}
        {points.map((p, i) => (
          <circle key={i} cx={p.x} cy={p.y} r={3} fill="var(--color-accent)">
            <title>{`${p.date}: ${formatMetricValue(p.value, metric)}`}</title>
          </circle>
        ))}

        {/* X-axis date labels */}
        {xLabels.map((xl, i) => (
          <text
            key={i}
            x={xl.x}
            y={VB_H - 6}
            textAnchor="end"
            fill="var(--color-text-secondary)"
            fontSize="11"
            transform={`rotate(-30, ${xl.x}, ${VB_H - 6})`}
          >
            {xl.label}
          </text>
        ))}
      </svg>

      {/* Monthly projection cards */}
      <div className="chart-projection">
        <div className="chart-projection-card chart-projection-primary">
          <span className="chart-projection-label">{t("reports.projectedMonthly")}</span>
          <span className="chart-projection-value">{formatCents(projectedMonthly)}</span>
        </div>
        <div className="chart-projection-card">
          <span className="chart-projection-label">{t("reports.periodTotal")}</span>
          <span className="chart-projection-value">{formatCents(periodTotal)}</span>
        </div>
        <div className="chart-projection-card">
          <span className="chart-projection-label">{t("reports.avgDaily")}</span>
          <span className="chart-projection-value">{formatCents(avgDailyCostCents)}</span>
        </div>
        <div className="chart-projection-card">
          <span className="chart-projection-label">{t("reports.daysRemaining")}</span>
          <span className="chart-projection-value">{daysRemaining}</span>
        </div>
      </div>
    </div>
  );
}