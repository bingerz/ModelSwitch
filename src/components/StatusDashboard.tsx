import { useCallback, useEffect, useState } from "react";
import { api, type Channel, type DispatchLog, type DispatchStats } from "../lib/api";
import { useQuota } from "../hooks/useQuota";
import "../styles/status-dashboard.css";

const STATUS_PRIORITY: Record<string, number> = {
  circuit_open: 0,
  disabled: 1,
  healthy: 2,
};

const BAR_MAX_HEIGHT = 80;
const LOG_LIMIT = 20;

function sortChannelsByHealth(channels: Channel[]): Channel[] {
  return [...channels].sort(
    (a, b) => (STATUS_PRIORITY[a.status] ?? 99) - (STATUS_PRIORITY[b.status] ?? 99)
  );
}

function renderStatCard(
  icon: string,
  value: string | number,
  label: string,
  accentClass: string,
  valueClass: string
) {
  return (
    <div className={`sd-stat-card ${accentClass}`}>
      <span className="sd-stat-icon">{icon}</span>
      <div className={`sd-stat-value ${valueClass}`}>{value}</div>
      <div className="sd-stat-label">{label}</div>
    </div>
  );
}

function renderHealthDot(status: string): string {
  if (status === "healthy") return "sd-health-dot sd-health-dot-healthy";
  if (status === "circuit_open") return "sd-health-dot sd-health-dot-circuit-open";
  return "sd-health-dot sd-health-dot-disabled";
}

function getStatusLabel(status: string): string {
  if (status === "healthy") return "Healthy";
  if (status === "circuit_open") return "Circuit Broken";
  return "Disabled";
}

