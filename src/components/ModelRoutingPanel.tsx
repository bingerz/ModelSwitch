import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Route } from "lucide-react";
import { SectionHeader } from "./ui/SectionHeader";
import { api, type ModelRoutingInfo } from "../lib/api";

export function ModelRoutingPanel() {
  const { t } = useTranslation();
  const [data, setData] = useState<ModelRoutingInfo | null>(null);
  const [loading, setLoading] = useState(true);

  const fetchData = async () => {
    setLoading(true);
    try {
      const result = await api.modelRouting();
      setData(result);
    } catch {
      setData(null);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchData();
  }, []);

  if (loading) return <div className="panel-loading">{t("common.loading")}</div>;
  if (!data) return null;

  const aliases = Object.entries(data.model_aliases);
  const groups = Object.entries(data.model_groups);
  const fallbacks = Object.entries(data.model_fallbacks);
  const contextFallbacks = Object.entries(data.context_window_fallbacks);
  const pricing = Object.entries(data.model_pricing);
  const ratios = Object.entries(data.group_ratios);

  return (
    <section>
      <SectionHeader
        title={t("modelRouting.title")}
        icon={Route}
        onRefresh={fetchData}
        refreshing={loading}
      />

      <div className="settings-hint" style={{ marginBottom: "1rem", padding: "0.5rem 0.75rem", background: "var(--color-bg-secondary, #f5f5f5)", borderRadius: "6px" }}>
        {t("modelRouting.readOnlyHint")}
      </div>

            {/* Model Aliases */}
      <div className="settings-section">
        <h3 className="settings-section-title">{t("modelRouting.aliases")}</h3>
        {aliases.length === 0 ? (
          <p className="settings-hint">{t("modelRouting.noAliases")}</p>
        ) : (
          <div className="settings-table-wrapper">
            <table className="settings-table">
              <thead>
                <tr>
                  <th scope="col">{t("modelRouting.alias")}</th>
                  <th scope="col">{t("modelRouting.canonical")}</th>
                </tr>
              </thead>
              <tbody>
                {aliases.map(([alias, canonical]) => (
                  <tr key={alias}>
                    <th scope="row" className="mono">
                      {alias}
                    </th>
                    <td className="mono">{canonical}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* Model Groups */}
      <div className="settings-section">
        <h3 className="settings-section-title">{t("modelRouting.groups")}</h3>
        {groups.length === 0 ? (
          <p className="settings-hint">{t("modelRouting.noGroups")}</p>
        ) : (
          <div className="settings-table-wrapper">
            <table className="settings-table">
              <thead>
                <tr>
                  <th scope="col">{t("modelRouting.group")}</th>
                  <th scope="col">{t("modelRouting.models")}</th>
                </tr>
              </thead>
              <tbody>
                {groups.map(([group, models]) => (
                  <tr key={group}>
                    <th scope="row" className="mono">
                      {group}
                    </th>
                    <td>
                      <div
                        style={{
                          display: "flex",
                          gap: "var(--space-1)",
                          flexWrap: "wrap",
                        }}
                      >
                        {models.map((model) => (
                          <span
                            key={model}
                            className="meta-tag"
                            style={{
                              fontSize: "var(--text-xs)",
                              background: "var(--color-bg-secondary)",
                            }}
                          >
                            {model}
                          </span>
                        ))}
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* Fallback Chains */}
      <div className="settings-section">
        <h3 className="settings-section-title">{t("modelRouting.fallbacks")}</h3>
        {fallbacks.length === 0 ? (
          <p className="settings-hint">{t("modelRouting.noFallbacks")}</p>
        ) : (
          <div className="settings-table-wrapper">
            <table className="settings-table">
              <thead>
                <tr>
                  <th scope="col">{t("modelRouting.model")}</th>
                  <th scope="col">{t("modelRouting.fallbackList")}</th>
                </tr>
              </thead>
              <tbody>
                {fallbacks.map(([model, chain]) => (
                  <tr key={model}>
                    <th scope="row" className="mono">
                      {model}
                    </th>
                    <td>
                      <div
                        style={{
                          display: "flex",
                          gap: "var(--space-1)",
                          alignItems: "center",
                          flexWrap: "wrap",
                        }}
                      >
                        <span className="mono" style={{ fontSize: "var(--text-sm)" }}>
                          {"\u2192"}
                        </span>
                        {chain.map((fb, i) => (
                          <span key={fb} className="mono" style={{ fontSize: "var(--text-sm)" }}>
                            {fb}
                            {i < chain.length - 1 && (
                              <span style={{ opacity: 0.5, margin: "0 var(--space-1)" }}>
                                {"\u2192"}
                              </span>
                            )}
                          </span>
                        ))}
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* Context Window Fallbacks */}
      <div className="settings-section">
        <h3 className="settings-section-title">
          {t("modelRouting.contextFallbacks")}
        </h3>
        {contextFallbacks.length === 0 ? (
          <p className="settings-hint">{t("modelRouting.noContextFallbacks")}</p>
        ) : (
          <div className="settings-table-wrapper">
            <table className="settings-table">
              <thead>
                <tr>
                  <th scope="col">{t("modelRouting.model")}</th>
                  <th scope="col">{t("modelRouting.fallbackList")}</th>
                </tr>
              </thead>
              <tbody>
                {contextFallbacks.map(([model, chain]) => (
                  <tr key={model}>
                    <th scope="row" className="mono">
                      {model}
                    </th>
                    <td>
                      <div
                        style={{
                          display: "flex",
                          gap: "var(--space-1)",
                          flexWrap: "wrap",
                        }}
                      >
                        {chain.map((fb) => (
                          <span
                            key={fb}
                            className="meta-tag"
                            style={{
                              fontSize: "var(--text-xs)",
                              background: "var(--color-bg-secondary)",
                            }}
                          >
                            {fb}
                          </span>
                        ))}
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* Model Pricing */}
      <div className="settings-section">
        <h3 className="settings-section-title">{t("modelRouting.pricing")}</h3>
        {pricing.length === 0 ? (
          <p className="settings-hint">{t("modelRouting.noPricing")}</p>
        ) : (
          <div className="settings-table-wrapper">
            <table className="settings-table">
              <thead>
                <tr>
                  <th scope="col">{t("modelRouting.model")}</th>
                  <th scope="col">{t("modelRouting.inputCost")}</th>
                  <th scope="col">{t("modelRouting.outputCost")}</th>
                </tr>
              </thead>
              <tbody>
                {pricing.map(([model, p]) => (
                  <tr key={model}>
                    <th scope="row" className="mono">
                      {model}
                    </th>
                    <td className="mono">
                      ${p.input_per_mtok.toFixed(2)}
                    </td>
                    <td className="mono">
                      ${p.output_per_mtok.toFixed(2)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* Group Ratios */}
      <div className="settings-section">
        <h3 className="settings-section-title">{t("modelRouting.groupRatios")}</h3>
        {ratios.length === 0 ? (
          <p className="settings-hint">{t("modelRouting.noRatios")}</p>
        ) : (
          <div className="settings-table-wrapper">
            <table className="settings-table">
              <thead>
                <tr>
                  <th scope="col">{t("modelRouting.group")}</th>
                  <th scope="col">{t("modelRouting.ratio")}</th>
                </tr>
              </thead>
              <tbody>
                {ratios.map(([group, ratio]) => (
                  <tr key={group}>
                    <th scope="row" className="mono">
                      {group}
                    </th>
                    <td className="mono">{ratio.toFixed(2)}x</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </section>
  );
}
