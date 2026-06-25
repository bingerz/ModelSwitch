import { useState } from "react";
import { useTranslation } from "react-i18next";
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
  accountGroup: string;
  setAccountGroup: (v: string) => void;
  excludedModels: string;
  setExcludedModels: (v: string) => void;
  tags: string;
  setTags: (v: string) => void;
  modelsEndpoint: string;
  setModelsEndpoint: (v: string) => void;
  modelsRefreshInterval: string;
  setModelsRefreshInterval: (v: string) => void;
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
  accountGroup,
  setAccountGroup,
  excludedModels,
  setExcludedModels,
  tags,
  setTags,
  modelsEndpoint,
  setModelsEndpoint,
  modelsRefreshInterval,
  setModelsRefreshInterval,
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
  const { t } = useTranslation();
  const [showAdvanced, setShowAdvanced] = useState(false);

  const formatLabel = (fmt: ApiFormat): string => {
    if (fmt === "openai") return t("channels.openaiChat");
    if (fmt === "anthropic") return t("channels.anthropic");
    return t("channels.geminiFormat");
  };

  return (
    <>
      <div className="form-grid">
        <label className="form-field">
          <span>{t("channels.name")}</span>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            required
          />
        </label>
        <label className="form-field">
          <span>{t("common.provider")}{presetLocked ? ` \u00B7 ${t("channels.presetLocked")}` : ""}</span>
          <select
            value={provider}
            onChange={(e) => setProvider(e.target.value)}
            disabled={presetLocked}
          >
            <option value="openai">{t("channels.openai")}</option>
            <option value="anthropic">{t("channels.anthropic")}</option>
            <option value="deepseek">{t("channels.deepseek")}</option>
            <option value="gemini">{t("channels.gemini")}</option>
            <option value="openrouter">{t("channels.openrouter")}</option>
            <option value="custom">{t("channels.custom")}</option>
          </select>
        </label>
        <label className="form-field">
          <span>{t("common.priority")}</span>
          <select
            value={priority}
            onChange={(e) => setPriority(Number(e.target.value))}
          >
            <option value={1}>{t("channels.priority1Free")}</option>
            <option value={2}>{t("channels.priority2Economy")}</option>
            <option value={3}>{t("channels.priority3Official")}</option>
          </select>
        </label>
        <label className="form-field">
          <span>{t("common.weight")}</span>
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
              <span>{t("channels.credentialType")}</span>
              <select
                value={credentialType}
                onChange={(e) => setCredentialType(e.target.value)}
              >
                <option value="api_key">{t("channels.credentialApiKey")}</option>
                <option value="web_session">{t("channels.credentialWebSession")}</option>
              </select>
            </label>
            <label className="form-field">
              <span>{credentialType === "api_key" ? t("channels.apiKey") : t("channels.cookie")}</span>
              <div style={{ display: "flex", gap: "var(--space-2)", alignItems: "center" }}>
                {credentialType === "web_session" ? (
                  <textarea
                    value={credentialValue}
                    onChange={(e) => setCredentialValue(e.target.value)}
                    placeholder={
                      credentialPlaceholder || t("channels.enterCookie")
                    }
                    style={{
                      flex: 1,
                      minHeight: "60px",
                      fontFamily: "monospace",
                      fontSize: "var(--text-xs)",
                    }}
                  />
                ) : (
                  <input
                    type="password"
                    value={credentialValue}
                    onChange={(e) => setCredentialValue(e.target.value)}
                    placeholder={
                      credentialPlaceholder || t("channels.enterApiKey")
                    }
                    style={{ flex: 1 }}
                  />
                )}
                {apiKeyUrl && (
                  <a
                    href={apiKeyUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="btn btn-sm"
                    title={t("channels.apiKey")}
                  >
                    {t("channels.getKey")}
                  </a>
                )}
                {credentialType === "web_session" && onWebViewLogin && (
                  <button
                    type="button"
                    className="btn btn-sm btn-primary"
                    onClick={onWebViewLogin}
                    disabled={loginInProgress}
                    title={t("channels.loginWebview")}
                  >
                    {loginInProgress ? t("channels.loggingIn") : t("channels.login")}
                  </button>
                )}
              </div>
            </label>
            {credentialType === "web_session" && !onWebViewLogin && (
              <div className="form-hint">
                {t("channels.webSessionHint")}
              </div>
            )}
          </>
        )}
        {apiFormats && onApiFormatChange && (
          <label className="form-field">
            <span>{t("channels.apiFormat")}</span>
            <select
              value={apiFormat}
              onChange={(e) => onApiFormatChange(e.target.value as ApiFormat)}
              disabled={apiFormats.length <= 1}
            >
              {apiFormats.map((fmt) => (
                <option key={fmt} value={fmt}>
                  {formatLabel(fmt)}
                </option>
              ))}
            </select>
          </label>
        )}
        <label className="form-field">
          <span>{t("channels.baseUrl")}</span>
          <input
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder="https://api.openai.com"
            required
          />
        </label>
        <div className="form-field span-2">
          <span>{t("channels.models")}</span>
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
          {showAdvanced ? "\u25BC" : "\u25B6"} {t("common.advancedSettings")}
        </button>
      </div>

      {showAdvanced && (
        <div className="form-grid">
          <label className="form-field">
            <span>{t("channels.costPerToken")}</span>
            <input
              type="number"
              step="0.0001"
              value={costPerToken}
              onChange={(e) => setCostPerToken(e.target.value)}
              placeholder={t("channels.costPlaceholder")}
            />
          </label>
          <label className="form-field">
            <span>{t("channels.inputCostPerMtok")}</span>
            <input
              type="number"
              step="0.0001"
              value={inputCostPerMtok}
              onChange={(e) => setInputCostPerMtok(e.target.value)}
              placeholder={t("channels.costPlaceholder")}
            />
          </label>
          <label className="form-field">
            <span>{t("channels.outputCostPerMtok")}</span>
            <input
              type="number"
              step="0.0001"
              value={outputCostPerMtok}
              onChange={(e) => setOutputCostPerMtok(e.target.value)}
              placeholder={t("channels.costPlaceholder")}
            />
          </label>
          <label className="form-field">
            <span>{t("channels.cooldown")}</span>
            <input
              type="number"
              value={cooldownMinutes}
              onChange={(e) => setCooldownMinutes(e.target.value)}
              placeholder={t("channels.cooldownPlaceholder")}
              min={1}
            />
          </label>
          <label className="form-field">
            <span>{t("channels.rpmLimit")}</span>
            <input
              type="number"
              value={rpmLimit}
              onChange={(e) => setRpmLimit(e.target.value)}
              placeholder={t("channels.rpmPlaceholder")}
              min={1}
            />
          </label>
          <label className="form-field">
            <span>{t("channels.tpmLimit")}</span>
            <input
              type="number"
              value={tpmLimit}
              onChange={(e) => setTpmLimit(e.target.value)}
              placeholder={t("channels.tpmPlaceholder")}
              min={1}
            />
          </label>
          <label className="form-field">
            <span>{t("channels.accountGroup")}</span>
            <input
              type="text"
              value={accountGroup}
              onChange={(e) => setAccountGroup(e.target.value)}
              placeholder="e.g., production, staging"
            />
            <small className="form-hint">{t("channels.accountGroupHint")}</small>
          </label>
          <label className="form-field">
            <span>{t("channels.excludedModels")}</span>
            <input
              type="text"
              value={excludedModels}
              onChange={(e) => setExcludedModels(e.target.value)}
              placeholder="e.g., gpt-4, claude-3-opus"
            />
            <small className="form-hint">{t("channels.excludedModelsHint")}</small>
          </label>
          <label className="form-field">
            <span>{t("channels.tags")}</span>
            <input
              type="text"
              value={tags}
              onChange={(e) => setTags(e.target.value)}
              placeholder="e.g., priority, backup"
            />
            <small className="form-hint">{t("channels.tagsHint")}</small>
          </label>
          <label className="form-field">
            <span>{t("channels.modelsEndpoint")}</span>
            <input
              type="text"
              value={modelsEndpoint}
              onChange={(e) => setModelsEndpoint(e.target.value)}
              placeholder="/v1/models"
            />
            <small className="form-hint">{t("channels.modelsEndpointHint")}</small>
          </label>
          <label className="form-field">
            <span>{t("channels.modelsRefreshInterval")}</span>
            <input
              type="number"
              value={modelsRefreshInterval}
              onChange={(e) => setModelsRefreshInterval(e.target.value)}
              placeholder="300"
              min={0}
            />
            <small className="form-hint">{t("channels.modelsRefreshIntervalHint")}</small>
          </label>
        </div>
      )}
    </>
  );
}
