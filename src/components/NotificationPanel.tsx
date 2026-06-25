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
        <div className="settings-actions">
          <label style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", cursor: "pointer", fontSize: "var(--text-sm)" }}>
            <input
              type="checkbox"
              checked={config.webhook.enabled}
              onChange={(e) =>
                setConfig({ ...config, webhook: { ...config.webhook, enabled: e.target.checked } })
              }
              style={{ width: 16, height: 16, cursor: "pointer" }}
            />
            {t("notifications.enableWebhook")}
          </label>
        </div>
        {config.webhook.enabled && (
          <>
            <div style={{ marginTop: "var(--space-3)" }}>
              <label className="settings-stat-label">{t("notifications.webhookUrl")}</label>
              <input
                type="url"
                className="settings-input"
                value={config.webhook.url ?? ""}
                onChange={(e) =>
                  setConfig({ ...config, webhook: { ...config.webhook, url: e.target.value || null } })
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
                value={config.webhook.secret ?? ""}
                onChange={(e) =>
                  setConfig({ ...config, webhook: { ...config.webhook, secret: e.target.value || null } })
                }
                placeholder="HMAC-SHA256 signing secret"
                style={{ width: "100%", maxWidth: "300px" }}
              />
            </div>
          </>
        )}
      </div>

      {/* Bark */}
      <div className="settings-section">
        <h3 className="settings-section-title">{t("notifications.bark")}</h3>
        <div className="settings-actions">
          <label style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", cursor: "pointer", fontSize: "var(--text-sm)" }}>
            <input
              type="checkbox"
              checked={config.bark.enabled}
              onChange={(e) =>
                setConfig({ ...config, bark: { ...config.bark, enabled: e.target.checked } })
              }
              style={{ width: 16, height: 16, cursor: "pointer" }}
            />
            {t("notifications.enableBark")}
          </label>
        </div>
        {config.bark.enabled && (
          <>
            <div style={{ marginTop: "var(--space-3)" }}>
              <label className="settings-stat-label">{t("notifications.barkUrl")}</label>
              <input
                type="url"
                className="settings-input"
                value={config.bark.url ?? ""}
                onChange={(e) =>
                  setConfig({ ...config, bark: { ...config.bark, url: e.target.value || null } })
                }
                placeholder="https://api.day.app/yourkey"
                style={{ width: "100%", maxWidth: "400px" }}
              />
            </div>
            <div style={{ marginTop: "var(--space-2)" }}>
              <label className="settings-stat-label">{t("notifications.barkKey")}</label>
              <input
                type="text"
                className="settings-input"
                value={config.bark.key ?? ""}
                onChange={(e) =>
                  setConfig({ ...config, bark: { ...config.bark, key: e.target.value || null } })
                }
                placeholder="Bark device key"
                style={{ width: "100%", maxWidth: "300px" }}
              />
            </div>
          </>
        )}
      </div>

      {/* Events */}
      <div className="settings-section">
        <h3 className="settings-section-title">{t("notifications.events")}</h3>
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
          {([
            ["channel_failure", t("notifications.eventChannelFailure")],
            ["quota_warning", t("notifications.eventQuotaWarning")],
            ["cooldown_triggered", t("notifications.eventCooldown")],
          ] as const).map(([key, label]) => (
            <label
              key={key}
              style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", cursor: "pointer", fontSize: "var(--text-sm)" }}
            >
              <input
                type="checkbox"
                checked={config.events[key]}
                onChange={(e) =>
                  setConfig({
                    ...config,
                    events: { ...config.events, [key]: e.target.checked },
                  })
                }
                style={{ width: 16, height: 16, cursor: "pointer" }}
              />
              {label}
            </label>
          ))}
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