function renderHealthList(sortedChannels: Channel[]) {
  return (
    <div className="sd-section">
      <div className="sd-section-header">
        <h3 className="sd-section-title">Channel Health</h3>
        <span className="sd-section-count">{sortedChannels.length}</span>
      </div>
      {sortedChannels.length === 0 ? (
        <div className="empty-state">
          <div className="empty-state-icon">📡</div>
          <div className="empty-state-title">No Channels</div>
          <div className="empty-state-description">
            Add channels in the Channels tab to see health information.
          </div>
        </div>
      ) : (
        <div className="sd-health-list">
          {sortedChannels.map((channel) => (
            <div key={channel.id} className="sd-health-item">
              <span className={renderHealthDot(channel.status)} />
              <span className="sd-health-name" title={`${channel.name} - ${getStatusLabel(channel.status)}`}>
                {channel.name}
              </span>
              <span className="sd-health-provider">{channel.provider}</span>
              {channel.status === "circuit_open" && channel.consecutive_failures > 0 && (
                <span className="sd-health-warning">
                  {"\u26A0\uFE0F"} {channel.consecutive_failures}
                </span>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function renderActivityBars(logs: DispatchLog[]) {
  if (logs.length === 0) {
    return (
      <div className="sd-section">
        <div className="empty-state">
          <div className="empty-state-icon">📊</div>
          <div className="empty-state-title">No Activity Yet</div>
          <div className="empty-state-description">
            Recent dispatch activity will appear here once requests are processed.
          </div>
        </div>
      </div>
    );
  }

  const maxLatency = Math.max(...logs.map((l) => l.latency_ms), 1);
  const successCount = logs.filter((l) => l.success).length;
  const failureCount = logs.filter((l) => !l.success).length;

  return (
    <div className="sd-section">
      <div className="sd-activity-card">
        <div className="sd-activity-header">
          <span className="sd-activity-title">Recent Activity</span>
          <span className="sd-activity-meta">
            {successCount}/{logs.length} success
          </span>
        </div>
        <div className="sd-activity-bars-wrap">
          {logs.map((log) => {
            const height = Math.max((log.latency_ms / maxLatency) * BAR_MAX_HEIGHT, 2);
            const barClass = log.success
              ? "sd-activity-bar sd-activity-bar-success"
              : "sd-activity-bar sd-activity-bar-failure";
            return (
              <div
                key={log.id}
                className={barClass}
                style={{ height: `${height}px` }}
                title={`${log.channel_name}: ${log.latency_ms}ms (${log.success ? "success" : "failure"})`}
              />
            );
          })}
        </div>
        <div className="sd-activity-legend">
          <div className="sd-activity-legend-item">
            <span className="sd-activity-legend-dot sd-activity-legend-dot-success" />
            Success ({successCount})
          </div>
          <div className="sd-activity-legend-item">
            <span className="sd-activity-legend-dot sd-activity-legend-dot-failure" />
            Failure ({failureCount})
          </div>
        </div>
      </div>
    </div>
  );
}

function renderQuotaSection(
  totalBalance: number,
  channelsWithDataCount: number,
  lowBalance: number,
  errors: number,
  totalCh: number
) {
  if (totalCh === 0) return null;

  return (
    <div className="sd-section">
      <div className="sd-section-header">
        <h3 className="sd-section-title">Quota Summary</h3>
      </div>
      <div className="sd-quota-grid">
        <div className="sd-quota-item">
          <span className="sd-quota-item-label">Total Balance</span>
          <span className="sd-quota-item-value">${totalBalance.toFixed(2)}</span>
        </div>
        <div className="sd-quota-item">
          <span className="sd-quota-item-label">With Data</span>
          <span className="sd-quota-item-value">
            {channelsWithDataCount}/{totalCh}
          </span>
        </div>
        {lowBalance > 0 && (
          <div className="sd-quota-item">
            <span className="sd-quota-item-label">Low Balance</span>
            <span className="sd-quota-item-value sd-quota-item-value-warn">
              {lowBalance}
            </span>
          </div>
        )}
        {errors > 0 && (
          <div className="sd-quota-item">
            <span className="sd-quota-item-label">Errors</span>
            <span className="sd-quota-item-value sd-quota-item-value-danger">
              {errors}
            </span>
          </div>
        )}
      </div>
      <p className="sd-quota-hint">
        View detailed quota info on the Quota tab in the sidebar.
      </p>
    </div>
  );
}

export function StatusDashboard() {
  const [channels, setChannels] = useState<Channel[]>([]);
  const [stats, setStats] = useState<DispatchStats | null>(null);
  const [logs, setLogs] = useState<DispatchLog[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const { totalBalance, channelsWithData, lowBalanceCount, errorCount, totalChannels } = useQuota();

  const fetchData = useCallback(async () => {
    try {
      const [ch, st, lg] = await Promise.all([
        api.listChannels(),
        api.stats(),
        api.logs(0, LOG_LIMIT),
      ]);
      setChannels(ch);
      setStats(st);
      // Reverse so oldest entry appears first (chronological left-to-right)
      setLogs([...lg].reverse());
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to fetch dashboard data");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 5000);
    return () => clearInterval(interval);
  }, [fetchData]);

  if (loading) {
    return (
      <section>
        <div className="panel-header">
          <h2 className="panel-title">Status Dashboard</h2>
        </div>
        <div className="sd-loading">
          <div className="sd-loading-spinner" />
          <span className="sd-loading-text">Loading dashboard data...</span>
        </div>
      </section>
    );
  }

  if (error) {
    return (
      <section>
        <div className="panel-header">
          <h2 className="panel-title">Status Dashboard</h2>
        </div>
        <div className="sd-error">
          <span className="sd-error-icon">{"\u26A0\uFE0F"}</span>
          <div className="sd-error-text">{error}</div>
          <button className="sd-error-btn" onClick={fetchData}>
            Retry
          </button>
        </div>
      </section>
    );
  }

  const healthyCount = channels.filter((c) => c.status === "healthy").length;
  const circuitOpenCount = channels.filter((c) => c.status === "circuit_open").length;
  const disabledCount = channels.filter((c) => c.status === "disabled").length;

  return (
    <section>
      <div className="panel-header">
        <h2 className="panel-title">Status Dashboard</h2>
      </div>

      {/* Enhanced Stat Cards */}
      <div className="sd-stats-row">
        {renderStatCard("\uD83D\uDCE1", channels.length, "Total Channels", "sd-stat-card-accent-blue", "sd-stat-value-blue")}
        {renderStatCard("\u2705", healthyCount, "Healthy", "sd-stat-card-accent-green", "sd-stat-value-green")}
        {renderStatCard("\u26A0\uFE0F", circuitOpenCount, "Circuit Broken", "sd-stat-card-accent-amber", "sd-stat-value-amber")}
        {renderStatCard("\u26D4\uFE0F", disabledCount, "Disabled", "sd-stat-card-accent-gray", "sd-stat-value-gray")}
      </div>

      {/* Stats row (requests data) */}
      {stats && (
        <div className="sd-stats-row">
          {renderStatCard("\uD83D\uDCCA", stats.total_requests, "Total Requests", "sd-stat-card-accent-blue", "sd-stat-value-blue")}
          {renderStatCard("\u2705", stats.successes, "Successes", "sd-stat-card-accent-green", "sd-stat-value-green")}
          {renderStatCard("\u274C", stats.failures, "Failures", "sd-stat-card-accent-red", "sd-stat-value-red")}
          {renderStatCard("\u23F1\uFE0F", `${stats.avg_latency_ms}ms`, "Avg Latency", "sd-stat-card-accent-blue", "sd-stat-value-blue")}
        </div>
      )}

      {/* Channel Health List */}
      {renderHealthList(sortChannelsByHealth(channels))}

      {/* Mini Activity Visualization */}
      {renderActivityBars(logs)}

      {/* Quota Summary */}
      {renderQuotaSection(totalBalance, channelsWithData, lowBalanceCount, errorCount, totalChannels)}
    </section>
  );
}
