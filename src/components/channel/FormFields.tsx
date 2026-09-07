import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { ApiFormat } from "../../lib/presets";
import { ModelSelector } from "./ModelSelector";

/**
 * Consolidated form state for channel editor fields.
 *
 * Previously each field was threaded through `FormFieldsProps` as an
 * individual value + setter pair (24+ props). Grouping them into a single
 * object eliminates the prop drilling without changing the wire format
 * the parents already use to track this state.
 */
export interface FormState {
  name: string;
  provider: string;
  priority: number;
  weight: number;
  costPerToken: string;
  credentialType: string;
  credentialValue: string;
  baseUrl: string;
  cooldownMinutes: string;
  inputCostPerMtok: string;
  outputCostPerMtok: string;
  rpmLimit: string;
  tpmLimit: string;
  accountGroup: string;
  excludedModels: string;
  tags: string;
  modelsEndpoint: string;
  modelsRefreshInterval: string;
  modelMapping: Record<string, string>;
  maxConcurrent: number;
  maxRetries: number;
  proxyUrl: string;
  headers: string;
  apiKeys: string;
}

export interface FormFieldsProps {
  values: FormState;
  onChange: (patch: Partial<FormState>) => void;
  // Non-form-field props kept as individuals
  presetModels: string[];
  apiKeyUrl?: string;
  websiteUrl?: string;
  modelsUrl?: string;
  endpointCandidates?: string[];
  defaultModel?: string;
  showCredential: boolean;
  onWebViewLogin?: () => void;
  loginInProgress?: boolean;
  presetLocked?: boolean;
  credentialPlaceholder?: string;
  apiFormat?: ApiFormat;
  apiFormats?: ApiFormat[];
  onApiFormatChange?: (format: ApiFormat) => void;
  onRefreshModels?: (models: string[]) => void;
}

