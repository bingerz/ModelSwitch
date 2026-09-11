import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronRight } from "lucide-react";
import {
  api,
  type VirtualKey,
  type CreateVirtualKeyResponse,
  type CreateVirtualKeyData,
  type UpdateVirtualKeyData,
} from "../../lib/api";
import { useToast } from "../Toast";
import { useZodValidation } from "../../hooks/useZodValidation";
import { virtualKeyFormSchema } from "../../lib/validation";
import { centsToDollars, dollarsToCents } from "./types";

export interface VirtualKeyFormProps {
  mode: "create" | "edit";
  existingKey?: VirtualKey;
  onSave: (response: CreateVirtualKeyResponse | null) => void;
  onCancel: () => void;
}

export function VirtualKeyForm({
  mode,
  existingKey,
  onSave,
  onCancel,
}: VirtualKeyFormProps) {
  const { t } = useTranslation();
  const toast = useToast();
  const [name, setName] = useState(existingKey?.name ?? "");
  const [dailyBudget, setDailyBudget] = useState(
    centsToDollars(existingKey?.daily_budget_cents ?? null),
  );
  const [monthlyBudget, setMonthlyBudget] = useState(
    centsToDollars(existingKey?.monthly_budget_cents ?? null),
  );
  const [allowedIps, setAllowedIps] = useState(
    (existingKey?.allowed_ips ?? []).join(", "),
  );
  const [allowedModels, setAllowedModels] = useState(
    (existingKey?.allowed_models ?? []).join(", "),
  );
  const [deniedModels, setDeniedModels] = useState(
    (existingKey?.denied_models ?? []).join(", "),
  );

  // Advanced settings (collapsible)
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [group, setGroup] = useState(existingKey?.group ?? "");
  const [rpmLimit, setRpmLimit] = useState(
    existingKey?.rpm_limit != null ? String(existingKey.rpm_limit) : "",
  );
  const [tpmLimit, setTpmLimit] = useState(
    existingKey?.tpm_limit != null ? String(existingKey.tpm_limit) : "",
  );
  // datetime-local requires `YYYY-MM-DDTHH:mm` format. Strip seconds/zone from ISO.
  const [expiresAt, setExpiresAt] = useState(
    existingKey?.expires_at ? existingKey.expires_at.slice(0, 16) : "",
  );

  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const { errors: fieldErrors, validate } = useZodValidation(virtualKeyFormSchema);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    const dailyCents = dollarsToCents(dailyBudget);
    const monthlyCents = dollarsToCents(monthlyBudget);

    // ponytail: dollar→cents parsing returns null for both empty and invalid input;
    // Zod's nullable would otherwise swallow invalid numbers, so surface them here.
    if (dailyBudget.trim() !== "" && dailyCents === null) {
      setError(t("virtualKeys.dailyBudgetInvalid"));
      return;
    }
    if (monthlyBudget.trim() !== "" && monthlyCents === null) {
      setError(t("virtualKeys.monthlyBudgetInvalid"));
      return;
    }

    const parsedRpm = rpmLimit.trim() === "" ? null : Number(rpmLimit.trim());
    const parsedTpm = tpmLimit.trim() === "" ? null : Number(tpmLimit.trim());

    const formData = {
      name: name.trim(),
      daily_budget_cents: dailyCents,
      monthly_budget_cents: monthlyCents,
      rpm_limit: parsedRpm,
      tpm_limit: parsedTpm,
    };
    const result = validate(formData);
    if (!result.ok) return;

    setSubmitting(true);
    try {
      const trimmedGroup = group.trim();
      const trimmedExpiry = expiresAt.trim();

      if (mode === "create") {
        const payload: CreateVirtualKeyData = {
          name: name.trim(),
          daily_budget_cents: dailyCents,
          monthly_budget_cents: monthlyCents,
          allowed_ips: allowedIps.split(",").map((s) => s.trim()).filter(Boolean),
          allowed_models: allowedModels.trim()
            ? allowedModels.split(",").map((s) => s.trim()).filter(Boolean)
            : null,
          denied_models: deniedModels.split(",").map((s) => s.trim()).filter(Boolean),
          rpm_limit: parsedRpm,
          tpm_limit: parsedTpm,
          expires_at: trimmedExpiry ? new Date(trimmedExpiry).toISOString() : null,
          group: trimmedGroup ? trimmedGroup : null,
        };
        const response = await api.virtualKeys.create(payload);
        toast.success(t("virtualKeys.createdToast"));
        onSave(response);
      } else if (existingKey) {
        const payload: UpdateVirtualKeyData = {
          name: name.trim(),
          daily_budget_cents: dailyCents,
          monthly_budget_cents: monthlyCents,
          allowed_ips: allowedIps.split(",").map((s) => s.trim()).filter(Boolean),
          allowed_models: allowedModels.trim()
            ? allowedModels.split(",").map((s) => s.trim()).filter(Boolean)
            : null,
          denied_models: deniedModels.split(",").map((s) => s.trim()).filter(Boolean),
          rpm_limit: parsedRpm,
          tpm_limit: parsedTpm,
          expires_at: trimmedExpiry ? new Date(trimmedExpiry).toISOString() : null,
          group: trimmedGroup ? trimmedGroup : null,
        };
        await api.virtualKeys.update(existingKey.id, payload);
        toast.success(t("virtualKeys.updatedToast"));
        onSave(null);
      }
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : t("virtualKeys.saveFailed");
      setError(msg);
      toast.error(msg);
    } finally {
      setSubmitting(false);
    }
  };

  const title = mode === "create" ? t("virtualKeys.addTitle") : t("virtualKeys.editTitle", { name: existingKey?.name });

  return (
    <form className="channel-form" onSubmit={handleSubmit}>
      <h3 className="form-title">{title}</h3>
      <div className="form-grid">
        <label className="form-field">
          <span>{t("virtualKeys.keyName")}</span>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={t("virtualKeys.namePlaceholder")}
            required
          />
          {fieldErrors.name && <span className="field-error">{fieldErrors.name}</span>}
        </label>
        <label className="form-field">
          <span>{t("virtualKeys.dailyBudget")}</span>
          <input
            value={dailyBudget}
            onChange={(e) => setDailyBudget(e.target.value)}
            placeholder={t("virtualKeys.dailyBudgetPlaceholder")}
            inputMode="decimal"
            title={t("virtualKeys.dailyBudgetTooltip")}
          />
          <span className="field-hint">{t("virtualKeys.dailyBudgetTooltip")}</span>
          {fieldErrors.daily_budget_cents && (
            <span className="field-error">{fieldErrors.daily_budget_cents}</span>
          )}
        </label>
        <label className="form-field">
          <span>{t("virtualKeys.monthlyBudget")}</span>
          <input
            value={monthlyBudget}
            onChange={(e) => setMonthlyBudget(e.target.value)}
            placeholder={t("virtualKeys.monthlyBudgetPlaceholder")}
            inputMode="decimal"
            title={t("virtualKeys.monthlyBudgetTooltip")}
          />
          <span className="field-hint">{t("virtualKeys.monthlyBudgetTooltip")}</span>
          {fieldErrors.monthly_budget_cents && (
            <span className="field-error">{fieldErrors.monthly_budget_cents}</span>
          )}
        </label>
      </div>
      <div className="form-grid">
        <label className="form-field">
          <span>{t("virtualKeys.allowedIps")}</span>
          <input
            value={allowedIps}
            onChange={(e) => setAllowedIps(e.target.value)}
            placeholder={t("virtualKeys.allowedIpsHint")}
          />
        </label>
        <label className="form-field">
          <span>{t("virtualKeys.allowedModels")}</span>
          <input
            value={allowedModels}
            onChange={(e) => setAllowedModels(e.target.value)}
            placeholder={t("virtualKeys.allowedModelsHint")}
          />
        </label>
        <label className="form-field">
          <span>{t("virtualKeys.deniedModels")}</span>
          <input
            value={deniedModels}
            onChange={(e) => setDeniedModels(e.target.value)}
            placeholder={t("virtualKeys.deniedModelsHint")}
          />
        </label>
      </div>

      {/* Advanced settings (collapsible) */}
      <div className="vk-advanced-section">
        <button
          type="button"
          className="vk-advanced-toggle"
          onClick={() => setShowAdvanced((v) => !v)}
          aria-expanded={showAdvanced}
        >
          {showAdvanced ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          <span>{t("virtualKeys.advancedSettings")}</span>
        </button>
        {showAdvanced && (
          <div className="form-grid">
            <label className="form-field">
              <span>{t("virtualKeys.group")}</span>
              <input
                value={group}
                onChange={(e) => setGroup(e.target.value)}
                placeholder={t("virtualKeys.groupPlaceholder")}
              />
            </label>
            <label className="form-field">
              <span>{t("virtualKeys.rpmLimit")}</span>
              <input
                value={rpmLimit}
                onChange={(e) => setRpmLimit(e.target.value)}
                placeholder={t("virtualKeys.rpmLimitPlaceholder")}
                inputMode="numeric"
                title={t("virtualKeys.rpmLimitTooltip")}
              />
              <span className="field-hint">{t("virtualKeys.rpmLimitTooltip")}</span>
              {fieldErrors.rpm_limit && (
                <span className="field-error">{fieldErrors.rpm_limit}</span>
              )}
            </label>
            <label className="form-field">
              <span>{t("virtualKeys.tpmLimit")}</span>
              <input
                value={tpmLimit}
                onChange={(e) => setTpmLimit(e.target.value)}
                placeholder={t("virtualKeys.tpmLimitPlaceholder")}
                inputMode="numeric"
                title={t("virtualKeys.tpmLimitTooltip")}
              />
              <span className="field-hint">{t("virtualKeys.tpmLimitTooltip")}</span>
              {fieldErrors.tpm_limit && (
                <span className="field-error">{fieldErrors.tpm_limit}</span>
              )}
            </label>
            <label className="form-field">
              <span>{t("virtualKeys.expiresAt")}</span>
              <input
                type="datetime-local"
                value={expiresAt}
                onChange={(e) => setExpiresAt(e.target.value)}
              />
            </label>
          </div>
        )}
      </div>

      {error && <div className="form-error">{error}</div>}
      <div style={{ display: "flex", gap: "var(--space-2)" }}>
        <button type="submit" className="btn btn-primary" disabled={submitting}>
          {submitting
            ? t("common.saving")
            : mode === "create"
              ? t("virtualKeys.createKey")
              : t("common.save")}
        </button>
        <button
          type="button"
          className="btn"
          onClick={onCancel}
          disabled={submitting}
        >
          {t("common.cancel")}
        </button>
      </div>
    </form>
  );
}
