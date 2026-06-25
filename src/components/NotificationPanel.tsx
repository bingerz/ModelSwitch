import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Bell } from "lucide-react";
import { SectionHeader } from "./ui/SectionHeader";
import { api, type NotificationConfig } from "../lib/api";
import { useToast } from "./Toast";

export function NotificationPanel() {
  const { t } = useTranslation();
  const toast = useToast();
  const [config, setConfig] = useState<NotificationConfig | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const data = await api.notificationConfig();
      setConfig(data);
    } catch {
      // silently fail
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const handleSave = async () => {
    if (!config) return;
    setSaving(true);
    try {
      const updated = await api.updateNotification(config);
      setConfig(updated);
      toast.success(t("notifications.saved"));
    } catch {
      toast.error(t("notifications.saveFailed"));
    } finally {
      setSaving(false);
    }
  };

  if (loading) return <div className="panel-loading">{t("common.loading")}</div>;
  if (!config) return null;

  return (
    <section>
      <SectionHeader title={t("notifications.title")} icon={Bell} onRefresh={refresh} refreshing={loading} />

      {/* Webhook */}
      <div className="settings-section">
        <h3 className="settings-section-title">{t("notifications.webhook")}</h3>
        <div style={{ marginTop: "var(--space-3)" }}>
          <label className="settings-stat-label">{t("notifications.webhookUrl")}</label>
          <input
            type="url"
            className="settings-input"
            value={config.webhook_url ?? ""}
            onChange={(e) =>
              setConfig({ ...config, webhook_url: e.target.value || null })
            }
            placeholder="https://hooks.slack.com/services/..."
            style={{ width: "100%", maxWidth: "500px" }}
          />
        </div>
        <div style={{ marginTop: "var(--space-2)" }}>
          <label className="settings-stat-label">{t("notifications.webhookSecret")}</label>
          <input
            type="password"
            className="settings-input"
            value={config.webhook_secret ?? ""}
            onChange={(e) =>
              setConfig({ ...config, webhook_secret: e.target.value || null })
            }
            placeholder={t("notifications.webhookSecretPlaceholder")}
            style={{ width: "100%", maxWidth: "300px" }}
          />
        </div>
      </div>

      {/* Bark */}
      <div className="settings-section">
        <h3 className="settings-section-title">{t("notifications.bark")}</h3>
        <div style={{ marginTop: "var(--space-3)" }}>
          <label className="settings-stat-label">{t("notifications.barkUrl")}</label>
          <input
            type="url"
            className="settings-input"
            value={config.bark_url ?? ""}
            onChange={(e) =>
              setConfig({ ...config, bark_url: e.target.value || null })
            }
            placeholder="https://api.day.app/yourkey"
            style={{ width: "100%", maxWidth: "400px" }}
          />
        </div>
      </div>

      {/* Budget Threshold */}
      <div className="settings-section">
        <h3 className="settings-section-title">{t("notifications.budgetThreshold")}</h3>
        <div style={{ marginTop: "var(--space-3)" }}>
          <label className="settings-stat-label">{t("notifications.budgetThresholdHint")}</label>
          <input
            type="number"
            min="1"
            max="100"
            className="settings-input"
            value={config.budget_threshold_pct}
            onChange={(e) =>
              setConfig({ ...config, budget_threshold_pct: parseInt(e.target.value, 10) || 80 })
            }
            style={{ width: "100%", maxWidth: "100px" }}
          />
          <span style={{ marginLeft: "var(--space-2)", fontSize: "var(--text-sm)" }}>%</span>
        </div>
      </div>

      <div className="settings-actions">
        <button className="btn btn-primary" onClick={handleSave} disabled={saving}>
          {saving ? t("common.saving") : t("common.save")}
        </button>
      </div>
    </section>
  );
}
