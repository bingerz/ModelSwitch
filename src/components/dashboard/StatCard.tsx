import type { ComponentType } from "react";
import { ArrowUpRight, TrendingDown, TrendingUp } from "./icons";
import { Sparkline } from "./Sparkline";

type IconType = ComponentType<{ size?: number; className?: string }>;

export type StatAccent = "blue" | "green" | "red" | "amber" | "gray";

export interface StatCardProps {
  icon: IconType;
  value: number | string;
  label: string;
  accent: StatAccent;
  sparkline?: number[];
  trend?: "up" | "down" | "flat";
  trendLabel?: string;
  subtitle?: string;
  size?: "mega" | "hero" | "regular";
}

export function StatCard({
  icon: Icon,
  value,
  label,
  accent,
  sparkline,
  trend,
  trendLabel,
  subtitle,
  size = "regular",
}: StatCardProps) {
  const isMega = size === "mega";
  const isHero = isMega || size === "hero";
  const trendIcon =
    trend === "up" ? TrendingUp : trend === "down" ? TrendingDown : ArrowUpRight;
  const TrendIcon = trendIcon;

  return (
    <div
      className={`dsh-card dsh-card-hover dsh-stat-card dsh-stat-card-${accent} ${
        isMega ? "dsh-card-mega" : isHero ? "dsh-card-hero" : ""
      }`}
    >
      <span className={`dsh-card-accent-bar dsh-accent-${accent}`} />
      <div className="dsh-stat-header">
        <span className={`dsh-stat-icon-chip dsh-stat-icon-chip-${accent}`}>
          <Icon size={isMega ? 28 : isHero ? 24 : 18} />
        </span>
        {trend && trendLabel && (
          <span
            className={`dsh-stat-trend dsh-stat-trend-${
              trend === "down" ? "down" : "up"
            }`}
          >
            <TrendIcon size={12} />
            {trendLabel}
          </span>
        )}
      </div>
      <div className={`dsh-stat-value ${isMega ? "dsh-stat-value-mega" : isHero ? "dsh-stat-value-hero" : ""}`}>
        {value}
      </div>
      <div className="dsh-stat-label">{label}</div>
      {subtitle && <div className="dsh-stat-subtitle">{subtitle}</div>}
      {sparkline && sparkline.length > 0 && (
        <div className={`dsh-stat-spark ${isMega ? "dsh-stat-spark-mega" : ""}`}>
          <Sparkline
            values={sparkline}
            width={isMega ? 240 : isHero ? 160 : 100}
            height={isMega ? 80 : isHero ? 48 : 32}
            color={`var(--stat-accent, var(--color-accent))`}
          />
        </div>
      )}
    </div>
  );
}
