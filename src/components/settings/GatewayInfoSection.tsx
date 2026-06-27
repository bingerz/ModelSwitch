import { useTranslation } from "react-i18next";
import { Server } from "lucide-react";
import type { GatewayInfo } from "../../lib/api";

interface GatewayInfoSectionProps {
  gatewayInfo: GatewayInfo | null;
}

/** Live gateway snapshot — version, uptime, channel health, routing. */
export function GatewayInfoSection({ gatewayInfo }: GatewayInfoSectionProps) {
  const { t } = useTranslation();

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
          <span className="settings-stat-value">
            {gatewayInfo.routing_strategy}
          </span>
        </div>
        <div className="settings-stat">
          <span className="settings-stat-label">{t("dashboard.maxRetries")}</span>
          <span className="settings-stat-value mono">
            {gatewayInfo.max_retries}
          </span>
        </div>
      </div>
    </div>
  );
}
