import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api, type Channel, validateChannelForm } from "../../lib/api";
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
  const [credentialType, setCredentialType] = useState("api_key");
  const [credentialValue, setCredentialValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    const validationError = validateChannelForm({ name, baseUrl });
    if (validationError) {
      setError(validationError);
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
        presetModels={[]}
        modelMapping={modelMapping} setModelMapping={setModelMapping}
        showCredential={true}
        credentialPlaceholder={t("channels.keepCredentialEmpty")}
      />
      {error && <div className="form-error">{error}</div>}
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
