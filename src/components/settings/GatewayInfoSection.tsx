import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Info, Server } from "lucide-react";
import { api, type GatewayInfo } from "../../lib/api";
import { useToast } from "../Toast";

interface GatewayInfoSectionProps {
  gatewayInfo: GatewayInfo | null;
  onRefresh?: () => Promise<void>;
}

const STRATEGY_OPTIONS = [
  { value: "weighted_random", labelKey: "settings.strategyWeightedRandom", descKey: "settings.strategyWeightedRandomDesc" },
  { value: "latency", labelKey: "settings.strategyLatency", descKey: "settings.strategyLatencyDesc" },
  { value: "least_busy", labelKey: "settings.strategyLeastBusy", descKey: "settings.strategyLeastBusyDesc" },
  { value: "usage", labelKey: "settings.strategyUsage", descKey: "settings.strategyUsageDesc" },
  { value: "lowest_cost", labelKey: "settings.strategyLowestCost", descKey: "settings.strategyLowestCostDesc" },
] as const;

/** Live gateway snapshot — version, uptime, channel health, routing. */
export function GatewayInfoSection({
  gatewayInfo,
  onRefresh,
}: GatewayInfoSectionProps) {
  const { t } = useTranslation();
  const toast = useToast();
  const [saving, setSaving] = useState(false);

  const handleStrategyChange = async (
    e: React.ChangeEvent<HTMLSelectElement>,
  ) => {
    const newStrategy = e.target.value;
    if (newStrategy === gatewayInfo?.routing_strategy) return;
    setSaving(true);
    try {
      await api.updateRoutingStrategy(newStrategy);
      toast.success(t("settings.routingStrategyUpdated"));
      await onRefresh?.();
    } catch {
      toast.error(t("settings.routingStrategyUpdateFailed"));
    } finally {
      setSaving(false);
    }
  };

  if (!gatewayInfo) return null;

  return (
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
          <span className="settings-stat-value">
            {gatewayInfo.uptime_formatted}
          </span>
        </div>
        <div className="settings-stat">
          <span className="settings-stat-label">
            {t("dashboard.activeChannels")}
          </span>
          <span className="settings-stat-value">
            {t("settings.channelsHealthy", {
              healthy: gatewayInfo.healthy_channels,
              total: gatewayInfo.total_channels,
            })}
          </span>
        </div>
        <div className="settings-stat">
          <span className="settings-stat-label">
            {t("dashboard.activeRequests")}
          </span>
          <span className="settings-stat-value mono">
            {gatewayInfo.active_requests}
          </span>
        </div>
        <div className="settings-stat">
          <span className="settings-stat-label">
            {t("dashboard.routingStrategy")}
          </span>
          <span className="settings-stat-value" style={{ padding: 0 }}>
            <select
              className="settings-select"
              value={gatewayInfo.routing_strategy}
              onChange={handleStrategyChange}
              disabled={saving}
              aria-label={t("dashboard.routingStrategy")}
            >
              {STRATEGY_OPTIONS.map((opt) => (
                <option key={opt.value} value={opt.value}>
                  {t(opt.labelKey)}
                </option>
              ))}
            </select>
          </span>
          <span className="settings-hint" style={{ marginTop: "var(--space-1)" }}>
            {t(STRATEGY_OPTIONS.find((o) => o.value === gatewayInfo.routing_strategy)?.descKey ?? "")}
          </span>
        </div>
        <div className="settings-stat">
          <span className="settings-stat-label">{t("dashboard.maxRetries")}</span>
          <span className="settings-stat-value mono">
            {gatewayInfo.max_retries}
          </span>
        </div>
      </div>
      <div className="backup-info" style={{ marginTop: "var(--space-3)" }}>
        <Info size={12} className="icon-inline" />
        {t("settings.singleInstanceNote")}
      </div>
    </div>
  );
}
