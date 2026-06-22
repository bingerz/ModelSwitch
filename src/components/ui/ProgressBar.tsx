interface ProgressBarProps {
  value: number;
  max?: number;
  color?: string;
  thresholds?: { upto: number; color: string }[];
  height?: number;
  showLabel?: boolean;
  label?: string;
}

export function ProgressBar({
  value,
  max = 100,
  color,
  thresholds,
  height = 8,
  showLabel = false,
  label,
}: ProgressBarProps) {
  const pct = max > 0 ? Math.min(100, Math.max(0, (value / max) * 100)) : 0;

  let barColor = color ?? "var(--color-accent)";
  if (thresholds) {
    for (const { upto, color: thresholdColor } of thresholds) {
      if (pct <= upto) {
        barColor = thresholdColor;
        break;
      }
    }
  }

  return (
    <div className="ui-progress" style={{ height: `${height}px` }}>
      <div
        className="ui-progress-fill"
        style={{ width: `${pct}%`, background: barColor }}
      />
      {showLabel && (
        <span className="ui-progress-label">
          {label ?? `${pct.toFixed(0)}%`}
        </span>
      )}
    </div>
  );
}
