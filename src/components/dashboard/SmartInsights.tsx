import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { Channel, DispatchLog, DispatchStats } from "../../lib/api";
import {
  Brain,
  CircleAlert,
  CircleCheck,
  LucideIcon,
  RotateCw,
  Timer,
  TrendingDown,
  TrendingUp,
  TriangleAlert,
} from "./icons";

export interface SmartInsightsProps {
  channels: Channel[];
  stats: DispatchStats | null;
  logs: DispatchLog[];
}

type Severity = "critical" | "warning" | "info";

interface Insight {
  id: string;
  severity: Severity;
  icon: LucideIcon;
  title: string;
  description: string;
}

const SEVERITY_RANK: Record<Severity, number> = {
  critical: 0,
  warning: 1,
  info: 2,
};

const MAX_INSIGHTS = 6;
const HIGH_LATENCY_THRESHOLD_MS = 3000;
const HIGH_FAILURE_RATE_THRESHOLD = 0.15;
const HIGH_RETRY_THRESHOLD = 2;
const HIGH_RETRY_INSIGHT_MIN = 3;
const CONSECUTIVE_FAILURE_THRESHOLD = 3;
const COST_SPIKE_RATIO = 1.5;
const RECENT_COST_WINDOW = 10;

function formatCostValue(value: number): string {
  if (value <= 0) return "$0";
  if (value < 0.01) return "<$0.01";
  return `$${value.toFixed(4)}`;
}

/**
 * Build the list of actionable insights from dashboard data.
 * Order matters: critical first, then warning, then info. The result is
 * truncated to MAX_INSIGHTS so the panel stays scannable.
 */
function buildInsights(
  channels: Channel[],
  stats: DispatchStats | null,
  logs: DispatchLog[],
  t: ReturnType<typeof useTranslation>["t"]
): Insight[] {
  const insights: Insight[] = [];

  // 1. Circuit-open channels (critical)
  for (const ch of channels) {
    if (ch.status !== "circuit_open") continue;
    insights.push({
      id: `circuit-open-${ch.id}`,
      severity: "critical",
      icon: TriangleAlert,
      title: t("insights.circuitOpen", { name: ch.name }),
      description: t("insights.circuitOpenDesc"),
    });
  }

  // 2. Elevated overall failure rate (warning)
  if (stats && stats.total_requests > 0) {
    const failureRate = stats.failures / stats.total_requests;
    if (failureRate > HIGH_FAILURE_RATE_THRESHOLD) {
      const pct = (failureRate * 100).toFixed(1);
      insights.push({
        id: "high-failure-rate",
        severity: "warning",
        icon: TrendingDown,
        title: t("insights.highFailureRate", { pct }),
        description: t("insights.highFailureRateDesc", {
          failures: stats.failures,
          total: stats.total_requests,
        }),
      });
    }
  }

  // 3. Per-channel high latency (warning)
  for (const ch of channels) {
    if (ch.avg_latency_ms <= HIGH_LATENCY_THRESHOLD_MS) continue;
    insights.push({
      id: `high-latency-${ch.id}`,
      severity: "warning",
      icon: Timer,
      title: t("insights.highLatency", {
        name: ch.name,
        ms: Math.round(ch.avg_latency_ms),
      }),
      description: t("insights.highLatencyDesc"),
    });
  }

  // 4. Frequent retries across recent logs (warning)
  const retryCount = logs.filter((l) => l.retry_count >= HIGH_RETRY_THRESHOLD).length;
  if (retryCount >= HIGH_RETRY_INSIGHT_MIN) {
    insights.push({
      id: "frequent-retries",
      severity: "warning",
      icon: RotateCw,
      title: t("insights.frequentRetries"),
      description: t("insights.frequentRetriesDesc", { count: retryCount }),
    });
  }

  // 5. Channels approaching circuit-breaker threshold (warning)
  for (const ch of channels) {
    if (ch.consecutive_failures < CONSECUTIVE_FAILURE_THRESHOLD) continue;
    if (ch.status === "circuit_open") continue; // already surfaced above
    insights.push({
      id: `consecutive-failures-${ch.id}`,
      severity: "warning",
      icon: CircleAlert,
      title: t("insights.consecutiveFailures", {
        name: ch.name,
        n: ch.consecutive_failures,
      }),
      description: t("insights.consecutiveFailuresDesc"),
    });
  }

  // 6. Recent cost spike (info)
  const costedLogs = logs.filter(
    (l) => l.estimated_cost != null && l.estimated_cost > 0
  );
  if (costedLogs.length >= RECENT_COST_WINDOW) {
    const overallAvg =
      costedLogs.reduce((sum, l) => sum + (l.estimated_cost ?? 0), 0) /
      costedLogs.length;
    const recent = costedLogs.slice(0, RECENT_COST_WINDOW);
    const recentAvg =
      recent.reduce((sum, l) => sum + (l.estimated_cost ?? 0), 0) / recent.length;
    if (overallAvg > 0 && recentAvg > overallAvg * COST_SPIKE_RATIO) {
      insights.push({
        id: "cost-spike",
        severity: "info",
        icon: TrendingUp,
        title: t("insights.costSpike"),
        description: t("insights.costSpikeDesc", {
          recent: formatCostValue(recentAvg),
          overall: formatCostValue(overallAvg),
        }),
      });
    }
  }

  // 7. Healthy state (info) — only when nothing critical/warning fired
  const hasActionable = insights.some(
    (i) => i.severity === "critical" || i.severity === "warning"
  );
  if (!hasActionable) {
    const sampleSize = stats?.total_requests ?? logs.length;
    insights.push({
      id: "all-healthy",
      severity: "info",
      icon: CircleCheck,
      title: t("insights.allHealthy"),
      description: t("insights.allHealthyDesc", { n: sampleSize }),
    });
  }

  return insights
    .sort((a, b) => {
      const rankDiff = SEVERITY_RANK[a.severity] - SEVERITY_RANK[b.severity];
      if (rankDiff !== 0) return rankDiff;
      return a.id.localeCompare(b.id);
    })
    .slice(0, MAX_INSIGHTS);
}

export function SmartInsights({ channels, stats, logs }: SmartInsightsProps) {
  const { t } = useTranslation();

  const insights = useMemo(
    () => buildInsights(channels, stats, logs, t),
    [channels, stats, logs, t]
  );

  const hasData = channels.length > 0 || logs.length > 0;

  return (
    <div className="smart-insights">
      <div className="insights-header">
        <span className="insights-header-icon">
          <Brain size={16} />
        </span>
        <h3>{t("insights.title")}</h3>
      </div>
      {!hasData ? (
        <div className="insights-empty">{t("insights.insufficientData")}</div>
      ) : (
        <div className="insights-grid">
          {insights.map((insight) => {
            const Icon = insight.icon;
            return (
              <div
                key={insight.id}
                className={`insight-card insight-${insight.severity}`}
              >
                <span className="insight-icon">
                  <Icon size={18} />
                </span>
                <div className="insight-content">
                  <div className="insight-title">{insight.title}</div>
                  <div className="insight-desc">{insight.description}</div>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
