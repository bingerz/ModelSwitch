import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Activity } from "lucide-react";
import { SectionHeader } from "./ui/SectionHeader";
import { api } from "../lib/api";

export function MetricsPanel() {
  const { t } = useTranslation();
  const { data: metrics = "", isLoading: loading, refetch } = useQuery({
    queryKey: ["metrics"],
    queryFn: () => api.metrics(),
    refetchInterval: 10_000,
    // Silently fail on error — keep showing stale data
    retry: false,
  });

  const refresh = async () => {
    await refetch();
  };

  // Parse Prometheus text format for key metrics summary
  const summary: Record<string, string> = {};
  for (const line of metrics.split("\n")) {
    if (line.startsWith("#") || !line.trim()) continue;
    const match = line.match(/^([a-z_:]+)(\{[^}]*\})?\s+([\d.eE+-]+)$/);
    if (match) {
      const [, name, labels, value] = match;
      const num = parseFloat(value);
      if (!isNaN(num)) {
        const key = labels ? `${name}${labels}` : name;
        summary[key] = num.toLocaleString(undefined, { maximumFractionDigits: 2 });
      }
    }
  }

  return (
    <section>
      <SectionHeader title={t("metrics.title")} icon={Activity} onRefresh={refresh} refreshing={loading} />

      {Object.keys(summary).length > 0 && (
        <div className="settings-section">
          <h3 className="settings-section-title">{t("metrics.summary")}</h3>
          <div className="settings-stats-grid">
            {Object.entries(summary).slice(0, 12).map(([key, value]) => (
              <div key={key} className="settings-stat">
                <span className="settings-stat-label mono" style={{ fontSize: "var(--text-xs)" }}>{key}</span>
                <span className="settings-stat-value mono">{value}</span>
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="settings-section">
        <h3 className="settings-section-title">{t("metrics.rawData")}</h3>
        <p className="settings-hint">
          {t("metrics.rawHint")}
          {"  "}
          <code className="mono" style={{ fontSize: "var(--text-xs)" }}>GET /metrics</code>
        </p>
        <pre
          className="metrics-raw"
          style={{
            background: "var(--color-bg-secondary)",
            border: "1px solid var(--color-border)",
            borderRadius: "8px",
            padding: "var(--space-3)",
            fontSize: "var(--text-xs)",
            fontFamily: "var(--font-mono, monospace)",
            overflow: "auto",
            maxHeight: "500px",
            whiteSpace: "pre-wrap",
            wordBreak: "break-all",
          }}
        >
          {metrics || t("common.noData")}
        </pre>
      </div>
    </section>
  );
}
