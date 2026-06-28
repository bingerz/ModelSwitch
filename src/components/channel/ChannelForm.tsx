import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  api,
  validateChannelForm,
  invokeTauri,
} from "../../lib/api";
import { isTauri } from "../../lib/runtime";
import { useToast } from "../Toast";
import { type ProviderPreset, type ApiFormat } from "../../lib/presets";
import { PresetSelector, getAvailableFormats } from "./PresetSelector";
import { FormFields, type FormState } from "./FormFields";

const INITIAL_FORM: FormState = {
  name: "",
  provider: "openai",
  priority: 1,
  weight: 100,
  costPerToken: "",
  credentialType: "api_key",
  credentialValue: "",
  baseUrl: "",
  modelMapping: {},
  cooldownMinutes: "",
  inputCostPerMtok: "",
  outputCostPerMtok: "",
  rpmLimit: "",
  tpmLimit: "",
  accountGroup: "",
  excludedModels: "",
  tags: "",
  modelsEndpoint: "",
  modelsRefreshInterval: "",
  maxConcurrent: 0,
  maxRetries: 3,
  proxyUrl: "",
  apiKeys: "",
  headers: "",
};

export function ChannelForm({ onSave }: { onSave: () => void }) {
  const { t } = useTranslation();
  const toast = useToast();
  const [selectedPreset, setSelectedPreset] = useState<string | null>(null);
  const [activePreset, setActivePreset] = useState<ProviderPreset | null>(null);
  const [form, setForm] = useState<FormState>(INITIAL_FORM);
  const [loginInProgress, setLoginInProgress] = useState(false);
  const [apiFormat, setApiFormat] = useState<ApiFormat>("anthropic");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  // Listen for WebView login cookies (safe lazy import)
  useEffect(() => {
    let destroyed = false;
    let unlistenFn: (() => void) | null = null;

    import("@tauri-apps/api/event").then(({ listen }) => {
      if (destroyed) return;
      listen("login-cookies-received", (event: { payload: { cookies?: string } }) => {
        const { cookies } = event.payload;
        if (cookies) {
          setForm((prev) => ({ ...prev, credentialValue: cookies }));
        }
        setLoginInProgress(false);
      }).then((fn) => {
        unlistenFn = fn;
      });
    }).catch(() => {
      // Tauri API not available (running in browser)
    });

    return () => {
      destroyed = true;
      unlistenFn?.();
    };
  }, []);

  const handleWebViewLogin = async () => {
    setLoginInProgress(true);
    try {
      await invokeTauri("open_login_webview", { provider: form.provider });
    } catch (e) {
      toast.error(t("channels.webViewLoginFailed", { error: String(e) }));
      setLoginInProgress(false);
    }
  };

  const handlePresetSelect = (preset: ProviderPreset) => {
    const isSelected = selectedPreset === preset.name;
    if (isSelected) {
      // Deselect
      setSelectedPreset(null);
      setActivePreset(null);
      setForm((prev) => ({
        ...prev,
        name: "",
        provider: "openai",
        priority: 1,
        baseUrl: "",
        modelMapping: {},
      }));
      setApiFormat("anthropic");
    } else {
      // Apply preset
      setSelectedPreset(preset.name);
      setActivePreset(preset);
      // Initialize API format — prefer anthropic, then preset's apiFormat, then first available
      const formats = getAvailableFormats(preset);
      let defaultFormat = formats[0];
      if (formats.includes("anthropic")) defaultFormat = "anthropic";
      else if (preset.apiFormat && formats.includes(preset.apiFormat))
        defaultFormat = preset.apiFormat;
      setApiFormat(defaultFormat);
      // Set baseUrl for the selected format
      const url = preset.endpoints?.[defaultFormat] ?? preset.baseUrl;
      // Ensure defaultModel is in the mapping
      const mapping = { ...preset.modelMapping };
      if (preset.defaultModel && !mapping[preset.defaultModel]) {
        mapping[preset.defaultModel] = preset.defaultModel;
      }
      setForm((prev) => ({
        ...prev,
        name: preset.name,
        provider: preset.provider,
        priority: preset.priority,
        baseUrl: url,
        modelMapping: mapping,
      }));
    }
  };

  const handleApiFormatChange = (format: ApiFormat) => {
    setApiFormat(format);
    if (activePreset) {
      const url = activePreset.endpoints?.[format] ?? activePreset.baseUrl;
      setForm((prev) => ({ ...prev, baseUrl: url }));
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    const validationError = validateChannelForm({
      name: form.name,
      baseUrl: form.baseUrl,
    });
    if (validationError) {
      setError(t(validationError));
      return;
    }
    if (form.costPerToken && parseFloat(form.costPerToken) <= 0) {
      setError(t("channels.costPositive"));
      return;
    }
    if (!form.credentialValue.trim() && form.credentialType === "api_key") {
      setError(t("channels.apiKeyRequired"));
      return;
    }
    setSubmitting(true);
    try {
      // Parse headers JSON string into an object; default to empty on failure.
      let parsedHeaders: Record<string, string> = {};
      if (form.headers.trim()) {
        try {
          parsedHeaders = JSON.parse(form.headers) as Record<string, string>;
        } catch {
          parsedHeaders = {};
        }
      }
      await api.createChannel({
        name: form.name,
        provider: form.provider,
        priority: form.priority,
        weight: form.weight,
        cost_per_token: form.costPerToken ? parseFloat(form.costPerToken) : null,
        credential_type: form.credentialType,
        credential_value: form.credentialValue,
        base_url: form.baseUrl,
        model_mapping: form.modelMapping,
        input_cost_per_mtok: form.inputCostPerMtok
          ? parseFloat(form.inputCostPerMtok)
          : null,
        output_cost_per_mtok: form.outputCostPerMtok
          ? parseFloat(form.outputCostPerMtok)
          : null,
        cooldown_minutes: form.cooldownMinutes
          ? parseInt(form.cooldownMinutes, 10)
          : null,
        rpm_limit: form.rpmLimit ? parseInt(form.rpmLimit, 10) : null,
        tpm_limit: form.tpmLimit ? parseInt(form.tpmLimit, 10) : null,
        account_group: form.accountGroup.trim() || null,
        excluded_models: form.excludedModels
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean),
        tags: form.tags
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean),
        models_endpoint: form.modelsEndpoint.trim() || null,
        models_refresh_interval_secs: form.modelsRefreshInterval
          ? parseInt(form.modelsRefreshInterval, 10)
          : 300,
        max_concurrent: form.maxConcurrent,
        max_retries: form.maxRetries,
        proxy_url: form.proxyUrl.trim() || null,
        api_keys: form.apiKeys
          .split("\n")
          .map((s) => s.trim())
          .filter(Boolean),
        headers: parsedHeaders,
      });
      toast.success(t("channels.created"));
      onSave();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : t("channels.createFailed");
      setError(msg);
      toast.error(msg);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <form className="channel-form" onSubmit={handleSubmit}>
      <h3 className="form-title">{t("channels.createChannel")}</h3>

      <PresetSelector selected={selectedPreset} onSelect={handlePresetSelect} />

      <div className="form-divider"><span>{t("channels.orConfigureManually")}</span></div>

      <FormFields
        values={form}
        onChange={(patch) => setForm((prev) => ({ ...prev, ...patch }))}
        presetModels={activePreset?.models ?? []}
        apiKeyUrl={activePreset?.apiKeyUrl}
        defaultModel={activePreset?.defaultModel}
        showCredential={true}
        onWebViewLogin={isTauri ? handleWebViewLogin : undefined}
        loginInProgress={isTauri ? loginInProgress : false}
        presetLocked={selectedPreset !== null}
        apiFormat={apiFormat}
        apiFormats={activePreset ? getAvailableFormats(activePreset) : ["openai"]}
        onApiFormatChange={handleApiFormatChange}
      />
      {error && <div className="form-error">{error}</div>}
      <button type="submit" className="btn btn-primary" disabled={submitting}>
        {submitting ? t("channels.creating") : t("channels.createChannel")}
      </button>
    </form>
  );
}