export function FormFields({
  values,
  onChange,
  presetModels,
  apiKeyUrl,
  websiteUrl,
  modelsUrl,
  endpointCandidates,
  defaultModel,
  showCredential,
  onWebViewLogin,
  loginInProgress,
  presetLocked,
  credentialPlaceholder,
  apiFormat,
  apiFormats,
  onApiFormatChange,
  onRefreshModels,
}: FormFieldsProps) {
  const { t } = useTranslation();
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [refreshingModels, setRefreshingModels] = useState(false);
  const [probingEndpoints, setProbingEndpoints] = useState(false);
  const [probeResults, setProbeResults] = useState<Record<string, { latencyMs?: number; error?: string; reachable: boolean }>>({});

  const formatLabel = (fmt: ApiFormat): string => {
    if (fmt === "openai") return t("channels.openaiChat");
    if (fmt === "anthropic") return t("channels.anthropic");
    return t("channels.geminiFormat");
  };

  const handleRefreshModels = async () => {
    if (!values.baseUrl || !values.credentialValue || !onRefreshModels) return;
    
    setRefreshingModels(true);
    try {
      const { fetchProviderModels } = await import("../../lib/api");
      const models = await fetchProviderModels({
        baseUrl: values.baseUrl,
        apiKey: values.credentialValue,
        modelsUrl,
        apiFormat: apiFormat || "openai",
      });
      const modelIds = models.map((m) => m.id);
      onRefreshModels(modelIds);
    } catch (err) {
      console.error("Failed to refresh models:", err);
      alert(t("channels.modelRefreshFailed", { error: err instanceof Error ? err.message : String(err) }));
    } finally {
      setRefreshingModels(false);
    }
  };

  const handleProbeEndpoints = async () => {
    if (!endpointCandidates || endpointCandidates.length === 0) return;
    
    // Build full candidate list: endpointCandidates + current baseUrl if not already included
    const candidateSet = new Set(endpointCandidates);
    const allCandidates = [...candidateSet];
    if (values.baseUrl && !candidateSet.has(values.baseUrl)) {
      allCandidates.unshift(values.baseUrl);
    }

    setProbingEndpoints(true);
    setProbeResults({});
    
    try {
      const { probeEndpoints } = await import("../../lib/api");
      const results = await probeEndpoints(allCandidates);
      
      const resultsMap: Record<string, { latencyMs?: number; error?: string; reachable: boolean }> = {};
      for (const result of results) {
        resultsMap[result.url] = {
          latencyMs: result.latencyMs,
          error: result.error,
          reachable: result.reachable,
        };
      }
      setProbeResults(resultsMap);
    } catch (err) {
      console.error("Failed to probe endpoints:", err);
      alert(t("channels.probeEndpointsFailed", { 
        defaultValue: "Failed to probe endpoints: {{error}}", 
        error: err instanceof Error ? err.message : String(err) 
      }));
    } finally {
      setProbingEndpoints(false);
    }
  };

  const handleSwitchEndpoint = (url: string) => {
    onChange({ baseUrl: url });
    // Clear probe results when switching
    setProbeResults({});
  };

  // Build full candidate list for display
  const allEndpointCandidates = endpointCandidates && endpointCandidates.length > 0
    ? (() => {
        const candidateSet = new Set(endpointCandidates);
        const all = [...candidateSet];
        if (values.baseUrl && !candidateSet.has(values.baseUrl)) {
          all.unshift(values.baseUrl);
        }
        return all;
      })()
    : null;

  const credentialType = values.credentialType;

  return (
    <>
      <div className="form-grid">
        <label className="form-field">
          <span>{t("channels.name")}</span>
          <input
            value={values.name}
            onChange={(e) => onChange({ name: e.target.value })}
            required
          />
        </label>
        <label className="form-field">
          <span>{t("common.provider")}{presetLocked ? ` \u00B7 ${t("channels.presetLocked")}` : ""}</span>
          <select
            value={values.provider}
            onChange={(e) => onChange({ provider: e.target.value })}
            disabled={presetLocked}
          >
            <option value="openai">{t("channels.openai")}</option>
            <option value="anthropic">{t("channels.anthropic")}</option>
            <option value="deepseek">{t("channels.deepseek")}</option>
            <option value="gemini">{t("channels.gemini")}</option>
            <option value="openrouter">{t("channels.openrouter")}</option>
            <option value="ollama">{t("channels.providerOllama")}</option>
            <option value="mistral">{t("channels.providerMistral")}</option>
            <option value="groq">{t("channels.providerGroq")}</option>
            <option value="together">{t("channels.providerTogether")}</option>
            <option value="cohere">{t("channels.providerCohere")}</option>
            <option value="xai">{t("channels.providerXai")}</option>
            <option value="siliconflow">{t("channels.providerSiliconflow")}</option>
            <option value="yi">{t("channels.providerYi")}</option>
            <option value="moonshot">{t("channels.providerMoonshot")}</option>
            <option value="zhipu">{t("channels.providerZhipu")}</option>
            <option value="custom">{t("channels.custom")}</option>
          </select>
        </label>
        <label className="form-field">
          <span>{t("common.priority")}</span>
          <select
            value={values.priority}
            onChange={(e) => onChange({ priority: Number(e.target.value) })}
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
            value={values.weight}
            onChange={(e) => onChange({ weight: Number(e.target.value) })}
            min={1}
            max={1000}
          />
        </label>
        {showCredential && (
          <>
            <label className="form-field">
              <span>{t("channels.credentialType")}</span>
              <select
                value={values.credentialType}
                onChange={(e) => onChange({ credentialType: e.target.value })}
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
                    value={values.credentialValue}
                    onChange={(e) => onChange({ credentialValue: e.target.value })}
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
                    value={values.credentialValue}
                    onChange={(e) => onChange({ credentialValue: e.target.value })}
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
                {websiteUrl && !apiKeyUrl && (
                  <a
                    href={websiteUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="btn btn-sm"
                    title="Provider website"
                  >
                    →
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
          <div style={{ display: "flex", gap: "var(--space-2)", alignItems: "flex-start", flexDirection: "column" }}>
            <div style={{ display: "flex", gap: "var(--space-2)", alignItems: "center", width: "100%" }}>
              {allEndpointCandidates && allEndpointCandidates.length > 1 ? (
                <select
                  value={values.baseUrl}
                  onChange={(e) => handleSwitchEndpoint(e.target.value)}
                  style={{ flex: 1 }}
                  disabled={presetLocked}
                >
                  {allEndpointCandidates.map((url) => (
                    <option key={url} value={url}>
                      {url}
                      {probeResults[url]?.latencyMs !== undefined 
                        ? ` (${probeResults[url].latencyMs}ms)` 
                        : probeResults[url]?.error 
                        ? ` (${probeResults[url].error})` 
                        : ""}
                    </option>
                  ))}
                </select>
              ) : (
                <input
                  value={values.baseUrl}
                  onChange={(e) => onChange({ baseUrl: e.target.value })}
                  placeholder="https://api.openai.com"
                  required
                  style={{ flex: 1 }}
                  disabled={presetLocked}
                />
              )}
              {allEndpointCandidates && allEndpointCandidates.length > 1 && (
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={handleProbeEndpoints}
                  disabled={probingEndpoints}
                  title={t("channels.speedTest", { defaultValue: "测速" })}
                >
                  {probingEndpoints ? "⟳" : "⚡"}
                </button>
              )}
              {onRefreshModels && values.credentialValue && (
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={handleRefreshModels}
                  disabled={refreshingModels || !values.baseUrl}
                  title={t("channels.refreshModels", { defaultValue: "刷新模型列表" })}
                >
                  {refreshingModels ? "⟳" : "↻"}
                </button>
              )}
            </div>
            {allEndpointCandidates && allEndpointCandidates.length > 1 && Object.keys(probeResults).length > 0 && (
              <div style={{ fontSize: "0.85em", color: "var(--text-muted)", width: "100%" }}>
                {allEndpointCandidates.map((url) => {
                  const result = probeResults[url];
                  if (!result) return null;
                  const isCurrent = url === values.baseUrl;
                  return (
                    <div 
                      key={url} 
                      style={{ 
                        padding: "4px 8px", 
                        marginBottom: "2px",
                        background: isCurrent ? "var(--bg-selected)" : "transparent",
                        borderRadius: "4px",
                        display: "flex",
                        justifyContent: "space-between",
                        alignItems: "center"
                      }}
                    >
                      <span style={{ 
                        overflow: "hidden", 
                        textOverflow: "ellipsis", 
                        whiteSpace: "nowrap",
                        flex: 1,
                        marginRight: "8px"
                      }}>
                        {isCurrent && "→ "}{new URL(url).hostname}
                      </span>
                      <span style={{ 
                        fontWeight: "500",
                        color: result.latencyMs !== undefined 
                          ? result.latencyMs < 200 ? "var(--success)" 
                          : result.latencyMs < 500 ? "var(--warning)" 
                          : "var(--text-muted)"
                          : "var(--error)"
                      }}>
                        {result.latencyMs !== undefined 
                          ? `${result.latencyMs}ms` 
                          : result.error || "Failed"}
                      </span>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </label>
        <div className="form-field span-2">
          <span>{t("channels.models")}</span>
          <ModelSelector
            availableModels={presetModels}
            modelMapping={values.modelMapping}
            onModelMappingChange={(mm) => onChange({ modelMapping: mm })}
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
              value={values.costPerToken}
              onChange={(e) => onChange({ costPerToken: e.target.value })}
              placeholder={t("channels.costPlaceholder")}
            />
          </label>
          <label className="form-field">
            <span>{t("channels.inputCostPerMtok")}</span>
            <input
              type="number"
              step="0.0001"
              value={values.inputCostPerMtok}
              onChange={(e) => onChange({ inputCostPerMtok: e.target.value })}
              placeholder={t("channels.costPlaceholder")}
            />
          </label>
          <label className="form-field">
            <span>{t("channels.outputCostPerMtok")}</span>
            <input
              type="number"
              step="0.0001"
              value={values.outputCostPerMtok}
              onChange={(e) => onChange({ outputCostPerMtok: e.target.value })}
              placeholder={t("channels.costPlaceholder")}
            />
          </label>
          <label className="form-field">
            <span>{t("channels.cooldown")}</span>
            <input
              type="number"
              value={values.cooldownMinutes}
              onChange={(e) => onChange({ cooldownMinutes: e.target.value })}
              placeholder={t("channels.cooldownPlaceholder")}
              min={1}
            />
          </label>
          <label className="form-field">
            <span>{t("channels.rpmLimit")}</span>
            <input
              type="number"
              value={values.rpmLimit}
              onChange={(e) => onChange({ rpmLimit: e.target.value })}
              placeholder={t("channels.rpmPlaceholder")}
              min={1}
            />
          </label>
          <label className="form-field">
            <span>{t("channels.tpmLimit")}</span>
            <input
              type="number"
              value={values.tpmLimit}
              onChange={(e) => onChange({ tpmLimit: e.target.value })}
              placeholder={t("channels.tpmPlaceholder")}
              min={1}
            />
          </label>
          <label className="form-field">
            <span>{t("channels.accountGroup")}</span>
            <input
              type="text"
              value={values.accountGroup}
              onChange={(e) => onChange({ accountGroup: e.target.value })}
              placeholder="e.g., production, staging"
            />
            <small className="form-hint">{t("channels.accountGroupHint")}</small>
          </label>
          <label className="form-field">
            <span>{t("channels.excludedModels")}</span>
            <input
              type="text"
              value={values.excludedModels}
              onChange={(e) => onChange({ excludedModels: e.target.value })}
              placeholder="e.g., gpt-4, claude-3-opus"
            />
            <small className="form-hint">{t("channels.excludedModelsHint")}</small>
          </label>
          <label className="form-field">
            <span>{t("channels.tags")}</span>
            <input
              type="text"
              value={values.tags}
              onChange={(e) => onChange({ tags: e.target.value })}
              placeholder="e.g., priority, backup"
            />
            <small className="form-hint">{t("channels.tagsHint")}</small>
          </label>
          <label className="form-field">
            <span>{t("channels.modelsEndpoint")}</span>
            <input
              type="text"
              value={values.modelsEndpoint}
              onChange={(e) => onChange({ modelsEndpoint: e.target.value })}
              placeholder="/v1/models"
            />
            <small className="form-hint">{t("channels.modelsEndpointHint")}</small>
          </label>
          <label className="form-field">
            <span>{t("channels.modelsRefreshInterval")}</span>
            <input
              type="number"
              value={values.modelsRefreshInterval}
              onChange={(e) => onChange({ modelsRefreshInterval: e.target.value })}
              placeholder="300"
              min={0}
            />
            <small className="form-hint">{t("channels.modelsRefreshIntervalHint")}</small>
          </label>
          <label className="form-field">
            <span>{t("channels.maxConcurrent")}</span>
            <input
              type="number"
              value={values.maxConcurrent}
              onChange={(e) => onChange({ maxConcurrent: Number(e.target.value) })}
              placeholder="10"
              min={0}
            />
            <small className="form-hint">{t("channels.maxConcurrentDesc")}</small>
          </label>
          <label className="form-field">
            <span>{t("channels.maxRetries")}</span>
            <input
              type="number"
              value={values.maxRetries}
              onChange={(e) => onChange({ maxRetries: Number(e.target.value) })}
              placeholder="3"
              min={0}
            />
            <small className="form-hint">{t("channels.maxRetriesDesc")}</small>
          </label>
          <label className="form-field">
            <span>{t("channels.proxyUrl")}</span>
            <input
              type="text"
              value={values.proxyUrl}
              onChange={(e) => onChange({ proxyUrl: e.target.value })}
              placeholder="http://proxy:8080"
            />
            <small className="form-hint">{t("channels.proxyUrlDesc")}</small>
          </label>
          <label className="form-field">
            <span>{t("channels.apiKeys")}</span>
            <textarea
              value={values.apiKeys}
              onChange={(e) => onChange({ apiKeys: e.target.value })}
              placeholder={"sk-...\nsk-..."}
              rows={3}
              style={{
                fontFamily: "monospace",
                fontSize: "var(--text-xs)",
              }}
            />
            <small className="form-hint">{t("channels.apiKeysDesc")}</small>
          </label>
          <label className="form-field">
            <span>{t("channels.customHeaders")}</span>
            <textarea
              value={values.headers}
              onChange={(e) => onChange({ headers: e.target.value })}
              placeholder={'{"X-Custom":"value"}'}
              rows={3}
              style={{
                fontFamily: "monospace",
                fontSize: "var(--text-xs)",
              }}
            />
            <small className="form-hint">{t("channels.customHeadersDesc")}</small>
          </label>
        </div>
      )}
    </>
  );
}
