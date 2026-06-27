import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Settings, Server, Database, RotateCcw, Wallet, FlaskConical } from "lucide-react";
import { SectionHeader } from "./ui/SectionHeader";
import { ProgressBar } from "./ui/ProgressBar";
import { api, type CacheStats, type GatewayInfo, type ProviderBudgetEntry } from "../lib/api";
import { formatCents } from "../lib/format";
import { isMockMode, setMockMode } from "../lib/mock";
import { useToast } from "./Toast";

interface BudgetFormState {
  provider: string;
  daily: string;
  monthly: string;
}

const EMPTY_BUDGET_FORM: BudgetFormState = { provider: "", daily: "", monthly: "" };

interface SettingsData {
  cache: CacheStats | null;
  info: GatewayInfo | null;
  budgets: ProviderBudgetEntry[];
  ratios: Record<string, number>;
}

export function SettingsPanel() {
  const { t } = useTranslation();
  const toast = useToast();
  const [flushing, setFlushing] = useState(false);
  const [reloading, setReloading] = useState(false);
  const [completionRatios, setCompletionRatios] = useState<Record<string, number>>({});
  const [newRatioModel, setNewRatioModel] = useState("");
  const [newRatioValue, setNewRatioValue] = useState("");
  const [savingRatios, setSavingRatios] = useState(false);
  const [showBudgetForm, setShowBudgetForm] = useState(false);
  const [budgetForm, setBudgetForm] = useState<BudgetFormState>(EMPTY_BUDGET_FORM);
  const [budgetSaving, setBudgetSaving] = useState(false);
  const [deletingBudget, setDeletingBudget] = useState<string | null>(null);

  const { data, isLoading: loading, refetch } = useQuery({
    queryKey: ["settings"],
    queryFn: async (): Promise<SettingsData> => {
      const [cache, info, budgetData] = await Promise.all([
        api.cacheStats().catch(() => null),
        api.gatewayInfo().catch(() => null),
        api.providerBudgets().catch(() => []),
      ]);
      let ratios: Record<string, number> = {};
      try {
        ratios = await api.completionRatios();
      } catch {
        // ratios not available yet
      }
      return { cache, info, budgets: budgetData, ratios };
    },
    refetchInterval: 10_000,
    // Silently fail — StatusBar shows gateway status
    retry: false,
  });

  const cacheStats = data?.cache ?? null;
  const gatewayInfo = data?.info ?? null;
  const budgets = data?.budgets ?? [];
  // Sync completion ratios from server into local editable state on each poll.
  // Local state is needed because the UI lets users edit ratios before saving.
  useEffect(() => {
    if (data) {
      setCompletionRatios(data.ratios);
    }
  }, [data]);

  const refresh = async () => {
    await refetch();
  };

  const handleFlushCache = async () => {
    setFlushing(true);
    try {
      await api.flushCache();
      toast.success(t("settings.cacheFlushed"));
      refresh();
    } catch {
      toast.error(t("settings.cacheFlushFailed"));
    } finally {
      setFlushing(false);
    }
  };

  const handleReloadConfig = async () => {
    setReloading(true);
    try {
      const result = await api.reloadConfig();
      toast.success(t("settings.reloadSuccess", { created: result.created, updated: result.updated, removed: result.removed }));
      refresh();
    } catch {
      toast.error(t("settings.reloadFailed"));
    } finally {
      setReloading(false);
    }
  };

  const resetBudgetForm = () => {
    setBudgetForm(EMPTY_BUDGET_FORM);
    setShowBudgetForm(false);
  };

  const handleSaveBudget = async () => {
    const provider = budgetForm.provider.trim();
    if (!provider) {
      toast.error(t("settings.providerNameRequired"));
      return;
    }
    const daily = budgetForm.daily.trim() === "" ? null : Number(budgetForm.daily);
    const monthly = budgetForm.monthly.trim() === "" ? null : Number(budgetForm.monthly);
    if (daily != null && (!Number.isFinite(daily) || daily < 0)) {
      toast.error(t("settings.budgetInvalid"));
      return;
    }
    if (monthly != null && (!Number.isFinite(monthly) || monthly < 0)) {
      toast.error(t("settings.budgetInvalid"));
      return;
    }
    if (daily != null && monthly != null && monthly < daily) {
      toast.error(t("settings.monthlyBelowDaily"));
      return;
    }
    setBudgetSaving(true);
    try {
      await api.setProviderBudget(provider, {
        daily_budget_cents: daily,
        monthly_budget_cents: monthly,
      });
      toast.success(t("settings.budgetSaved"));
      resetBudgetForm();
      await refresh();
    } catch {
      toast.error(t("settings.budgetSaveFailed"));
    } finally {
      setBudgetSaving(false);
    }
  };

  const handleDeleteBudget = async (provider: string) => {
    setDeletingBudget(provider);
    try {
      await api.deleteProviderBudget(provider);
      toast.success(t("settings.budgetDeleted"));
      await refresh();
    } catch {
      toast.error(t("settings.budgetDeleteFailed"));
    } finally {
      setDeletingBudget(null);
    }
  };

  if (loading) return <div className="panel-loading">{t("settings.loading")}</div>;

  return (
    <section>
      <SectionHeader title={t("settings.title")} icon={Settings} onRefresh={refresh} refreshing={false} />

      {/* Demo Mode */}
      <div className="settings-section">
        <h3 className="settings-section-title">
          <FlaskConical size={14} className="icon-inline" />
          {t("settings.demoMode")}
        </h3>
        <p className="settings-hint">
          {t("settings.demoModeHint")}
        </p>
        <div className="settings-actions">
          <label style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", cursor: "pointer", fontSize: "var(--text-sm)" }}>
            <input
              type="checkbox"
              checked={isMockMode()}
              onChange={(e) => setMockMode(e.target.checked)}
              style={{ width: 16, height: 16, cursor: "pointer" }}
            />
            {t("settings.enableDemoMode")}
          </label>
        </div>
      </div>

      {/* Gateway Info */}
      {gatewayInfo && (
        <div className="settings-section">
          <h3 className="settings-section-title">
            <Server size={14} className="icon-inline" />
            {t("settings.gatewayInfo")}
          </h3>
          <div className="settings-stats-grid">
            <div className="settings-stat">
              <span className="settings-stat-label">{t("dashboard.version")}</span>
              <span className="settings-stat-value mono">v{gatewayInfo.version}</span>
            </div>
            <div className="settings-stat">
              <span className="settings-stat-label">{t("dashboard.uptime")}</span>
              <span className="settings-stat-value">{gatewayInfo.uptime_formatted}</span>
            </div>
            <div className="settings-stat">
              <span className="settings-stat-label">{t("dashboard.activeChannels")}</span>
              <span className="settings-stat-value">
                {t("settings.channelsHealthy", { healthy: gatewayInfo.healthy_channels, total: gatewayInfo.total_channels })}
              </span>
            </div>
            <div className="settings-stat">
              <span className="settings-stat-label">{t("dashboard.activeRequests")}</span>
              <span className="settings-stat-value mono">{gatewayInfo.active_requests}</span>
            </div>
            <div className="settings-stat">
              <span className="settings-stat-label">{t("dashboard.routingStrategy")}</span>
              <span className="settings-stat-value">{gatewayInfo.routing_strategy}</span>
            </div>
            <div className="settings-stat">
              <span className="settings-stat-label">{t("dashboard.maxRetries")}</span>
              <span className="settings-stat-value mono">{gatewayInfo.max_retries}</span>
            </div>
          </div>
        </div>
      )}

      {/* Cache Management */}
      {cacheStats && (
        <div className="settings-section">
          <h3 className="settings-section-title">
            <Database size={14} className="icon-inline" />
            {t("settings.cacheManagement")}
          </h3>
          <div className="settings-stats-grid">
            <div className="settings-stat">
              <span className="settings-stat-label">{t("common.mode")}</span>
              <span className="settings-stat-value">{cacheStats.mode}</span>
            </div>
            <div className="settings-stat">
              <span className="settings-stat-label">{t("settings.entries")}</span>
              <span className="settings-stat-value mono">{cacheStats.entries}</span>
            </div>
            <div className="settings-stat">
              <span className="settings-stat-label">{t("settings.hitRate")}</span>
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
              <span className="settings-stat-label">{t("settings.hitsMisses")}</span>
              <span className="settings-stat-value mono">
                {cacheStats.hits} / {cacheStats.misses}
              </span>
            </div>
            <div className="settings-stat">
              <span className="settings-stat-label">{t("settings.evictions")}</span>
              <span className="settings-stat-value mono">{cacheStats.evictions}</span>
            </div>
          </div>
          <div className="settings-actions">
            <button
              className="btn btn-sm"
              onClick={handleFlushCache}
              disabled={flushing || cacheStats.entries === 0}
            >
              {flushing ? t("settings.flushing") : t("settings.flushCache")}
            </button>
          </div>
        </div>
      )}

      {/* Config Reload */}
      <div className="settings-section">
        <h3 className="settings-section-title">
          <RotateCcw size={14} className="icon-inline" />
          {t("settings.config")}
        </h3>
        <p className="settings-hint">
          {t("settings.configHint")}
        </p>
        <div className="settings-actions">
          <button
            className="btn btn-sm btn-primary"
            onClick={handleReloadConfig}
            disabled={reloading}
          >
            {reloading ? t("settings.reloading") : t("settings.reloadConfig")}
          </button>
        </div>
      </div>

      {/* Provider Budgets */}
      <div className="settings-section">
        <h3 className="settings-section-title">
          <Wallet size={14} className="icon-inline" />
          {t("settings.providerBudgets")}
        </h3>
        {budgets.length > 0 && (
          <div className="settings-table-wrapper">
            <table className="settings-table">
              <thead>
                <tr>
                  <th>{t("common.provider")}</th>
                  <th>{t("settings.today")}</th>
                  <th>{t("settings.dailyBudget")}</th>
                  <th>{t("settings.thisMonthColumn")}</th>
                  <th>{t("settings.monthlyBudget")}</th>
                  <th>{t("settings.total")}</th>
                  <th aria-label="actions" />
                </tr>
              </thead>
              <tbody>
                {budgets.map((b) => (
                  <tr key={b.provider}>
                    <td className="mono">{b.provider}</td>
                    <td className="mono">{formatCents(b.spend.today.cents)}</td>
                    <td className="mono">
                      {b.daily_budget_cents != null ? formatCents(b.daily_budget_cents) : "\u221e"}
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
                      {b.monthly_budget_cents != null ? formatCents(b.monthly_budget_cents) : "\u221e"}
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
                    <td>
                      <button
                        className="btn btn-sm"
                        style={{ color: "var(--color-danger)" }}
                        onClick={() => handleDeleteBudget(b.provider)}
                        disabled={deletingBudget === b.provider}
                      >
                        {deletingBudget === b.provider ? t("common.deleting") : t("settings.deleteBudget")}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        {showBudgetForm ? (
          <div style={{ display: "flex", gap: "var(--space-2)", alignItems: "flex-end", flexWrap: "wrap", marginTop: "var(--space-2)" }}>
            <div>
              <label className="settings-stat-label">{t("settings.providerName")}</label>
              <input
                type="text"
                className="settings-input"
                value={budgetForm.provider}
                onChange={(e) => setBudgetForm((prev) => ({ ...prev, provider: e.target.value }))}
                placeholder="e.g. openai"
                style={{ width: "150px" }}
              />
            </div>
            <div>
              <label className="settings-stat-label">{t("settings.budgetDailyCents")}</label>
              <input
                type="number"
                className="settings-input"
                value={budgetForm.daily}
                onChange={(e) => setBudgetForm((prev) => ({ ...prev, daily: e.target.value }))}
                placeholder="e.g. 500"
                min="0"
                step="1"
                style={{ width: "120px" }}
              />
            </div>
            <div>
              <label className="settings-stat-label">{t("settings.budgetMonthlyCents")}</label>
              <input
                type="number"
                className="settings-input"
                value={budgetForm.monthly}
                onChange={(e) => setBudgetForm((prev) => ({ ...prev, monthly: e.target.value }))}
                placeholder="e.g. 15000"
                min="0"
                step="1"
                style={{ width: "120px" }}
              />
            </div>
            <button
              className="btn btn-sm btn-primary"
              onClick={handleSaveBudget}
              disabled={budgetSaving}
            >
              {budgetSaving ? t("common.saving") : t("common.save")}
            </button>
            <button
              className="btn btn-sm"
              onClick={resetBudgetForm}
              disabled={budgetSaving}
            >
              {t("common.cancel")}
            </button>
          </div>
        ) : (
          <div className="settings-actions">
            <button className="btn btn-sm" onClick={() => setShowBudgetForm(true)}>
              {t("settings.addBudget")}
            </button>
          </div>
        )}
      </div>
      {/* Completion Ratios */}
      <div className="settings-section">
        <h3 className="settings-section-title">
          {t("settings.completionRatios")}
        </h3>
        <p className="settings-hint">{t("settings.completionRatiosHint")}</p>
        {Object.keys(completionRatios).length > 0 && (
          <div className="settings-table-wrapper mb-2">
            <table className="settings-table">
              <thead>
                <tr>
                  <th>Model</th>
                  <th>Ratio</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                {Object.entries(completionRatios).map(([model, ratio]) => (
                  <tr key={model}>
                    <td className="mono">{model}</td>
                    <td className="mono">{ratio.toFixed(2)}x</td>
                    <td>
                      <button
                        className="btn btn-sm"
                        onClick={() => {
                          const next = { ...completionRatios };
                          delete next[model];
                          setCompletionRatios(next);
                        }}
                      >
                        {t("common.remove")}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        <div style={{ display: "flex", gap: "var(--space-2)", alignItems: "flex-end" }}>
          <div>
            <label className="settings-stat-label">Model</label>
            <input
              type="text"
              className="settings-input"
              value={newRatioModel}
              onChange={(e) => setNewRatioModel(e.target.value)}
              placeholder="e.g. gpt-4o"
              style={{ width: "150px" }}
            />
          </div>
          <div>
            <label className="settings-stat-label">Ratio</label>
            <input
              type="number"
              className="settings-input"
              value={newRatioValue}
              onChange={(e) => setNewRatioValue(e.target.value)}
              placeholder="e.g. 2.0"
              step="0.1"
              min="0.1"
              style={{ width: "100px" }}
            />
          </div>
          <button
            className="btn btn-sm"
            onClick={() => {
              const model = newRatioModel.trim();
              const ratio = parseFloat(newRatioValue);
              if (model && !isNaN(ratio) && ratio > 0) {
                setCompletionRatios({ ...completionRatios, [model]: ratio });
                setNewRatioModel("");
                setNewRatioValue("");
              }
            }}
          >
            {t("common.add")}
          </button>
          <button
            className="btn btn-sm btn-primary"
            disabled={savingRatios}
            onClick={async () => {
              setSavingRatios(true);
              try {
                const updated = await api.updateCompletionRatios(completionRatios);
                setCompletionRatios(updated);
                toast.success(t("settings.completionRatiosSaved"));
              } catch {
                toast.error(t("settings.completionRatiosSaveFailed"));
              } finally {
                setSavingRatios(false);
              }
            }}
          >
            {savingRatios ? t("common.saving") : t("common.save")}
          </button>
        </div>
      </div>

      {/* Quick Links to Feature Panels */}
      <div className="settings-section">
        <h3 className="settings-section-title">{t("settings.advancedFeatures")}</h3>
        <p className="settings-hint">{t("settings.advancedFeaturesHint")}</p>
      </div>
    </section>
  );
}
