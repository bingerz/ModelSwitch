import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Copy, CheckCheck, X } from "lucide-react";
import {
  api,
  type BatchCreateVirtualKeyItem,
} from "../../lib/api";
import { useToast } from "../Toast";
import {
  validateBatchCreateInput,
  buildBatchPayload,
  MAX_BATCH_COUNT,
  type BatchCreateSettings,
  type BatchValidationError,
} from "./batch-validate";

export interface BatchCreateModalProps {
  onClose: () => void;
  onCreated: () => void;
}

type SharedSettings = BatchCreateSettings;

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

// Maps validator error codes to existing i18n keys. Note budget/limit errors
// live directly under `virtualKeys.*` (not `virtualKeys.batch.*`), so we cannot
// build the key by naive concatenation.
const ERROR_I18N_KEYS: Record<BatchValidationError, string> = {
  namePrefixRequired: "virtualKeys.batch.namePrefixRequired",
  countInvalid: "virtualKeys.batch.countInvalid",
  dailyBudgetInvalid: "virtualKeys.dailyBudgetInvalid",
  monthlyBudgetInvalid: "virtualKeys.monthlyBudgetInvalid",
  rpmLimitInvalid: "virtualKeys.rpmLimitInvalid",
  tpmLimitInvalid: "virtualKeys.tpmLimitInvalid",
};

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

    const errorCode = validateBatchCreateInput(settings);
    if (errorCode) {
      setError(
        t(ERROR_I18N_KEYS[errorCode], { max: MAX_BATCH_COUNT }),
      );
      return;
    }

    const payload = buildBatchPayload(settings);

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
                  max={MAX_BATCH_COUNT}
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
