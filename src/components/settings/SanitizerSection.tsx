import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Shield } from "lucide-react";
import { api, type SanitizerConfig, type SanitizerCustomPattern } from "../../lib/api";
import { useToast } from "../Toast";
import { SectionHeader } from "../ui/SectionHeader";

/** Privacy / secret redaction configuration section. */
export function SanitizerSection() {
  const { t } = useTranslation();
  const toast = useToast();
  const [config, setConfig] = useState<SanitizerConfig | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    (async () => {
      try {
        const data = await api.sanitizerConfig();
        setConfig(data);
      } catch {
        // Silently fail
      } finally {
        setLoading(false);
      }
    })();
  }, []);

  const handleSave = async () => {
    if (!config) return;
    setSaving(true);
    try {
      const updated = await api.updateSanitizer(config);
      setConfig(updated);
      toast.success(t("sanitizer.saved"));
    } catch {
      toast.error(t("sanitizer.saveFailed"));
    } finally {
      setSaving(false);
    }
  };

  const handleAddPattern = () => {
    if (!config) return;
    setConfig({
      ...config,
      custom_patterns: [
        ...config.custom_patterns,
        { name: "", pattern: "", replacement: "***" },
      ],
    });
  };

  const handleRemovePattern = (index: number) => {
    if (!config) return;
    setConfig({
      ...config,
      custom_patterns: config.custom_patterns.filter((_, i) => i !== index),
    });
  };

  const handleUpdatePattern = (
    index: number,
    field: keyof SanitizerCustomPattern,
    value: string,
  ) => {
    if (!config) return;
    setConfig({
      ...config,
      custom_patterns: config.custom_patterns.map((p, i) =>
        i === index ? { ...p, [field]: value } : p,
      ),
    });
  };

  if (loading) {
    return <div className="panel-loading">{t("common.loading")}</div>;
  }
  if (!config) return null;

  return (
    <section>
      <SectionHeader title={t("sanitizer.title")} icon={Shield} />

      <div className="settings-section">
        <h3 className="settings-section-title">{t("sanitizer.title")}</h3>
        <div
          className="settings-actions"
          style={{
            flexDirection: "column",
            alignItems: "stretch",
            gap: "var(--space-2)",
          }}
        >
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
              checked={config.enabled}
              onChange={(e) =>
                setConfig({ ...config, enabled: e.target.checked })
              }
              style={{ width: 16, height: 16, cursor: "pointer" }}
            />
            {t("sanitizer.enable")}
          </label>

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
              checked={config.redact_secrets}
              onChange={(e) =>
                setConfig({ ...config, redact_secrets: e.target.checked })
              }
              style={{ width: 16, height: 16, cursor: "pointer" }}
            />
            {t("sanitizer.redactSecrets")}
          </label>

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
              checked={config.scan_response}
              onChange={(e) =>
                setConfig({ ...config, scan_response: e.target.checked })
              }
              style={{ width: 16, height: 16, cursor: "pointer" }}
            />
            {t("sanitizer.scanResponse")}
          </label>
        </div>
      </div>

      <div className="settings-section">
        <h3 className="settings-section-title">
          {t("sanitizer.customPatterns")}
        </h3>
        <p className="settings-hint">{t("sanitizer.customPatternsHint")}</p>

        {config.custom_patterns.length > 0 ? (
          <div className="settings-table-wrapper">
            <table className="settings-table">
              <caption className="sr-only">
                {t("sanitizer.customPatterns")}
              </caption>
              <thead>
                <tr>
                  <th scope="col">{t("sanitizer.patternName")}</th>
                  <th scope="col">{t("sanitizer.patternRegex")}</th>
                  <th scope="col">{t("sanitizer.patternReplacement")}</th>
                  <th scope="col" aria-label={t("common.actions")} />
                </tr>
              </thead>
              <tbody>
                {config.custom_patterns.map((pattern, idx) => (
                  <tr key={idx}>
                    <td>
                      <input
                        type="text"
                        className="settings-input"
                        value={pattern.name}
                        onChange={(e) =>
                          handleUpdatePattern(idx, "name", e.target.value)
                        }
                        placeholder="e.g. SSN"
                        style={{ width: "120px" }}
                      />
                    </td>
                    <td>
                      <input
                        type="text"
                        className="settings-input"
                        value={pattern.pattern}
                        onChange={(e) =>
                          handleUpdatePattern(idx, "pattern", e.target.value)
                        }
                        placeholder="e.g. \\d{3}-\\d{2}-\\d{4}"
                        style={{
                          width: "180px",
                          fontFamily: "var(--font-mono, monospace)",
                        }}
                      />
                    </td>
                    <td>
                      <input
                        type="text"
                        className="settings-input"
                        value={pattern.replacement}
                        onChange={(e) =>
                          handleUpdatePattern(
                            idx,
                            "replacement",
                            e.target.value,
                          )
                        }
                        placeholder="***"
                        style={{ width: "100px" }}
                      />
                    </td>
                    <td>
                      <button
                        className="btn btn-sm"
                        onClick={() => handleRemovePattern(idx)}
                      >
                        {t("sanitizer.removePattern")}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <p
            className="settings-hint"
            style={{ fontStyle: "italic" }}
          >
            {t("sanitizer.noPatterns")}
          </p>
        )}

        <div style={{ marginTop: "var(--space-3)" }}>
          <button className="btn btn-sm" onClick={handleAddPattern}>
            {t("sanitizer.addPattern")}
          </button>
        </div>
      </div>

      <div className="settings-actions">
        <button
          className="btn btn-primary"
          onClick={handleSave}
          disabled={saving}
        >
          {saving ? t("common.saving") : t("common.save")}
        </button>
      </div>
    </section>
  );
}