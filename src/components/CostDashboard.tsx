import { useCallback, useEffect, useState } from "react";
import { api, type CostStats } from "../lib/api";

const PRIORITY_LABELS: Record<number, { name: string; desc: string; color: string }> = {
  1: { name: "Priority 1", desc: "Free / Subscription", color: "var(--color-success)" },
  2: { name: "Priority 2", desc: "Economy API", color: "var(--color-warning)" },
  3: { name: "Priority 3", desc: "Official API", color: "var(--color-danger)" },
};

export function CostDashboard() {
  const [stats, setStats] = useState<CostStats | null>(null);

  const fetchData = useCallback(async () => {
    try {
      const s = await api.costStats();
      setStats(s);
    } catch {
      // Gateway not running
    }
  }, []);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 5000);
    return () => clearInterval(interval);
  }, [fetchData]);

  if (!stats) {
    return <div className="panel-loading">Loading cost data...</div>;
  }

  const maxTierRequests = Math.max(
    ...stats.priority_breakdown.map((t) => t.requests),
    1
  );

  return (
    <section>
      <div className="panel-header">
        <h2 className="panel-title">Cost Dashboard</h2>
        <button className="btn btn-sm" onClick={fetchData}>
          Refresh
        </button>
      </div>

      {/* Summary cards */}
      <div className="cost-summary-grid">
        <div className="stat-card">
          <div className="stat-value">{stats.total_requests}</div>
          <div className="stat-label">Total Requests</div>
        </div>
        <div className="stat-card">
          <div className="stat-value">${stats.total_estimated_cost.toFixed(4)}</div>
          <div className="stat-label">Estimated Cost</div>
        </div>
        <div className="stat-card">
          <div className="stat-value">
            ${stats.total_requests > 0
              ? (stats.total_estimated_cost / stats.total_requests).toFixed(4)
              : "0"}
          </div>
          <div className="stat-label">Avg Cost / Request</div>
        </div>
        <div className="stat-card">
          <div className="stat-value">
            {(stats.total_input_tokens / 1_000_000).toFixed(2)}M / {(stats.total_output_tokens / 1_000_000).toFixed(2)}M
          </div>
          <div className="stat-label">Input / Output Tokens</div>
        </div>
      </div>

      {/* Per-priority breakdown */}
      <h3 className="cost-section-title">Priority Breakdown</h3>
      <div className="cost-tier-bars">
        {[1, 2, 3].map((priority) => {
          const data = stats.priority_breakdown.find((t) => t.priority === priority);
          const meta = PRIORITY_LABELS[priority] || { name: `Priority ${priority}`, desc: "", color: "var(--color-text-muted)" };
          const requests = data?.requests || 0;
          const cost = data?.estimated_cost || 0;
          const pct = maxTierRequests > 0 ? (requests / maxTierRequests) * 100 : 0;

          return (
            <div key={priority} className="cost-tier-row">
              <div className="cost-tier-label">
                <span className="cost-tier-dot" style={{ background: meta.color }} />
                <span>
                  <strong>{meta.name}</strong>
                  <br />
                  <span style={{ fontSize: "var(--text-xs)", color: "var(--color-text-muted)" }}>{meta.desc}</span>
                </span>
              </div>
              <div className="cost-tier-bar-track">
                <div
                  className="cost-tier-bar-fill"
                  style={{ width: `${pct}%`, background: meta.color }}
                />
              </div>
              <div className="cost-tier-stats">
                <span>{requests} req</span>
                <span className="separator">|</span>
                <span>${cost.toFixed(4)}</span>
              </div>
            </div>
          );
        })}
      </div>

      {/* Model usage */}
      {Object.keys(stats.model_counts).length > 0 && (
        <>
          <h3 className="cost-section-title">Model Usage</h3>
          <div className="cost-model-table">
            {Object.entries(stats.model_counts)
              .sort(([, a], [, b]) => b - a)
              .map(([model, count]) => (
                <div key={model} className="cost-model-row">
                  <span className="mono">{model}</span>
                  <span>{count} requests</span>
                </div>
              ))}
          </div>
        </>
      )}
    </section>
  );
}
