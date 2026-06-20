import type { EntityStat } from "./helpers";
import { formatCost, formatNumber } from "./helpers";

export interface TopEntitiesProps {
  channels: EntityStat[];
  models: EntityStat[];
}

export function TopEntities({ channels, models }: TopEntitiesProps) {
  return (
    <div className="dsh-top-grid">
      <TopCard title="Top Channels" items={channels} accent="var(--color-accent)" />
      <TopCard title="Top Models" items={models} accent="var(--color-success)" />
    </div>
  );
}

function TopCard({
  title,
  items,
  accent,
}: {
  title: string;
  items: EntityStat[];
  accent: string;
}) {
  const max = Math.max(...items.map((i) => i.requests), 1);

  return (
    <div className="dsh-card dsh-top-card">
      <div className="dsh-card-title-row">
        <span className="dsh-card-title">{title}</span>
        <span className="dsh-card-count">{items.length}</span>
      </div>
      {items.length === 0 ? (
        <div className="dsh-top-empty">No data yet</div>
      ) : (
        <div className="dsh-top-list">
          {items.slice(0, 5).map((item, i) => (
            <div key={item.id} className="dsh-top-row">
              <span className="dsh-top-rank">{i + 1}</span>
              <div className="dsh-top-main">
                <div className="dsh-top-name" title={item.name}>
                  {item.name}
                </div>
                <div className="dsh-top-bar-bg">
                  <div
                    className="dsh-top-bar-fill"
                    style={{
                      width: `${(item.requests / max) * 100}%`,
                      background: accent,
                    }}
                  />
                </div>
              </div>
              <span className="dsh-top-reqs">{formatNumber(item.requests)}</span>
              <span className="dsh-top-cost">{formatCost(item.cost)}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
