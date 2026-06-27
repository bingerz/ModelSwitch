import { useState } from "react";
import { useTranslation } from "react-i18next";
import { FlaskConical, RotateCcw } from "lucide-react";
import { api } from "../../lib/api";
import { isMockMode, setMockMode } from "../../lib/mock-flag";
import { useToast } from "../Toast";

interface GeneralSettingsProps {
  onRefresh: () => Promise<void>;
}

/**
 * Demo Mode + Config Reload — top-of-panel controls that don't fit a more
 * specific domain section.
 */
export function GeneralSettings({ onRefresh }: GeneralSettingsProps) {
  const { t } = useTranslation();
  const toast = useToast();
  const [reloading, setReloading] = useState(false);

  const handleReloadConfig = async () => {
    setReloading(true);
    try {
      const result = await api.reloadConfig();
      toast.success(
        t("settings.reloadSuccess", {
          created: result.created,
          updated: result.updated,
          removed: result.removed,
        }),
      );
      await onRefresh();
    } catch {
      toast.error(t("settings.reloadFailed"));
    } finally {
      setReloading(false);
    }
  };

  return (
    <>
      {/* Demo Mode */}
      <div className="settings-section">
        <h3 className="settings-section-title">
          <FlaskConical size={14} className="icon-inline" />
          {t("settings.demoMode")}
        </h3>
        <p className="settings-hint">{t("settings.demoModeHint")}</p>
        <div className="settings-actions">
          <label
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--space-2)",
              cursor: "pointer",
              fontSize: "var(--text-sm)",
            }}
          >
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

      {/* Config Reload */}
      <div className="settings-section">
        <h3 className="settings-section-title">
          <RotateCcw size={14} className="icon-inline" />
          {t("settings.config")}
        </h3>
        <p className="settings-hint">{t("settings.configHint")}</p>
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
    </>
  );
}
