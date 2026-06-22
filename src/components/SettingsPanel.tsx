import { useEffect, useState, useCallback } from "react";
import { Settings, Server, Database, RotateCcw, Wallet, FlaskConical } from "lucide-react";
import { SectionHeader } from "./ui/SectionHeader";
import { ProgressBar } from "./ui/ProgressBar";
import { api, type CacheStats, type GatewayInfo, type ProviderBudgetEntry } from "../lib/api";
import { isMockMode, setMockMode } from "../lib/mock";
import { useToast } from "./Toast";

/** Format cents to dollar display */
function formatCents(cents: number): string {
  return `$${(cents / 100).toFixed(2)}`;
}

export function SettingsPanel() {
  const toast = useToast();
  const [cacheStats, setCacheStats] = useState<CacheStats | null>(null);
  const [gatewayInfo, setGatewayInfo] = useState<GatewayInfo | null>(null);
  const [budgets, setBudgets] = useState<ProviderBudgetEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [flushing, setFlushing] = useState(false);
  const [reloading, setReloading] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [cache, info, budgetData] = await Promise.all([
        api.cacheStats().catch(() => null),
        api.gatewayInfo().catch(() => null),
        api.providerBudgets().catch(() => []),
      ]);
      setCacheStats(cache);
      setGatewayInfo(info);
      setBudgets(budgetData);
    } catch {
      // Silently fail — StatusBar shows gateway status
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
    const interval = setInterval(refresh, 10000);
    return () => clearInterval(interval);
  }, [refresh]);

  const handleFlushCache = async () => {
    setFlushing(true);
    try {
      await api.flushCache();
      toast.success("Cache flushed");
      refresh();
    } catch {
      toast.error("Failed to flush cache");
    } finally {
      setFlushing(false);
    }
  };

  const handleReloadConfig = async () => {
    setReloading(true);
    try {
      const result = await api.reloadConfig();
      toast.success(`Config reloaded: ${result.created} created, ${result.updated} updated, ${result.removed} removed`);
      refresh();
    } catch {
      toast.error("Failed to reload config");
    } finally {
      setReloading(false);
    }
  };

  if (loading) return <div className="panel-loading">Loading settings...</div>;

  return (
    <section>
      <SectionHeader title="Settings" icon={Settings} onRefresh={refresh} refreshing={false} />

      {/* Demo Mode */}
      <div className="settings-section">
        <h3 className="settings-section-title">
          <FlaskConical size={14} style={{ display: "inline", marginRight: "var(--space-2)", verticalAlign: "middle" }} />
          Demo Mode
        </h3>
        <p className="settings-hint">
          Uses simulated data for all API calls. Reloads the page when toggled.
        </p>
        <div className="settings-actions">
          <label style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", cursor: "pointer", fontSize: "var(--text-sm)" }}>
            <input
              type="checkbox"
              checked={isMockMode()}
              onChange={(e) => setMockMode(e.target.checked)}
              style={{ width: 16, height: 16, cursor: "pointer" }}
            />
            Enable Demo Mode
          </label>
        </div>
      </div>

      {/* Gateway Info */}
      {gatewayInfo && (
        <div className="settings-section">
          <h3 className="settings-section-title">
            <Server size={14} style={{ display: "inline", marginRight: "var(--space-2)", verticalAlign: "middle" }} />
            Gateway Info
          </h3>
          <div className="settings-stats-grid">
            <div className="settings-stat">
              <span className="settings-stat-label">Version</span>
              <span className="settings-stat-value mono">v{gatewayInfo.version}</span>
            </div>
            <div className="settings-stat">
              <span className="settings-stat-label">Uptime</span>
              <span className="settings-stat-value">{gatewayInfo.uptime_formatted}</span>
            </div>
            <div className="settings-stat">
              <span className="settings-stat-label">Channels</span>
              <span className="settings-stat-value">
                {gatewayInfo.healthy_channels} / {gatewayInfo.total_channels} healthy
              </span>
            </div>
            <div className="settings-stat">
              <span className="settings-stat-label">Active Requests</span>
              <span className="settings-stat-value mono">{gatewayInfo.active_requests}</span>
            </div>
            <div className="settings-stat">
              <span className="settings-stat-label">Routing Strategy</span>
              <span className="settings-stat-value">{gatewayInfo.routing_strategy}</span>
            </div>
            <div className="settings-stat">
              <span className="settings-stat-label">Max Retries</span>
              <span className="settings-stat-value mono">{gatewayInfo.max_retries}</span>
            </div>
          </div>
        </div>
      )}

      {/* Cache Management */}
      {cacheStats && (
        <div className="settings-section">
          <h3 className="settings-section-title">
            <Database size={14} style={{ display: "inline", marginRight: "var(--space-2)", verticalAlign: "middle" }} />
            Cache Management
          </h3>
          <div className="settings-stats-grid">
            <div className="settings-stat">
              <span className="settings-stat-label">Mode</span>
              <span className="settings-stat-value">{cacheStats.mode}</span>
            </div>
            <div className="settings-stat">
              <span className="settings-stat-label">Entries</span>
              <span className="settings-stat-value mono">{cacheStats.entries}</span>
            </div>
            <div className="settings-stat">
              <span className="settings-stat-label">Hit Rate</span>
              <span className="settings-stat-value mono">
                <span style={{ color: cacheStats.hit_rate_percent > 30 ? "var(--color-success)" : "var(--color-text-secondary)" }}>
                  {cacheStats.hit_rate_percent}%
                </span>
              </span>
              <div style={{ marginTop: "4px", maxWidth: "100px" }}>
                <ProgressBar
                  value={cacheStats.hit_rate_percent}
                  height={4}
                  color={cacheStats.hit_rate_percent > 30 ? "var(--color-success)" : "var(--color-text-muted)"}
                />
              </div>
            </div>
            <div className="settings-stat">
              <span className="settings-stat-label">Hits / Misses</span>
              <span className="settings-stat-value mono">
                {cacheStats.hits} / {cacheStats.misses}
              </span>
            </div>
            <div className="settings-stat">
              <span className="settings-stat-label">Evictions</span>
              <span className="settings-stat-value mono">{cacheStats.evictions}</span>
            </div>
          </div>
          <div className="settings-actions">
            <button
              className="btn btn-sm"
              onClick={handleFlushCache}
              disabled={flushing || cacheStats.entries === 0}
            >
              {flushing ? "Flushing..." : "Flush Cache"}
            </button>
          </div>
        </div>
      )}

      {/* Config Reload */}
      <div className="settings-section">
        <h3 className="settings-section-title">
          <RotateCcw size={14} style={{ display: "inline", marginRight: "var(--space-2)", verticalAlign: "middle" }} />
          Configuration
        </h3>
        <p className="settings-hint">
          Manually trigger a hot-reload of <code className="settings-code">config.toml</code>.
          Channels, rate limits, payload rules, and MCP servers will be updated.
        </p>
        <div className="settings-actions">
          <button
            className="btn btn-sm btn-primary"
            onClick={handleReloadConfig}
            disabled={reloading}
          >
            {reloading ? "Reloading..." : "Reload Config"}
          </button>
        </div>
      </div>

      {/* Provider Budgets */}
      {budgets.length > 0 && (
        <div className="settings-section">
          <h3 className="settings-section-title">
            <Wallet size={14} style={{ display: "inline", marginRight: "var(--space-2)", verticalAlign: "middle" }} />
            Provider Budgets
          </h3>
          <div className="settings-table-wrapper">
            <table className="settings-table">
              <thead>
                <tr>
                  <th>Provider</th>
                  <th>Today</th>
                  <th>Daily Budget</th>
                  <th>This Month</th>
                  <th>Monthly Budget</th>
                  <th>Total</th>
                </tr>
              </thead>
              <tbody>
                {budgets.map((b) => (
                  <tr key={b.provider}>
                    <td className="mono">{b.provider}</td>
                    <td className="mono">{formatCents(b.spend.today.cents)}</td>
                    <td className="mono">
                      {b.daily_budget_cents != null ? formatCents(b.daily_budget_cents) : "∞"}
                      {b.daily_budget_cents != null && (
                        <div style={{ marginTop: "4px", maxWidth: "120px" }}>
                          <ProgressBar
                            value={b.spend.today.cents}
                            max={b.daily_budget_cents}
                            thresholds={[
                              { upto: 50, color: "var(--color-success)" },
                              { upto: 80, color: "var(--color-warning)" },
                              { upto: 100, color: "var(--color-danger)" },
                            ]}
                            height={4}
                          />
                        </div>
                      )}
                    </td>
                    <td className="mono">{formatCents(b.spend.this_month.cents)}</td>
                    <td className="mono">
                      {b.monthly_budget_cents != null ? formatCents(b.monthly_budget_cents) : "∞"}
                      {b.monthly_budget_cents != null && (
                        <div style={{ marginTop: "4px", maxWidth: "120px" }}>
                          <ProgressBar
                            value={b.spend.this_month.cents}
                            max={b.monthly_budget_cents}
                            thresholds={[
                              { upto: 50, color: "var(--color-success)" },
                              { upto: 80, color: "var(--color-warning)" },
                              { upto: 100, color: "var(--color-danger)" },
                            ]}
                            height={4}
                          />
                        </div>
                      )}
                    </td>
                    <td className="mono">{formatCents(b.spend.total_cents)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </section>
  );
}
