import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../lib/api";
import { useToast } from "../Toast";

interface CompletionRatiosSectionProps {
  initialRatios: Record<string, number>;
  onRefresh: () => Promise<void>;
}

/** Completion-ratio overrides table + add/remove form. */
export function CompletionRatiosSection({
  initialRatios,
}: CompletionRatiosSectionProps) {
  const { t } = useTranslation();
  const toast = useToast();
  const [completionRatios, setCompletionRatios] = useState<Record<string, number>>(
    {},
  );
  const [newRatioModel, setNewRatioModel] = useState("");
  const [newRatioValue, setNewRatioValue] = useState("");
  const [savingRatios, setSavingRatios] = useState(false);

  // Sync server-provided ratios into local editable state on each poll.
  // Local state is needed because the UI lets users edit ratios before saving.
  useEffect(() => {
    setCompletionRatios(initialRatios);
  }, [initialRatios]);

  const handleAddRatio = () => {
    const model = newRatioModel.trim();
    const ratio = parseFloat(newRatioValue);
    if (model && !isNaN(ratio) && ratio > 0) {
      setCompletionRatios({ ...completionRatios, [model]: ratio });
      setNewRatioModel("");
      setNewRatioValue("");
    }
  };

  const handleRemoveRatio = (model: string) => {
    const next = { ...completionRatios };
    delete next[model];
    setCompletionRatios(next);
  };

  const handleSaveRatios = async () => {
    setSavingRatios(true);
    try {
      const updated = await api.updateCompletionRatios(completionRatios);
      setCompletionRatios(updated);
      toast.success(t("settings.completionRatiosSaved"));
    } catch {
      toast.error(t("settings.completionRatiosSaveFailed"));
    } finally {
      setSavingRatios(false);
    }
  };

  return (
    <div className="settings-section">
      <h3 className="settings-section-title">
        {t("settings.completionRatios")}
      </h3>
      <p className="settings-hint">{t("settings.completionRatiosHint")}</p>
      {Object.keys(completionRatios).length > 0 && (
        <div className="settings-table-wrapper mb-2">
          <table className="settings-table">
            <caption className="sr-only">
              {t("settings.completionRatios")}
            </caption>
            <thead>
              <tr>
                <th scope="col">Model</th>
                <th scope="col">Ratio</th>
                <th scope="col" aria-label={t("common.actions")} />
              </tr>
            </thead>
            <tbody>
              {Object.entries(completionRatios).map(([model, ratio]) => (
                <tr key={model}>
                  <th scope="row" className="mono">
                    {model}
                  </th>
                  <td className="mono">{ratio.toFixed(2)}x</td>
                  <td>
                    <button
                      className="btn btn-sm"
                      onClick={() => handleRemoveRatio(model)}
                    >
                      {t("common.remove")}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <div style={{ display: "flex", gap: "var(--space-2)", alignItems: "flex-end" }}>
        <div>
          <label className="settings-stat-label" htmlFor="ratio-model-input">
            Model
          </label>
          <input
            id="ratio-model-input"
            type="text"
            className="settings-input"
            value={newRatioModel}
            onChange={(e) => setNewRatioModel(e.target.value)}
            placeholder="e.g. gpt-4o"
            style={{ width: "150px" }}
          />
        </div>
        <div>
          <label className="settings-stat-label" htmlFor="ratio-value-input">
            Ratio
          </label>
          <input
            id="ratio-value-input"
            type="number"
            className="settings-input"
            value={newRatioValue}
            onChange={(e) => setNewRatioValue(e.target.value)}
            placeholder="e.g. 2.0"
            step="0.1"
            min="0.1"
            style={{ width: "100px" }}
          />
        </div>
        <button className="btn btn-sm" onClick={handleAddRatio}>
          {t("common.add")}
        </button>
        <button
          className="btn btn-sm btn-primary"
          disabled={savingRatios}
          onClick={handleSaveRatios}
        >
          {savingRatios ? t("common.saving") : t("common.save")}
        </button>
      </div>
    </div>
  );
}
