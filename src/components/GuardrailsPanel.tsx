import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Shield } from "lucide-react";
import { SectionHeader } from "./ui/SectionHeader";
import { api, type GuardrailsConfig } from "../lib/api";
import { useToast } from "./Toast";

export function GuardrailsPanel() {
  const { t } = useTranslation();
  const toast = useToast();
  const [config, setConfig] = useState<GuardrailsConfig | null>(null);
  const [saving, setSaving] = useState(false);
  const [blockedInput, setBlockedInput] = useState("");
  const [allowedInput, setAllowedInput] = useState("");

  const { isLoading: loading, refetch } = useQuery({
    queryKey: ["guardrails-config"],
    queryFn: async () => {
      const data = await api.guardrailsConfig();
      setConfig(data);
      setBlockedInput(data.blocked_patterns.join("\n"));
      setAllowedInput(data.allowed_patterns.join("\n"));
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
      const updated = await api.updateGuardrails({
        enabled: config.enabled,
        blocked_patterns: blockedInput.split("\n").map((s) => s.trim()).filter(Boolean),
        allowed_patterns: allowedInput.split("\n").map((s) => s.trim()).filter(Boolean),
        max_request_chars: config.max_request_chars,
        block_message: config.block_message,
      });
      setConfig(updated);
      toast.success(t("guardrails.saved"));
    } catch {
      toast.error(t("guardrails.saveFailed"));
    } finally {
      setSaving(false);
    }
  };

  if (loading) return <div className="panel-loading">{t("common.loading")}</div>;
  if (!config) return null;

  return (
    <section>
      <SectionHeader title={t("guardrails.title")} icon={Shield} onRefresh={refresh} refreshing={loading} />

      <div className="settings-section">
        <h3 className="settings-section-title">{t("guardrails.status")}</h3>
        <div className="settings-actions">
          <label style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", cursor: "pointer", fontSize: "var(--text-sm)" }}>
            <input
              type="checkbox"
              checked={config.enabled}
              onChange={(e) => setConfig({ ...config, enabled: e.target.checked })}
              style={{ width: 16, height: 16, cursor: "pointer" }}
            />
            {t("guardrails.enable")}
          </label>
        </div>
      </div>

      <div className="settings-section">
        <h3 className="settings-section-title">{t("guardrails.blockedPatterns")}</h3>
        <p className="settings-hint">{t("guardrails.blockedHint")}</p>
        <textarea
          className="settings-textarea"
          value={blockedInput}
          onChange={(e) => setBlockedInput(e.target.value)}
          placeholder={"e.g.\nignore all previous instructions\nreveal your system prompt"}
          rows={6}
          style={{ width: "100%", fontFamily: "var(--font-mono, monospace)", fontSize: "var(--text-sm)" }}
        />
      </div>

      <div className="settings-section">
        <h3 className="settings-section-title">{t("guardrails.allowedPatterns")}</h3>
        <p className="settings-hint">{t("guardrails.allowedHint")}</p>
        <textarea
          className="settings-textarea"
          value={allowedInput}
          onChange={(e) => setAllowedInput(e.target.value)}
          placeholder={"e.g.\n^marketplace_.*\n^internal_api_"}
          rows={4}
          style={{ width: "100%", fontFamily: "var(--font-mono, monospace)", fontSize: "var(--text-sm)" }}
        />
      </div>

      <div className="settings-section">
        <h3 className="settings-section-title">{t("guardrails.advanced")}</h3>
        <div className="settings-stats-grid">
          <div className="settings-stat">
            <span className="settings-stat-label">{t("guardrails.maxRequestChars")}</span>
            <input
              type="number"
              className="settings-input"
              value={config.max_request_chars ?? 0}
              onChange={(e) =>
                setConfig({
                  ...config,
                  max_request_chars: e.target.value ? parseInt(e.target.value) : null,
                })
              }
              placeholder="0 = unlimited"
              style={{ width: "120px" }}
            />
          </div>
        </div>
        <div className="settings-stat" style={{ marginTop: "var(--space-3)" }}>
          <span className="settings-stat-label">{t("guardrails.blockMessage")}</span>
          <input
            type="text"
            className="settings-input"
            value={config.block_message}
            onChange={(e) => setConfig({ ...config, block_message: e.target.value })}
            style={{ width: "100%", maxWidth: "400px" }}
          />
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
