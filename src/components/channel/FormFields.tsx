import { useState } from "react";
import type { ApiFormat } from "../../lib/presets";
import { ModelSelector } from "./ModelSelector";

export interface FormFieldsProps {
  name: string;
  setName: (v: string) => void;
  provider: string;
  setProvider: (v: string) => void;
  priority: number;
  setPriority: (v: number) => void;
  weight: number;
  setWeight: (v: number) => void;
  costPerToken: string;
  setCostPerToken: (v: string) => void;
  credentialType: string;
  setCredentialType: (v: string) => void;
  credentialValue: string;
  setCredentialValue: (v: string) => void;
  baseUrl: string;
  setBaseUrl: (v: string) => void;
  cooldownMinutes: string;
  setCooldownMinutes: (v: string) => void;
  inputCostPerMtok: string;
  setInputCostPerMtok: (v: string) => void;
  outputCostPerMtok: string;
  setOutputCostPerMtok: (v: string) => void;
  rpmLimit: string;
  setRpmLimit: (v: string) => void;
  tpmLimit: string;
  setTpmLimit: (v: string) => void;
  presetModels: string[];
  modelMapping: Record<string, string>;
  setModelMapping: (v: Record<string, string>) => void;
  apiKeyUrl?: string;
  defaultModel?: string;
  showCredential: boolean;
  onWebViewLogin?: () => void;
  loginInProgress?: boolean;
  presetLocked?: boolean;
  credentialPlaceholder?: string;
  apiFormat?: ApiFormat;
  apiFormats?: ApiFormat[];
  onApiFormatChange?: (format: ApiFormat) => void;
}

