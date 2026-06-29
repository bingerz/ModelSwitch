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

/** Normalize the backend cache mode string (e.g. "On", "ReadOnly") to the
 *  lowercase API value used in the <select> options. */
function normalizeCacheMode(mode: string): string {
  const lower = mode.toLowerCase();
  if (lower === "readonly" || lower === "read-only") return "readonly";
  if (lower === "writeonly" || lower === "write-only") return "writeonly";
  if (lower === "off" || lower === "disabled") return "off";
  return "on";
}

/** Cache stats dashboard and flush control. */
export function CacheManagementSection({
  cacheStats,
  onRefresh,
}: CacheManagementSectionProps) {
  const { t } = useTranslation();
  const toast = useToast();
  const [flushing, setFlushing] = useState(false);
  const [modeUpdating, setModeUpdating] = useState(false);

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

  const handleModeChange = async (mode: string) => {
    setModeUpdating(true);
    try {
      await api.updateCacheMode(mode);
      toast.success(t("settings.cacheModeUpdated"));
      await onRefresh();
    } catch {
      toast.error(t("settings.cacheModeUpdateFailed"));
    } finally {
      setModeUpdating(false);
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
          <select
            className="settings-stat-value"
            value={normalizeCacheMode(cacheStats.mode)}
            disabled={modeUpdating}
            onChange={(e) => handleModeChange(e.target.value)}
            style={{ cursor: modeUpdating ? "wait" : "pointer" }}
          >
            <option value="on">{t("settings.cacheModeOn")}</option>
            <option value="off">{t("settings.cacheModeOff")}</option>
            <option value="readonly">{t("settings.cacheModeReadOnly")}</option>
            <option value="writeonly">{t("settings.cacheModeWriteOnly")}</option>
          </select>
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
