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
import { FormFields } from "./FormFields";

export function ChannelForm({ onSave }: { onSave: () => void }) {
  const { t } = useTranslation();
  const toast = useToast();
  const [selectedPreset, setSelectedPreset] = useState<string | null>(null);
  const [activePreset, setActivePreset] = useState<ProviderPreset | null>(null);
  const [name, setName] = useState("");
  const [provider, setProvider] = useState("openai");
  const [priority, setPriority] = useState(1);
  const [weight, setWeight] = useState(100);
  const [costPerToken, setCostPerToken] = useState("");
  const [credentialType, setCredentialType] = useState("api_key");
  const [credentialValue, setCredentialValue] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [modelMapping, setModelMapping] = useState<Record<string, string>>({});
  const [cooldownMinutes, setCooldownMinutes] = useState("");
  const [loginInProgress, setLoginInProgress] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [inputCostPerMtok, setInputCostPerMtok] = useState("");
  const [outputCostPerMtok, setOutputCostPerMtok] = useState("");
  const [rpmLimit, setRpmLimit] = useState("");
  const [tpmLimit, setTpmLimit] = useState("");
  const [apiFormat, setApiFormat] = useState<ApiFormat>("anthropic");

  // Listen for WebView login cookies (safe lazy import)
  useEffect(() => {
    let destroyed = false;
    let unlistenFn: (() => void) | null = null;

    import("@tauri-apps/api/event").then(({ listen }) => {
      if (destroyed) return;
      listen("login-cookies-received", (event: { payload: { cookies?: string } }) => {
        const { cookies } = event.payload;
        if (cookies) {
          setCredentialValue(cookies);
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
      await invokeTauri("open_login_webview", { provider });
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
      setName("");
      setProvider("openai");
      setPriority(1);
      setBaseUrl("");
      setModelMapping({});
      setApiFormat("anthropic");
    } else {
      // Apply preset
      setSelectedPreset(preset.name);
      setActivePreset(preset);
      setName(preset.name);
      setProvider(preset.provider);
      setPriority(preset.priority);
      // Initialize API format — prefer anthropic, then preset's apiFormat, then first available
      const formats = getAvailableFormats(preset);
      let defaultFormat = formats[0];
      if (formats.includes("anthropic")) defaultFormat = "anthropic";
      else if (preset.apiFormat && formats.includes(preset.apiFormat))
        defaultFormat = preset.apiFormat;
      setApiFormat(defaultFormat);
      // Set baseUrl for the selected format
      const url = preset.endpoints?.[defaultFormat] ?? preset.baseUrl;
      setBaseUrl(url);
      // Ensure defaultModel is in the mapping
      const mapping = { ...preset.modelMapping };
      if (preset.defaultModel && !mapping[preset.defaultModel]) {
        mapping[preset.defaultModel] = preset.defaultModel;
      }
      setModelMapping(mapping);
    }
  };

  const handleApiFormatChange = (format: ApiFormat) => {
    setApiFormat(format);
    if (activePreset) {
      const url = activePreset.endpoints?.[format] ?? activePreset.baseUrl;
      setBaseUrl(url);
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    const validationError = validateChannelForm({ name, baseUrl });
    if (validationError) {
      setError(validationError);
      return;
    }
    if (costPerToken && parseFloat(costPerToken) <= 0) {
      setError(t("channels.costPositive"));
      return;
    }
    if (!credentialValue.trim() && credentialType === "api_key") {
      setError(t("channels.apiKeyRequired"));
      return;
    }
    setSubmitting(true);
    try {
      await api.createChannel({
        name,
        provider,
        priority,
        weight,
        cost_per_token: costPerToken ? parseFloat(costPerToken) : null,
        credential_type: credentialType,
        credential_value: credentialValue,
        base_url: baseUrl,
        model_mapping: modelMapping,
        input_cost_per_mtok: inputCostPerMtok ? parseFloat(inputCostPerMtok) : null,
        output_cost_per_mtok: outputCostPerMtok ? parseFloat(outputCostPerMtok) : null,
        cooldown_minutes: cooldownMinutes ? parseInt(cooldownMinutes, 10) : null,
        rpm_limit: rpmLimit ? parseInt(rpmLimit, 10) : null,
        tpm_limit: tpmLimit ? parseInt(tpmLimit, 10) : null,
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
        name={name} setName={setName}
        provider={provider} setProvider={setProvider}
        priority={priority} setPriority={setPriority}
        weight={weight} setWeight={setWeight}
        costPerToken={costPerToken} setCostPerToken={setCostPerToken}
        credentialType={credentialType} setCredentialType={setCredentialType}
        credentialValue={credentialValue} setCredentialValue={setCredentialValue}
        baseUrl={baseUrl} setBaseUrl={setBaseUrl}
        cooldownMinutes={cooldownMinutes} setCooldownMinutes={setCooldownMinutes}
        inputCostPerMtok={inputCostPerMtok} setInputCostPerMtok={setInputCostPerMtok}
        outputCostPerMtok={outputCostPerMtok} setOutputCostPerMtok={setOutputCostPerMtok}
        rpmLimit={rpmLimit} setRpmLimit={setRpmLimit}
        tpmLimit={tpmLimit} setTpmLimit={setTpmLimit}
        presetModels={activePreset?.models ?? []}
        modelMapping={modelMapping} setModelMapping={setModelMapping}
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
