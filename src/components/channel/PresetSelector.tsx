import { useTranslation } from "react-i18next";
import {
  type ProviderPreset,
  type ApiFormat,
  groupPresetsByCategory,
} from "../../lib/presets";
import { CATEGORY_ORDER } from "./types";

/**
 * Returns the set of API formats a preset supports.
 * Always includes the preset's apiFormat, plus any from endpoints.
 */
export function getAvailableFormats(preset: ProviderPreset): ApiFormat[] {
  const formats = new Set<ApiFormat>();
  if (preset.apiFormat) formats.add(preset.apiFormat);
  if (preset.endpoints) {
    for (const fmt of Object.keys(preset.endpoints) as ApiFormat[]) {
      formats.add(fmt);
    }
  }
  if (formats.size === 0) formats.add("openai");
  return Array.from(formats);
}

export function PresetSelector({
  selected,
  onSelect,
}: {
  selected: string | null;
  onSelect: (preset: ProviderPreset) => void;
}) {
  const { t } = useTranslation();
  const grouped = groupPresetsByCategory();

  return (
    <div className="preset-selector">
      <p className="preset-hint">
        {t("channels.presetHint")}
      </p>
      {CATEGORY_ORDER.map((cat) => {
        const presets = grouped[cat];
        if (presets.length === 0) return null;
        return (
          <div key={cat} className="preset-section">
            <div className="preset-section-title">{t(`presets.${cat}`)}</div>
            <div className="preset-grid">
              {presets.map((p) => (
                <button
                  key={p.name}
                  type="button"
                  className={`preset-card ${selected === p.name ? "selected" : ""}`}
                  onClick={() => onSelect(p)}
                  title={p.baseUrl}
                >
                  <span className="preset-dot" style={{ background: p.iconColor }} />
                  <span className="preset-name">{p.name}</span>
                </button>
              ))}
            </div>
          </div>
        );
      })}
    </div>
  );
}
