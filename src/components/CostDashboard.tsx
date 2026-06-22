import { useCallback, useEffect, useState } from "react";
import { BarChart3, DollarSign, TrendingUp, Coins, Bot, Layers, Cpu } from "lucide-react";
import { api, type CostStats, PRIORITY_TIERS } from "../lib/api";
import { StatTile } from "./ui/StatTile";
import { SectionHeader } from "./ui/SectionHeader";
import { EmptyState } from "./ui/EmptyState";
import "../styles/pages-enhanced.css";

function formatNumber(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export function CostDashboard() {
  const [stats, setStats] = useState<CostStats | null>(null);
  const [error, setError] = useState<string | null>(null);

  const fetchData = useCallback(async () => {
    try {
      const s = await api.costStats();
      setStats(s);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to fetch cost data");
    }
  }, []);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 5000);
    return () => clearInterval(interval);
  }, [fetchData]);

  if (error && !stats) {
    return (
      <section>
        <SectionHeader title="Cost Dashboard" icon={BarChart3} onRefresh={fetchData} />
        <EmptyState
          icon={BarChart3}
          title="Unable to load cost data"
          description={error}
        />
      </section>
    );
  }

  if (!stats) {
    return (
      <div className="panel-loading-enhanced">
        <div className="spinner" />
        <span>Loading cost data...</span>
      </div>
    );
  }

  const maxTierRequests = Math.max(
    ...stats.priority_breakdown.map((t) => t.requests),
    1
  );

  return (
    <section>
      <SectionHeader title="Cost Dashboard" icon={BarChart3} onRefresh={fetchData} />

      {/* Summary cards */}
      <div className="cost-summary-grid">
        <StatTile icon={BarChart3} value={formatNumber(stats.total_requests)} label="Total Requests" accent="blue" />
        <StatTile icon={DollarSign} value={`$${stats.total_estimated_cost.toFixed(4)}`} label="Estimated Cost" accent="green" />
        <StatTile
          icon={TrendingUp}
          value={stats.total_requests > 0 ? `$${(stats.total_estimated_cost / stats.total_requests).toFixed(4)}` : "0"}
          label="Avg Cost / Request"
          accent="amber"
        />
        <StatTile
          icon={Coins}
          value={`${(stats.total_input_tokens / 1_000_000).toFixed(2)}M / ${(stats.total_output_tokens / 1_000_000).toFixed(2)}M`}
          label="Input / Output Tokens"
          accent="gray"
        />
      </div>

      {/* Per-priority breakdown */}
      <h3 className="cost-section-title">
        <Layers size={16} style={{ display: "inline", marginRight: "var(--space-2)", verticalAlign: "middle" }} />
        Priority Breakdown
      </h3>
      <div className="cost-tier-bars">
        {[1, 2, 3].map((priority) => {
          const data = stats.priority_breakdown.find((t) => t.priority === priority);
          const meta = PRIORITY_TIERS[priority] || { label: `Priority ${priority}`, desc: "", color: "var(--color-text-muted)" };
          const requests = data?.requests || 0;
          const cost = data?.estimated_cost || 0;
          const pct = maxTierRequests > 0 ? (requests / maxTierRequests) * 100 : 0;

          return (
            <div key={priority} className="cost-tier-row">
              <div className="cost-tier-label">
                <span className="cost-tier-dot" style={{ background: meta.color }} />
                <span>
                  <strong>{meta.label}</strong>
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
      <h3 className="cost-section-title">
        <Cpu size={16} style={{ display: "inline", marginRight: "var(--space-2)", verticalAlign: "middle" }} />
        Model Usage
      </h3>
      {Object.keys(stats.model_counts).length > 0 ? (
        <div className="cost-model-list">
          {Object.entries(stats.model_counts)
            .sort(([, a], [, b]) => b - a)
            .map(([model, count]) => {
              const maxCount = Math.max(...Object.values(stats.model_counts), 1);
              const pct = (count / maxCount) * 100;
              return (
                <div key={model} className="cost-model-bar-row">
                  <span className="cost-model-name mono" title={model}>{model}</span>
                  <div className="cost-model-bar-track">
                    <div
                      className="cost-model-bar-fill"
                      style={{ width: `${pct}%` }}
                    />
                  </div>
                  <span className="cost-model-count">{count} req</span>
                </div>
              );
            })}
        </div>
      ) : (
        <EmptyState
          icon={Bot}
          title="No model usage data yet"
          description="Model usage statistics will appear here once the gateway starts processing requests."
        />
      )}
    </section>
  );
}
