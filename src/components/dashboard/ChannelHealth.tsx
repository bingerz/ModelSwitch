import { useMemo } from "react";
import type { Channel, DispatchLog } from "../../lib/api";
import { formatNumber } from "./helpers";
import { Sparkline } from "./Sparkline";

export interface ChannelHealthProps {
  channels: Channel[];
  usageByChannel: Map<string, {
    requests: number;
    tokens: number;
    cost: number;
    spark: number[];
  }>;
  logs: DispatchLog[];
  healthyCount?: number;
  circuitOpenCount?: number;
  disabledCount?: number;
}

const STATUS_PRIORITY: Record<string, number> = {
  circuit_open: 0,
  disabled: 1,
  healthy: 2,
};

function statusColor(status: string): string {
  if (status === "healthy") return "var(--color-success)";
  if (status === "circuit_open") return "var(--color-danger)";
  return "var(--color-text-muted)";
}

function errorRate(chId: string, logs: DispatchLog[]): number {
  const relevant = logs.filter((l) => l.channel_id === chId);
  if (relevant.length === 0) return 0;
  const failures = relevant.filter((l) => !l.success).length;
  return (failures / relevant.length) * 100;
}

export function ChannelHealth({
  channels, usageByChannel, logs,
  healthyCount, circuitOpenCount, disabledCount,
}: ChannelHealthProps) {
  const sorted = useMemo(() => {
    return [...channels].sort((a, b) => {
      const pa = STATUS_PRIORITY[a.status] ?? 99;
      const pb = STATUS_PRIORITY[b.status] ?? 99;
      if (pa !== pb) return pa - pb;
      return b.avg_latency_ms - a.avg_latency_ms;
    });
  }, [channels]);

  if (channels.length === 0) {
    return (
      <div className="dsh-card dsh-health-card">
        <div className="dsh-card-title">Channel Health</div>
        <div className="dsh-activity-empty">
          <div className="dsh-activity-empty-title">No channels</div>
          <div className="dsh-activity-empty-desc">
            Add channels to see live health data.
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="dsh-card dsh-health-card">
      <div className="dsh-card-title-row">
        <div className="dsh-health-title-group">
          <span className="dsh-card-title">Channel Health</span>
          <span className="dsh-card-count">{channels.length}</span>
        </div>
        <div className="dsh-health-pills">
          {healthyCount !== undefined && healthyCount > 0 && (
            <span className="dsh-health-pill dsh-health-pill-success">{healthyCount} healthy</span>
          )}
          {circuitOpenCount !== undefined && circuitOpenCount > 0 && (
            <span className="dsh-health-pill dsh-health-pill-danger">{circuitOpenCount} broken</span>
          )}
          {disabledCount !== undefined && disabledCount > 0 && (
            <span className="dsh-health-pill dsh-health-pill-muted">{disabledCount} disabled</span>
          )}
        </div>
      </div>
      <div className="dsh-health-list">
        {sorted.map((ch) => {
          const usage = usageByChannel.get(ch.id);
          const reqs = usage?.requests ?? 0;
          const errPct = errorRate(ch.id, logs);
          const spark = usage?.spark ?? [];
          return (
            <div key={ch.id} className="dsh-health-row">
              <span
                className={`dsh-health-dot ${
                  ch.status === "circuit_open" ? "dsh-health-dot-pulse" : ""
                }`}
                style={{ background: statusColor(ch.status) }}
                title={ch.status}
              />
              <span className="dsh-health-name" title={ch.name}>
                {ch.name}
              </span>
              <span className="dsh-health-provider">{ch.provider}</span>
              <span className="dsh-health-stats">
                {formatNumber(reqs)} req &middot; {Math.round(ch.avg_latency_ms)}ms
                {errPct > 0 && (
                  <span
                    className="dsh-health-err"
                    style={{
                      color:
                        errPct > 5
                          ? "var(--color-danger)"
                          : "var(--color-warning)",
                    }}
                  >
                    {" "}
                    &middot; {errPct.toFixed(1)}% err
                  </span>
                )}
              </span>
              {spark.length > 0 && (
                <span className="dsh-health-spark">
                  <Sparkline
                    values={spark}
                    width={60}
                    height={20}
                    color="var(--color-accent)"
                    strokeWidth={1.25}
                  />
                </span>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
