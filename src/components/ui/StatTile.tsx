import type { ComponentType } from "react";

type StatAccent = "blue" | "green" | "red" | "amber" | "gray";

interface StatTileProps {
  icon: ComponentType<{ size?: number; className?: string }>;
  value: number | string;
  label: string;
  accent?: StatAccent;
}

export function StatTile({
  icon: Icon,
  value,
  label,
  accent = "gray",
}: StatTileProps) {
  return (
    <div className={`ui-stat-tile ui-stat-tile-${accent}`}>
      <span className={`ui-stat-tile-icon ui-stat-tile-icon-${accent}`}>
        <Icon size={18} />
      </span>
      <div className="ui-stat-tile-body">
        <div className="ui-stat-tile-value">{value}</div>
        <div className="ui-stat-tile-label">{label}</div>
      </div>
    </div>
  );
}
