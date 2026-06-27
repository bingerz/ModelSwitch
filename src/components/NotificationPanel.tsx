import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Bell } from "lucide-react";
import { SectionHeader } from "./ui/SectionHeader";
import { api, type NotificationConfig } from "../lib/api";
import { useToast } from "./Toast";

export function NotificationPanel() {
  const { t } = useTranslation();
  const toast = useToast();
  const [config, setConfig] = useState<NotificationConfig | null>(null);
  const [saving, setSaving] = useState(false);

  const { isLoading: loading, refetch } = useQuery({
    queryKey: ["notification-config"],
    queryFn: async () => {
      const data = await api.notificationConfig();
      setConfig(data);
      return data;
    },
    // Silently fail
    retry: false,
  });

  const refresh = async () => {
    await refetch();
  };

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

      {/* SMTP Email */}
      <div className="settings-section">
        <h3 className="settings-section-title">{t("notifications.smtp")}</h3>
        <div style={{ marginTop: "var(--space-3)" }}>
          <label className="settings-stat-label">
            <input
              type="checkbox"
              checked={config.smtp_enabled}
              onChange={(e) => setConfig({ ...config, smtp_enabled: e.target.checked })}
              style={{ marginRight: "var(--space-2)" }}
            />
            {t("notifications.smtpEnable")}
          </label>
        </div>
        <div style={{ marginTop: "var(--space-2)" }}>
          <label className="settings-stat-label">{t("notifications.smtpHost")}</label>
          <input
            type="text"
            className="settings-input"
            value={config.smtp_host ?? ""}
            onChange={(e) =>
              setConfig({ ...config, smtp_host: e.target.value || null })
            }
            placeholder="smtp.gmail.com"
            style={{ width: "100%", maxWidth: "400px" }}
          />
        </div>
        <div style={{ marginTop: "var(--space-2)" }}>
          <label className="settings-stat-label">{t("notifications.smtpPort")}</label>
          <input
            type="number"
            min="1"
            max="65535"
            className="settings-input"
            value={config.smtp_port ?? 587}
            onChange={(e) =>
              setConfig({
                ...config,
                smtp_port: parseInt(e.target.value, 10) || 587,
              })
            }
            style={{ width: "100%", maxWidth: "100px" }}
          />
        </div>
        <div style={{ marginTop: "var(--space-2)" }}>
          <label className="settings-stat-label">{t("notifications.smtpUsername")}</label>
          <input
            type="text"
            className="settings-input"
            value={config.smtp_username ?? ""}
            onChange={(e) =>
              setConfig({ ...config, smtp_username: e.target.value || null })
            }
            placeholder="alerts@example.com"
            style={{ width: "100%", maxWidth: "300px" }}
          />
        </div>
        <div style={{ marginTop: "var(--space-2)" }}>
          <label className="settings-stat-label">{t("notifications.smtpPassword")}</label>
          <input
            type="password"
            className="settings-input"
            value={config.smtp_password ?? ""}
            onChange={(e) =>
              setConfig({ ...config, smtp_password: e.target.value || null })
            }
            placeholder={t("notifications.smtpPasswordPlaceholder")}
            style={{ width: "100%", maxWidth: "300px" }}
          />
        </div>
        <div style={{ marginTop: "var(--space-2)" }}>
          <label className="settings-stat-label">{t("notifications.smtpFrom")}</label>
          <input
            type="email"
            className="settings-input"
            value={config.smtp_from ?? ""}
            onChange={(e) =>
              setConfig({ ...config, smtp_from: e.target.value || null })
            }
            placeholder="alerts@example.com"
            style={{ width: "100%", maxWidth: "300px" }}
          />
        </div>
        <div style={{ marginTop: "var(--space-2)" }}>
          <label className="settings-stat-label">{t("notifications.smtpAdminEmail")}</label>
          <input
            type="email"
            className="settings-input"
            value={config.smtp_admin_email ?? ""}
            onChange={(e) =>
              setConfig({ ...config, smtp_admin_email: e.target.value || null })
            }
            placeholder="admin@example.com"
            style={{ width: "100%", maxWidth: "300px" }}
          />
        </div>
        <div style={{ marginTop: "var(--space-2)" }}>
          <label className="settings-stat-label">
            <input
              type="checkbox"
              checked={config.smtp_use_tls}
              onChange={(e) => setConfig({ ...config, smtp_use_tls: e.target.checked })}
              style={{ marginRight: "var(--space-2)" }}
            />
            {t("notifications.smtpUseTls")}
          </label>
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
