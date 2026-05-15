import { useCallback, useEffect, useState } from "react";
import { api, type Channel, type DispatchStats } from "../lib/api";
import { useQuota } from "../hooks/useQuota";

export function StatusDashboard() {
  const [channels, setChannels] = useState<Channel[]>([]);
  const [stats, setStats] = useState<DispatchStats | null>(null);
  const { totalBalance, channelsWithData, lowBalanceCount, errorCount, totalChannels } = useQuota();

  const fetchData = useCallback(async () => {
    try {
      const [ch, st] = await Promise.all([
        api.listChannels(),
        api.stats(),
      ]);
      setChannels(ch);
      setStats(st);
    } catch {
      // Silently ignore fetch failures
    }
  }, []);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 5000);
    return () => clearInterval(interval);
  }, [fetchData]);

  const healthy = channels.filter((c) => c.status === "healthy").length;
  const circuitOpen = channels.filter((c) => c.status === "circuit_open").length;
  const disabled = channels.filter((c) => c.status === "disabled").length;

  return (
    <section>
      <div className="panel-header">
        <h2 className="panel-title">Status Dashboard</h2>
      </div>

      <div className="dashboard-grid">
        <div className="stat-card">
          <div className="stat-value">{channels.length}</div>
          <div className="stat-label">Total Channels</div>
        </div>
        <div className="stat-card stat-success">
          <div className="stat-value">{healthy}</div>
          <div className="stat-label">Healthy</div>
        </div>
        <div className="stat-card stat-danger">
          <div className="stat-value">{circuitOpen}</div>
          <div className="stat-label">Circuit Open</div>
        </div>
        <div className="stat-card stat-muted">
          <div className="stat-value">{disabled}</div>
          <div className="stat-label">Disabled</div>
        </div>
      </div>

      {stats && (
        <div className="dashboard-grid" style={{ marginTop: "var(--space-6)" }}>
          <div className="stat-card">
            <div className="stat-value">{stats.total_requests}</div>
            <div className="stat-label">Total Requests</div>
          </div>
          <div className="stat-card stat-success">
            <div className="stat-value">{stats.successes}</div>
            <div className="stat-label">Successes</div>
          </div>
          <div className="stat-card stat-danger">
            <div className="stat-value">{stats.failures}</div>
            <div className="stat-label">Failures</div>
          </div>
          <div className="stat-card">
            <div className="stat-value">{stats.avg_latency_ms}ms</div>
            <div className="stat-label">Avg Latency</div>
          </div>
        </div>
      )}

      {/* Quota summary — links to dedicated Quota tab */}
      {totalChannels > 0 && (
        <>
          <div className="dashboard-grid" style={{ marginTop: "var(--space-6)" }}>
            <div className="stat-card">
              <div className="stat-value">${totalBalance.toFixed(2)}</div>
              <div className="stat-label">Total Balance</div>
            </div>
            <div className="stat-card stat-success">
              <div className="stat-value">{channelsWithData}/{totalChannels}</div>
              <div className="stat-label">Quota Monitored</div>
            </div>
            {errorCount > 0 && (
              <div className="stat-card stat-danger">
                <div className="stat-value">{errorCount}</div>
                <div className="stat-label">Quota Errors</div>
              </div>
            )}
            {lowBalanceCount > 0 && (
              <div className="stat-card stat-danger">
                <div className="stat-value">{lowBalanceCount}</div>
                <div className="stat-label">Low Balance</div>
              </div>
            )}
          </div>
          <p style={{ fontSize: "var(--text-xs)", color: "var(--color-text-muted)", marginTop: "var(--space-3)" }}>
            View detailed quota info on the Quota tab in the sidebar.
          </p>
        </>
      )}
    </section>
  );
}
