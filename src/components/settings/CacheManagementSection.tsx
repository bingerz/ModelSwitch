import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Database } from "lucide-react";
import { api, type CacheStats } from "../../lib/api";
import { useToast } from "../Toast";
import { ProgressBar } from "../ui/ProgressBar";

interface CacheManagementSectionProps {
  cacheStats: CacheStats | null;
  onRefresh: () => Promise<void>;
}

/** Cache stats dashboard and flush control. */
export function CacheManagementSection({
  cacheStats,
  onRefresh,
}: CacheManagementSectionProps) {
  const { t } = useTranslation();
  const toast = useToast();
  const [flushing, setFlushing] = useState(false);

  if (!cacheStats) return null;

  const handleFlushCache = async () => {
    setFlushing(true);
    try {
      await api.flushCache();
      toast.success(t("settings.cacheFlushed"));
      await onRefresh();
    } catch {
      toast.error(t("settings.cacheFlushFailed"));
    } finally {
      setFlushing(false);
    }
  };

  return (
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
            <span
              style={{
                color:
                  cacheStats.hit_rate_percent > 30
                    ? "var(--color-success)"
                    : "var(--color-text-secondary)",
              }}
            >
              {cacheStats.hit_rate_percent}%
            </span>
          </span>
          <div style={{ marginTop: "4px", maxWidth: "100px" }}>
            <ProgressBar
              value={cacheStats.hit_rate_percent}
              height={4}
              color={
                cacheStats.hit_rate_percent > 30
                  ? "var(--color-success)"
                  : "var(--color-text-muted)"
              }
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
  );
}