export function FormFields({
  name,
  setName,
  provider,
  setProvider,
  priority,
  setPriority,
  weight,
  setWeight,
  costPerToken,
  setCostPerToken,
  credentialType,
  setCredentialType,
  credentialValue,
  setCredentialValue,
  baseUrl,
  setBaseUrl,
  cooldownMinutes,
  setCooldownMinutes,
  inputCostPerMtok,
  setInputCostPerMtok,
  outputCostPerMtok,
  setOutputCostPerMtok,
  rpmLimit,
  setRpmLimit,
  tpmLimit,
  setTpmLimit,
  presetModels,
  modelMapping,
  setModelMapping,
  apiKeyUrl,
  defaultModel,
  showCredential,
  onWebViewLogin,
  loginInProgress,
  presetLocked,
  credentialPlaceholder,
  apiFormat,
  apiFormats,
  onApiFormatChange,
}: FormFieldsProps) {
  const [showAdvanced, setShowAdvanced] = useState(false);
  return (
    <>
      <div className="form-grid">
        <label className="form-field">
          <span>Name</span>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            required
          />
        </label>
        <label className="form-field">
          <span>Provider{presetLocked ? " · preset" : ""}</span>
          <select
            value={provider}
            onChange={(e) => setProvider(e.target.value)}
            disabled={presetLocked}
          >
            <option value="openai">OpenAI</option>
            <option value="anthropic">Anthropic</option>
            <option value="deepseek">DeepSeek</option>
            <option value="gemini">Gemini</option>
            <option value="openrouter">OpenRouter</option>
            <option value="custom">Custom</option>
          </select>
        </label>
        <label className="form-field">
          <span>Priority</span>
          <select
            value={priority}
            onChange={(e) => setPriority(Number(e.target.value))}
          >
            <option value={1}>Priority 1 · Free / Subscription</option>
            <option value={2}>Priority 2 · Economy API</option>
            <option value={3}>Priority 3 · Official API</option>
          </select>
        </label>
        <label className="form-field">
          <span>Weight</span>
          <input
            type="number"
            value={weight}
            onChange={(e) => setWeight(Number(e.target.value))}
            min={1}
            max={1000}
          />
        </label>
        {showCredential && (
          <>
            <label className="form-field">
              <span>Credential Type</span>
              <select
                value={credentialType}
                onChange={(e) => setCredentialType(e.target.value)}
              >
                <option value="api_key">API Key</option>
                <option value="web_session">Web Session</option>
              </select>
            </label>
            <label className="form-field">
              <span>{credentialType === "api_key" ? "API Key" : "Cookie"}</span>
              <div style={{ display: "flex", gap: "var(--space-2)", alignItems: "center" }}>
                <input
                  type="password"
                  value={credentialValue}
                  onChange={(e) => setCredentialValue(e.target.value)}
                  placeholder={
                    credentialPlaceholder ||
                    (credentialType === "api_key"
                      ? "Enter API key"
                      : "Enter cookie")
                  }
                  style={{ flex: 1 }}
                />
                {apiKeyUrl && (
                  <a
                    href={apiKeyUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="btn btn-sm"
                    title="Get API key"
                  >
                    Get Key
                  </a>
                )}
                {credentialType === "web_session" && onWebViewLogin && (
                  <button
                    type="button"
                    className="btn btn-sm btn-primary"
                    onClick={onWebViewLogin}
                    disabled={loginInProgress}
                    title="Open login window to extract cookies"
                  >
                    {loginInProgress ? "Logging in..." : "Login"}
                  </button>
                )}
              </div>
            </label>
          </>
        )}
        {apiFormats && onApiFormatChange && (
          <label className="form-field">
            <span>API Format</span>
            <select
              value={apiFormat}
              onChange={(e) => onApiFormatChange(e.target.value as ApiFormat)}
              disabled={apiFormats.length <= 1}
            >
              {apiFormats.map((fmt) => (
                <option key={fmt} value={fmt}>
                  {fmt === "openai"
                    ? "OpenAI Chat"
                    : fmt === "anthropic"
                      ? "Anthropic"
                      : "Gemini"}
                </option>
              ))}
            </select>
          </label>
        )}
        <label className="form-field">
          <span>Base URL</span>
          <input
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder="https://api.openai.com"
            required
          />
        </label>
        <div className="form-field span-2">
          <span>Models</span>
          <ModelSelector
            availableModels={presetModels}
            modelMapping={modelMapping}
            onModelMappingChange={setModelMapping}
            defaultModel={defaultModel}
          />
        </div>
      </div>

      <div className="form-advanced-toggle">
        <button
          type="button"
          className="btn btn-sm btn-ghost"
          onClick={() => setShowAdvanced(!showAdvanced)}
        >
          {showAdvanced ? "\u25BC" : "\u25B6"} Advanced Settings
        </button>
      </div>

      {showAdvanced && (
        <div className="form-grid">
          <label className="form-field">
            <span>Cost per 1K tokens ($)</span>
            <input
              type="number"
              step="0.0001"
              value={costPerToken}
              onChange={(e) => setCostPerToken(e.target.value)}
              placeholder="e.g. 0.0015"
            />
          </label>
          <label className="form-field">
            <span>Input cost per 1M tokens ($)</span>
            <input
              type="number"
              step="0.0001"
              value={inputCostPerMtok}
              onChange={(e) => setInputCostPerMtok(e.target.value)}
              placeholder="e.g. 0.0015"
            />
          </label>
          <label className="form-field">
            <span>Output cost per 1M tokens ($)</span>
            <input
              type="number"
              step="0.0001"
              value={outputCostPerMtok}
              onChange={(e) => setOutputCostPerMtok(e.target.value)}
              placeholder="e.g. 0.0075"
            />
          </label>
          <label className="form-field">
            <span>Cooldown (minutes)</span>
            <input
              type="number"
              value={cooldownMinutes}
              onChange={(e) => setCooldownMinutes(e.target.value)}
              placeholder="default (30)"
              min={1}
            />
          </label>
          <label className="form-field">
            <span>RPM Limit</span>
            <input
              type="number"
              value={rpmLimit}
              onChange={(e) => setRpmLimit(e.target.value)}
              placeholder="default (60)"
              min={1}
            />
          </label>
          <label className="form-field">
            <span>TPM Limit</span>
            <input
              type="number"
              value={tpmLimit}
              onChange={(e) => setTpmLimit(e.target.value)}
              placeholder="no limit"
              min={1}
            />
          </label>
        </div>
      )}
    </>
  );
}
