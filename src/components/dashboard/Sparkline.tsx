import { useId } from "react";
import { computeSparkArea, computeSparkPoints } from "./helpers";

export interface SparklineProps {
  values: number[];
  width?: number;
  height?: number;
  color?: string;
  fillOpacity?: number;
  strokeWidth?: number;
}

/**
 * Lightweight SVG sparkline with a gradient-filled area under the line.
 * Uses preserveAspectRatio="none" so it scales to fill its container width.
 */
export function Sparkline({
  values,
  width = 120,
  height = 36,
  color = "var(--color-accent)",
  fillOpacity = 0.15,
  strokeWidth = 1.5,
}: SparklineProps) {
  const rawId = useId();
  // useId returns characters that are invalid in SVG ids in some edge cases;
  // sanitize to alphanumeric.
  const gradientId = `dsh-spark-${rawId.replace(/[^a-zA-Z0-9]/g, "")}`;

  if (values.length === 0) {
    return (
      <svg
        className="dsh-sparkline"
        viewBox={`0 0 ${width} ${height}`}
        preserveAspectRatio="none"
        aria-hidden="true"
      />
    );
  }

  const linePath = computeSparkPoints(values, width, height);
  const areaPath = computeSparkArea(values, width, height);

  return (
    <svg
      className="dsh-sparkline"
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="none"
      role="img"
      aria-label="Trend sparkline"
    >
      <defs>
        <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={color} stopOpacity={fillOpacity} />
          <stop offset="100%" stopColor={color} stopOpacity={0} />
        </linearGradient>
      </defs>
      {areaPath && <path d={areaPath} fill={`url(#${gradientId})`} />}
      <path
        d={linePath}
        fill="none"
        stroke={color}
        strokeWidth={strokeWidth}
        strokeLinejoin="round"
        strokeLinecap="round"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}
