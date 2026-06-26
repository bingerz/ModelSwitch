import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Copy, CheckCheck, X } from "lucide-react";
import {
  api,
  type BatchCreateVirtualKeyData,
  type BatchCreateVirtualKeyItem,
} from "../../lib/api";
import { useToast } from "../Toast";
import { dollarsToCents } from "./types";

export interface BatchCreateModalProps {
  onClose: () => void;
  onCreated: () => void;
}

interface SharedSettings {
  count: string;
  namePrefix: string;
  dailyBudget: string;
  monthlyBudget: string;
  allowedModels: string;
  allowedIps: string;
  group: string;
  rpmLimit: string;
  tpmLimit: string;
  expiresAt: string;
}

const INITIAL_SETTINGS: SharedSettings = {
  count: "10",
  namePrefix: "",
  dailyBudget: "",
  monthlyBudget: "",
  allowedModels: "",
  allowedIps: "",
  group: "",
  rpmLimit: "",
  tpmLimit: "",
  expiresAt: "",
};

const MAX_COUNT = 500;

export function BatchCreateModal({ onClose, onCreated }: BatchCreateModalProps) {
  const { t } = useTranslation();
  const toast = useToast();
  const [settings, setSettings] = useState<SharedSettings>(INITIAL_SETTINGS);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [results, setResults] = useState<BatchCreateVirtualKeyItem[] | null>(null);
  const [copiedAll, setCopiedAll] = useState(false);
  const [copiedIdx, setCopiedIdx] = useState<number | null>(null);

  const update = <K extends keyof SharedSettings>(key: K, value: SharedSettings[K]) => {
    setSettings((prev) => ({ ...prev, [key]: value }));
  };

  const handleCreate = async () => {
    setError(null);

    const trimmedPrefix = settings.namePrefix.trim();
    if (!trimmedPrefix) {
      setError(t("virtualKeys.batch.namePrefixRequired"));
      return;
    }
    const count = Math.floor(Number(settings.count));
    if (!Number.isFinite(count) || count < 1 || count > MAX_COUNT) {
      setError(t("virtualKeys.batch.countInvalid", { max: MAX_COUNT }));
      return;
    }

    const dailyCents = dollarsToCents(settings.dailyBudget);
    const monthlyCents = dollarsToCents(settings.monthlyBudget);
    if (settings.dailyBudget.trim() !== "" && dailyCents === null) {
      setError(t("virtualKeys.dailyBudgetInvalid"));
      return;
    }
    if (settings.monthlyBudget.trim() !== "" && monthlyCents === null) {
      setError(t("virtualKeys.monthlyBudgetInvalid"));
      return;
    }

    const parsedRpm =
      settings.rpmLimit.trim() === "" ? null : Number(settings.rpmLimit.trim());
    const parsedTpm =
      settings.tpmLimit.trim() === "" ? null : Number(settings.tpmLimit.trim());
    if (parsedRpm !== null && (!Number.isFinite(parsedRpm) || parsedRpm < 0)) {
      setError(t("virtualKeys.rpmLimitInvalid"));
      return;
    }
    if (parsedTpm !== null && (!Number.isFinite(parsedTpm) || parsedTpm < 0)) {
      setError(t("virtualKeys.tpmLimitInvalid"));
      return;
    }

    const trimmedGroup = settings.group.trim();
    const trimmedExpiry = settings.expiresAt.trim();

    const payload: BatchCreateVirtualKeyData = {
      count,
      name_prefix: trimmedPrefix,
      daily_budget_cents: dailyCents,
      monthly_budget_cents: monthlyCents,
      allowed_models: settings.allowedModels.trim()
        ? settings.allowedModels.split(",").map((s) => s.trim()).filter(Boolean)
        : null,
      allowed_ips: settings.allowedIps
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean),
      rpm_limit: parsedRpm,
      tpm_limit: parsedTpm,
      expires_at: trimmedExpiry ? new Date(trimmedExpiry).toISOString() : null,
      group: trimmedGroup ? trimmedGroup : null,
    };

    setSubmitting(true);
    try {
      const items = await api.virtualKeys.batchCreate(payload);
      setResults(items);
      toast.success(t("virtualKeys.batch.createdCount", { count: items.length }));
      onCreated();
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : t("virtualKeys.batch.createFailed");
      setError(msg);
      toast.error(msg);
    } finally {
      setSubmitting(false);
    }
  };

  const handleCopyAll = async () => {
    if (!results) return;
    const text = results.map((r) => r.key).join("\n");
    try {
      await navigator.clipboard.writeText(text);
      setCopiedAll(true);
      toast.success(t("virtualKeys.batch.copiedAll"));
      setTimeout(() => setCopiedAll(false), 2000);
    } catch {
      toast.error(t("common.failed"));
    }
  };

  const handleCopyOne = async (item: BatchCreateVirtualKeyItem, idx: number) => {
    try {
      await navigator.clipboard.writeText(item.key);
      setCopiedIdx(idx);
      toast.success(t("common.copiedToClipboard"));
      setTimeout(() => setCopiedIdx(null), 2000);
    } catch {
      toast.error(t("common.failed"));
    }
  };

  const handleClose = () => {
    if (submitting) return;
    onClose();
  };

  return (
    <div
      className="vk-batch-backdrop"
      role="dialog"
      aria-modal="true"
      aria-labelledby="vk-batch-title"
      onClick={(e) => {
        if (e.target === e.currentTarget) handleClose();
      }}
    >
      <div className="vk-batch-modal">
        <div className="vk-batch-modal-header">
          <h3 id="vk-batch-title" className="form-title">
            {results ? t("virtualKeys.batch.resultsTitle") : t("virtualKeys.batch.title")}
          </h3>
          <button
            type="button"
            className="btn btn-sm"
            onClick={handleClose}
            disabled={submitting}
            aria-label={t("common.close")}
          >
            <X size={16} />
          </button>
        </div>

        {!results && (
          <>
            <div className="form-grid">
              <label className="form-field">
                <span>{t("virtualKeys.batch.count")}</span>
                <input
                  type="number"
                  min={1}
                  max={MAX_COUNT}
                  value={settings.count}
                  onChange={(e) => update("count", e.target.value)}
                  required
                />
              </label>
              <label className="form-field">
                <span>{t("virtualKeys.batch.namePrefix")}</span>
                <input
                  value={settings.namePrefix}
                  onChange={(e) => update("namePrefix", e.target.value)}
                  placeholder={t("virtualKeys.batch.namePrefixHint")}
                  required
                />
              </label>
            </div>

            <div className="form-grid">
              <label className="form-field">
                <span>{t("virtualKeys.dailyBudget")}</span>
                <input
                  value={settings.dailyBudget}
                  onChange={(e) => update("dailyBudget", e.target.value)}
                  placeholder={t("virtualKeys.dailyBudgetPlaceholder")}
                  inputMode="decimal"
                />
              </label>
              <label className="form-field">
                <span>{t("virtualKeys.monthlyBudget")}</span>
                <input
                  value={settings.monthlyBudget}
                  onChange={(e) => update("monthlyBudget", e.target.value)}
                  placeholder={t("virtualKeys.monthlyBudgetPlaceholder")}
                  inputMode="decimal"
                />
              </label>
            </div>

            <div className="form-grid">
              <label className="form-field">
                <span>{t("virtualKeys.allowedModels")}</span>
                <input
                  value={settings.allowedModels}
                  onChange={(e) => update("allowedModels", e.target.value)}
                  placeholder={t("virtualKeys.allowedModelsHint")}
                />
              </label>
              <label className="form-field">
                <span>{t("virtualKeys.allowedIps")}</span>
                <input
                  value={settings.allowedIps}
                  onChange={(e) => update("allowedIps", e.target.value)}
                  placeholder={t("virtualKeys.allowedIpsHint")}
                />
              </label>
            </div>

            <div className="form-grid">
              <label className="form-field">
                <span>{t("virtualKeys.group")}</span>
                <input
                  value={settings.group}
                  onChange={(e) => update("group", e.target.value)}
                  placeholder={t("virtualKeys.groupPlaceholder")}
                />
              </label>
              <label className="form-field">
                <span>{t("virtualKeys.rpmLimit")}</span>
                <input
                  value={settings.rpmLimit}
                  onChange={(e) => update("rpmLimit", e.target.value)}
                  placeholder={t("virtualKeys.rpmLimitPlaceholder")}
                  inputMode="numeric"
                />
              </label>
              <label className="form-field">
                <span>{t("virtualKeys.tpmLimit")}</span>
                <input
                  value={settings.tpmLimit}
                  onChange={(e) => update("tpmLimit", e.target.value)}
                  placeholder={t("virtualKeys.tpmLimitPlaceholder")}
                  inputMode="numeric"
                />
              </label>
              <label className="form-field">
                <span>{t("virtualKeys.expiresAt")}</span>
                <input
                  type="datetime-local"
                  value={settings.expiresAt}
                  onChange={(e) => update("expiresAt", e.target.value)}
                />
              </label>
            </div>

            {error && <div className="form-error">{error}</div>}

            <div className="vk-plaintext-warning">
              {t("virtualKeys.batch.plaintextWarning")}
            </div>

            <div style={{ display: "flex", gap: "var(--space-2)" }}>
              <button
                type="button"
                className="btn btn-primary"
                onClick={handleCreate}
                disabled={submitting}
              >
                {submitting ? t("common.saving") : t("virtualKeys.batch.createButton")}
              </button>
              <button
                type="button"
                className="btn"
                onClick={handleClose}
                disabled={submitting}
              >
                {t("common.cancel")}
              </button>
            </div>
          </>
        )}

        {results && (
          <>
            <div className="vk-plaintext-warning">
              {t("virtualKeys.plaintextWarning")}
            </div>
            <div className="vk-batch-results-toolbar">
              <span className="meta-tag">
                {t("virtualKeys.batch.resultsCount", { count: results.length })}
              </span>
              <button
                type="button"
                className={`btn btn-sm ${copiedAll ? "btn-primary" : ""}`}
                onClick={handleCopyAll}
              >
                {copiedAll ? <CheckCheck size={14} /> : <Copy size={14} />}
                {copiedAll
                  ? t("common.copied")
                  : t("virtualKeys.batch.copyAll")}
              </button>
            </div>
            <div className="vk-batch-results-table-wrapper">
              <table className="vk-batch-results-table">
                <thead>
                  <tr>
                    <th>#</th>
                    <th>{t("virtualKeys.keyName")}</th>
                    <th>{t("virtualKeys.plaintextKey")}</th>
                    <th></th>
                  </tr>
                </thead>
                <tbody>
                  {results.map((item, idx) => (
                    <tr key={item.id}>
                      <td className="mono">{idx + 1}</td>
                      <td>{item.name}</td>
                      <td>
                        <code className="mono vk-batch-key-cell">{item.key}</code>
                      </td>
                      <td>
                        <button
                          type="button"
                          className={`btn btn-sm ${copiedIdx === idx ? "btn-primary" : ""}`}
                          onClick={() => handleCopyOne(item, idx)}
                          aria-label={t("common.copy")}
                          title={t("common.copy")}
                        >
                          {copiedIdx === idx ? (
                            <CheckCheck size={14} />
                          ) : (
                            <Copy size={14} />
                          )}
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <div style={{ display: "flex", gap: "var(--space-2)", marginTop: "var(--space-3)" }}>
              <button
                type="button"
                className="btn btn-primary"
                onClick={handleClose}
              >
                {t("common.done")}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
