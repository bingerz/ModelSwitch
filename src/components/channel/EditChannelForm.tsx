import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  api,
  type Channel,
  type PayloadRulesConfig,
  validateChannelForm,
} from "../../lib/api";
import { useToast } from "../Toast";
import { FormFields } from "./FormFields";

export function EditChannelForm({
  channel,
  onSave,
  onCancel,
}: {
  channel: Channel;
  onSave: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  const toast = useToast();
  const [name, setName] = useState(channel.name);
  const [provider, setProvider] = useState(channel.provider);
  const [priority, setPriority] = useState(channel.priority);
  const [weight, setWeight] = useState(channel.weight);
  const [costPerToken, setCostPerToken] = useState(
    channel.cost_per_token != null ? String(channel.cost_per_token) : ""
  );
  const [baseUrl, setBaseUrl] = useState(channel.base_url);
  const [modelMapping, setModelMapping] = useState<Record<string, string>>(
    channel.model_mapping
  );
  const [cooldownMinutes, setCooldownMinutes] = useState(
    channel.cooldown_minutes != null ? String(channel.cooldown_minutes) : ""
  );
  const [inputCostPerMtok, setInputCostPerMtok] = useState(
    channel.input_cost_per_mtok != null ? String(channel.input_cost_per_mtok) : ""
  );
  const [outputCostPerMtok, setOutputCostPerMtok] = useState(
    channel.output_cost_per_mtok != null ? String(channel.output_cost_per_mtok) : ""
  );
  const [rpmLimit, setRpmLimit] = useState(
    channel.rpm_limit != null ? String(channel.rpm_limit) : ""
  );
  const [tpmLimit, setTpmLimit] = useState(
    channel.tpm_limit != null ? String(channel.tpm_limit) : ""
  );
  const [accountGroup, setAccountGroup] = useState(channel.account_group ?? "");
  const [excludedModels, setExcludedModels] = useState(
    channel.excluded_models.join(", ")
  );
  const [tags, setTags] = useState(channel.tags.join(", "));
  const [modelsEndpoint, setModelsEndpoint] = useState(
    channel.models_endpoint ?? ""
  );
  const [modelsRefreshInterval, setModelsRefreshInterval] = useState(
    channel.models_refresh_interval_secs
      ? String(channel.models_refresh_interval_secs)
      : ""
  );
  const [credentialType, setCredentialType] = useState("api_key");
  const [credentialValue, setCredentialValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  // Payload Rules state (runtime per-channel overrides)
  const [showPayloadRules, setShowPayloadRules] = useState(false);
  const [payloadStrip, setPayloadStrip] = useState("");
  const [payloadDefaults, setPayloadDefaults] = useState("");
  const [payloadOverrides, setPayloadOverrides] = useState("");
  const [payloadModelRules, setPayloadModelRules] = useState("");
  const [payloadSaving, setPayloadSaving] = useState(false);

  // Load existing payload rules when the form opens so users can see
  // what is currently configured rather than starting from empty fields.
  useEffect(() => {
    let cancelled = false;
    api
      .getPayloadRules(channel.id)
      .then((rules) => {
        if (cancelled) return;
        setPayloadStrip(rules.strip?.join("\n") ?? "");
        setPayloadDefaults(
          rules.defaults ? JSON.stringify(rules.defaults, null, 2) : ""
        );
        setPayloadOverrides(
          rules.overrides ? JSON.stringify(rules.overrides, null, 2) : ""
        );
        setPayloadModelRules(
          rules.model_rules ? JSON.stringify(rules.model_rules, null, 2) : ""
        );
      })
      .catch(() => {
        // No rules configured yet (or endpoint unreachable) — leave defaults empty.
      });
    return () => {
      cancelled = true;
    };
  }, [channel.id]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    const validationError = validateChannelForm({ name, baseUrl });
    if (validationError) {
      setError(t(validationError));
      return;
    }
    setSubmitting(true);
    try {
      await api.updateChannel(channel.id, {
        name,
        provider,
        priority,
        weight,
        cost_per_token: costPerToken ? parseFloat(costPerToken) : null,
        base_url: baseUrl,
        enabled: channel.enabled,
        model_mapping: modelMapping,
        cooldown_minutes: cooldownMinutes ? parseInt(cooldownMinutes, 10) : null,
        credential_type: credentialValue ? credentialType : undefined,
        credential_value: credentialValue || undefined,
        rpm_limit: rpmLimit ? parseInt(rpmLimit, 10) : null,
        tpm_limit: tpmLimit ? parseInt(tpmLimit, 10) : null,
        account_group: accountGroup.trim() || null,
        excluded_models: excludedModels
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean),
        tags: tags
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean),
        models_endpoint: modelsEndpoint.trim() || null,
        models_refresh_interval_secs: modelsRefreshInterval
          ? parseInt(modelsRefreshInterval, 10)
          : 300,
      });
      toast.success(t("channels.updated"));
      onSave();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : t("channels.updateFailed");
      setError(msg);
      toast.error(msg);
    } finally {
      setSubmitting(false);
    }
  };

  const handleSavePayloadRules = async () => {
    setPayloadSaving(true);
    try {
      const strip = payloadStrip
        .split(/[\n,]/)
        .map((s) => s.trim())
        .filter(Boolean);
      const parseJson = (text: string): Record<string, unknown> | undefined => {
        const trimmed = text.trim();
        if (!trimmed) return undefined;
        try {
          return JSON.parse(trimmed) as Record<string, unknown>;
        } catch {
          throw new Error(t("channels.payloadRulesJsonError"));
        }
      };
      let modelRules: PayloadRulesConfig["model_rules"];
      const mrText = payloadModelRules.trim();
      if (mrText) {
        try {
          modelRules = JSON.parse(mrText) as PayloadRulesConfig["model_rules"];
        } catch {
          throw new Error(t("channels.payloadRulesJsonError"));
        }
      }
      const rules: PayloadRulesConfig = {
        strip,
        defaults: parseJson(payloadDefaults),
        overrides: parseJson(payloadOverrides),
        model_rules: modelRules,
      };
      await api.updatePayloadRules(channel.id, rules);
      toast.success(t("channels.payloadRulesSaved"));
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : t("channels.payloadRulesSaveFailed");
      toast.error(msg);
    } finally {
      setPayloadSaving(false);
    }
  };

  return (
    <form className="channel-form" onSubmit={handleSubmit}>
      <h3 className="form-title">{t("channels.edit")}</h3>
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
        accountGroup={accountGroup} setAccountGroup={setAccountGroup}
        excludedModels={excludedModels} setExcludedModels={setExcludedModels}
        tags={tags} setTags={setTags}
        modelsEndpoint={modelsEndpoint} setModelsEndpoint={setModelsEndpoint}
        modelsRefreshInterval={modelsRefreshInterval} setModelsRefreshInterval={setModelsRefreshInterval}
        presetModels={[]}
        modelMapping={modelMapping} setModelMapping={setModelMapping}
        showCredential={true}
        credentialPlaceholder={t("channels.keepCredentialEmpty")}
      />
      {error && <div className="form-error">{error}</div>}

      <div className="form-advanced-toggle">
        <button
          type="button"
          className="btn btn-sm btn-ghost"
          onClick={() => setShowPayloadRules(!showPayloadRules)}
        >
          {showPayloadRules ? "\u25BC" : "\u25B6"} {t("channels.payloadRules")}
        </button>
      </div>

      {showPayloadRules && (
        <div className="form-grid" style={{ display: "block" }}>
          <label className="form-field">
            <span>{t("channels.stripFields")}</span>
            <textarea
              value={payloadStrip}
              onChange={(e) => setPayloadStrip(e.target.value)}
              placeholder={"temperature\nmax_tokens"}
              style={{
                minHeight: "60px",
                fontFamily: "monospace",
                fontSize: "var(--text-xs)",
              }}
            />
            <small className="form-hint">{t("channels.stripFieldsHint")}</small>
          </label>
          <label className="form-field">
            <span>{t("channels.defaultValues")}</span>
            <textarea
              value={payloadDefaults}
              onChange={(e) => setPayloadDefaults(e.target.value)}
              placeholder={'{\n  "temperature": 0.7\n}'}
              style={{
                minHeight: "80px",
                fontFamily: "monospace",
                fontSize: "var(--text-xs)",
              }}
            />
            <small className="form-hint">{t("channels.defaultValuesHint")}</small>
          </label>
          <label className="form-field">
            <span>{t("channels.overrides")}</span>
            <textarea
              value={payloadOverrides}
              onChange={(e) => setPayloadOverrides(e.target.value)}
              placeholder={'{\n  "max_tokens": 4096\n}'}
              style={{
                minHeight: "80px",
                fontFamily: "monospace",
                fontSize: "var(--text-xs)",
              }}
            />
            <small className="form-hint">{t("channels.overridesHint")}</small>
          </label>
          <label className="form-field">
            <span>{t("channels.modelRules")}</span>
            <textarea
              value={payloadModelRules}
              onChange={(e) => setPayloadModelRules(e.target.value)}
              placeholder={
                '[\n  {\n    "models": ["gpt-*"],\n    "defaults": { "temperature": 0.5 }\n  }\n]'
              }
              style={{
                minHeight: "80px",
                fontFamily: "monospace",
                fontSize: "var(--text-xs)",
              }}
            />
            <small className="form-hint">{t("channels.modelRulesHint")}</small>
          </label>
          <button
            type="button"
            className="btn btn-sm btn-primary"
            onClick={handleSavePayloadRules}
            disabled={payloadSaving}
          >
            {payloadSaving ? t("common.saving") : t("channels.savePayloadRules")}
          </button>
        </div>
      )}

      <div style={{ display: "flex", gap: "var(--space-2)" }}>
        <button type="submit" className="btn btn-primary" disabled={submitting}>
          {submitting ? t("common.saving") : t("common.save")}
        </button>
        <button type="button" className="btn" onClick={onCancel} disabled={submitting}>
          {t("common.cancel")}
        </button>
      </div>
    </form>
  );
}
