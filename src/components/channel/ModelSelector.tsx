import { useState } from "react";

export function ModelSelector({
  availableModels,
  modelMapping,
  onModelMappingChange,
  defaultModel,
}: {
  availableModels: string[];
  modelMapping: Record<string, string>;
  onModelMappingChange: (mapping: Record<string, string>) => void;
  defaultModel?: string;
}) {
  const [customModel, setCustomModel] = useState("");

  const toggleModel = (model: string) => {
    const next = { ...modelMapping };
    if (next[model] !== undefined) {
      delete next[model];
    } else {
      next[model] = model;
    }
    onModelMappingChange(next);
  };

  const addCustomModel = () => {
    const trimmed = customModel.trim();
    if (!trimmed || modelMapping[trimmed] !== undefined) return;
    onModelMappingChange({ ...modelMapping, [trimmed]: trimmed });
    setCustomModel("");
  };

  const selectedModels = Object.keys(modelMapping);

  return (
    <div className="model-selector">
      {availableModels.length > 0 && (
        <div className="model-checkboxes">
          {availableModels.map((model) => (
            <label key={model} className="model-checkbox">
              <input
                type="checkbox"
                checked={modelMapping[model] !== undefined}
                onChange={() => toggleModel(model)}
              />
              <span>
                {model}
                {defaultModel && model === defaultModel && (
                  <span className="model-default-indicator">default</span>
                )}
              </span>
            </label>
          ))}
        </div>
      )}
      {availableModels.length === 0 && selectedModels.length === 0 && (
        <p className="model-hint">
          Select a preset above, or add custom model IDs below.
        </p>
      )}
      <div className="model-custom-row">
        <input
          value={customModel}
          onChange={(e) => setCustomModel(e.target.value)}
          placeholder="Add custom model ID"
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              addCustomModel();
            }
          }}
        />
        <button type="button" className="btn btn-sm" onClick={addCustomModel}>
          Add
        </button>
      </div>
      {selectedModels.length > 0 && (
        <div className="model-tags">
          {selectedModels.map((model) => (
            <span key={model} className="model-tag">
              {model}
              <button
                type="button"
                className="model-tag-remove"
                onClick={() => toggleModel(model)}
                title={`Remove ${model}`}
              >
                ×
              </button>
            </span>
          ))}
        </div>
      )}
    </div>
  );
}
