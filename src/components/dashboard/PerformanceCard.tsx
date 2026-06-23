import type { ComponentType } from "react";
import { useTranslation } from "react-i18next";
import { Sparkline } from "./Sparkline";
import { CircleCheck, CircleX, DollarSign, Timer } from "./icons";
import { formatNumber } from "./helpers";

type IconType = ComponentType<{ size?: number; className?: string }>;

interface Quadrant {
  icon: IconType;
  value: string;
  label: string;
  accent: "green" | "red" | "amber" | "blue";
  spark: number[];
}

export interface PerformanceCardProps {
  successes: number;
  failures: number;
  avgLatencyMs: number | null;
  totalCost: number;
  totalTokens: number;
  successSpark: number[];
  failureSpark: number[];
  latencySpark: number[];
  costSpark: number[];
}

export function PerformanceCard({
  successes, failures, avgLatencyMs, totalCost, totalTokens,
  successSpark, failureSpark, latencySpark, costSpark,
}: PerformanceCardProps) {
  const { t } = useTranslation();
  const quadrants: Quadrant[] = [
    { icon: CircleCheck, value: formatNumber(successes), label: t("dashboard.successes"), accent: "green", spark: successSpark },
    { icon: CircleX, value: formatNumber(failures), label: t("dashboard.failures"), accent: "red", spark: failureSpark },
    { icon: Timer, value: avgLatencyMs != null ? `${Math.round(avgLatencyMs)}ms` : "\u2014", label: t("dashboard.avgLatency"), accent: "amber", spark: latencySpark },
    { icon: DollarSign, value: `$${totalCost.toFixed(2)}`, label: t("dashboard.estCost24h"), accent: "blue", spark: costSpark },
  ];

  return (
    <div className="dsh-card dsh-performance-card">
      <div className="dsh-card-title-row">
        <span className="dsh-card-title">{t("dashboard.performance24h")}</span>
        <span className="dsh-performance-tokens">
          {t("dashboard.tokensLabel", { count: formatNumber(totalTokens) })}
        </span>
      </div>
      <div className="dsh-performance-grid">
        {quadrants.map((q, i) => {
          const Icon = q.icon;
          return (
            <div key={i} className={`dsh-performance-quadrant dsh-performance-quadrant-${q.accent}`}>
              <div className="dsh-performance-quadrant-header">
                <span className={`dsh-performance-icon dsh-performance-icon-${q.accent}`}>
                  <Icon size={16} />
                </span>
                <span className="dsh-performance-value">{q.value}</span>
              </div>
              <div className="dsh-performance-label">{q.label}</div>
              <div className="dsh-performance-spark">
                <Sparkline
                  values={q.spark}
                  width={120}
                  height={32}
                  color={`var(--perf-accent, var(--color-accent))`}
                  strokeWidth={1.5}
                />
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
