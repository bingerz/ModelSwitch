import { useState, useRef, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { Copy, CheckCheck, X, Upload, FileText } from "lucide-react";
import {
  api,
  type CreateVirtualKeyResponse,
} from "../../lib/api";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import { useToast } from "../Toast";
import { dollarsToCents } from "./types";
import { parseCsv, type ParsedRow } from "./csv-parser";

export interface CsvImportModalProps {
  onClose: () => void;
  onCreated: () => void;
}

interface SharedSettings {
  dailyBudget: string;
  monthlyBudget: string;
  allowedModels: string;
  allowedIps: string;
  group: string;
  rpmLimit: string;
  tpmLimit: string;
  expiresAt: string;
}

interface CreationResult {
  name: string;
  group: string | null;
  plaintext: string;
  error: string | null;
}

type TFunc = ReturnType<typeof useTranslation>["t"];

const INITIAL_SETTINGS: SharedSettings = {
  dailyBudget: "",
  monthlyBudget: "",
  allowedModels: "",
  allowedIps: "",
  group: "",
  rpmLimit: "",
  tpmLimit: "",
  expiresAt: "",
};

const BATCH_SIZE = 10;

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

interface FileDropzoneProps {
  onFileSelected: (file: File) => void;
  t: TFunc;
}

function FileDropzone({ onFileSelected, t }: FileDropzoneProps) {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [dragOver, setDragOver] = useState(false);

  const handleFileInput = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) onFileSelected(file);
  };

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(false);
    const file = e.dataTransfer.files?.[0];
    if (file) onFileSelected(file);
  };

  return (
    <div
      className="vk-csv-dropzone"
      style={{
        border: dragOver
          ? "2px dashed var(--color-accent)"
          : "2px dashed var(--color-border)",
        borderRadius: "var(--radius-md, 8px)",
        padding: "var(--space-4, 1.5rem)",
        textAlign: "center",
        cursor: "pointer",
        transition: "border-color 0.15s ease",
      }}
      onClick={() => fileInputRef.current?.click()}
      onDragOver={(e) => {
        e.preventDefault();
        setDragOver(true);
      }}
      onDragLeave={() => setDragOver(false)}
      onDrop={handleDrop}
    >
      <Upload size={32} style={{ opacity: 0.5, marginBottom: "0.5rem" }} />
      <div style={{ fontWeight: 500, marginBottom: "0.25rem" }}>
        {t("virtualKeys.csv.dropHere")}
      </div>
      <div style={{ fontSize: "0.85em", color: "var(--color-text-muted)" }}>
        {t("virtualKeys.csv.dropHint")}
      </div>
      <input
        ref={fileInputRef}
        type="file"
        accept=".csv,text/csv"
        onChange={handleFileInput}
        style={{ display: "none" }}
      />
    </div>
  );
}

interface ParseErrorListProps {
  errors: string[];
}

function ParseErrorList({ errors }: ParseErrorListProps) {
  if (errors.length === 0) return null;
  return (
    <div className="form-error mt-2">
      {errors.map((e, i) => (
        <div key={i}>{e}</div>
      ))}
    </div>
  );
}

interface SharedSettingsFormProps {
  settings: SharedSettings;
  onChange: <K extends keyof SharedSettings>(key: K, value: SharedSettings[K]) => void;
  t: TFunc;
}

function SharedSettingsForm({ settings, onChange, t }: SharedSettingsFormProps) {
  return (
    <div className="mt-3">
      <div
        className="form-title"
        style={{ fontSize: "0.9em", marginBottom: "var(--space-2)" }}
      >
        {t("virtualKeys.csv.sharedSettings")}
      </div>

      <div className="form-grid">
        <label className="form-field">
          <span>{t("virtualKeys.dailyBudget")}</span>
          <input
            value={settings.dailyBudget}
            onChange={(e) => onChange("dailyBudget", e.target.value)}
            placeholder={t("virtualKeys.dailyBudgetPlaceholder")}
            inputMode="decimal"
          />
        </label>
        <label className="form-field">
          <span>{t("virtualKeys.monthlyBudget")}</span>
          <input
            value={settings.monthlyBudget}
            onChange={(e) => onChange("monthlyBudget", e.target.value)}
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
            onChange={(e) => onChange("allowedModels", e.target.value)}
            placeholder={t("virtualKeys.allowedModelsHint")}
          />
        </label>
        <label className="form-field">
          <span>{t("virtualKeys.allowedIps")}</span>
          <input
            value={settings.allowedIps}
            onChange={(e) => onChange("allowedIps", e.target.value)}
            placeholder={t("virtualKeys.allowedIpsHint")}
          />
        </label>
      </div>

      <div className="form-grid">
        <label className="form-field">
          <span>{t("virtualKeys.group")}</span>
          <input
            value={settings.group}
            onChange={(e) => onChange("group", e.target.value)}
            placeholder={t("virtualKeys.groupPlaceholder")}
          />
        </label>
        <label className="form-field">
          <span>{t("virtualKeys.rpmLimit")}</span>
          <input
            value={settings.rpmLimit}
            onChange={(e) => onChange("rpmLimit", e.target.value)}
            placeholder={t("virtualKeys.rpmLimitPlaceholder")}
            inputMode="numeric"
          />
        </label>
        <label className="form-field">
          <span>{t("virtualKeys.tpmLimit")}</span>
          <input
            value={settings.tpmLimit}
            onChange={(e) => onChange("tpmLimit", e.target.value)}
            placeholder={t("virtualKeys.tpmLimitPlaceholder")}
            inputMode="numeric"
          />
        </label>
        <label className="form-field">
          <span>{t("virtualKeys.expiresAt")}</span>
          <input
            type="datetime-local"
            value={settings.expiresAt}
            onChange={(e) => onChange("expiresAt", e.target.value)}
          />
        </label>
      </div>
    </div>
  );
}

interface ResultsTableProps {
  results: CreationResult[];
  copiedAll: boolean;
  copiedIdx: number | null;
  onCopyAll: () => void;
  onCopyIdx: (item: CreationResult, idx: number) => void;
  t: TFunc;
}

function ResultsTable({
  results,
  copiedAll,
  copiedIdx,
  onCopyAll,
  onCopyIdx,
  t,
}: ResultsTableProps) {
  const successCount = results.filter((r) => r.error === null).length;

  return (
    <>
      <div className="vk-plaintext-warning">
        {t("virtualKeys.batch.plaintextWarning")}
      </div>
      <div className="vk-batch-results-toolbar">
        <span className="meta-tag">
          {t("virtualKeys.csv.resultsSummary", {
            success: successCount,
            total: results.length,
          })}
        </span>
        {successCount > 0 && (
          <button
            type="button"
            className={`btn btn-sm ${copiedAll ? "btn-primary" : ""}`}
            onClick={onCopyAll}
          >
            {copiedAll ? <CheckCheck size={14} /> : <Copy size={14} />}
            {copiedAll ? t("common.copied") : t("virtualKeys.batch.copyAll")}
          </button>
        )}
      </div>
      <div className="vk-batch-results-table-wrapper">
        <table className="vk-batch-results-table">
          <thead>
            <tr>
              <th>#</th>
              <th>{t("virtualKeys.keyName")}</th>
              <th>{t("virtualKeys.plaintextKey")}</th>
              <th>{t("common.status")}</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {results.map((item, idx) => (
              <tr key={idx}>
                <td className="mono">{idx + 1}</td>
                <td>{item.name}</td>
                <td>
                  {item.plaintext ? (
                    <code className="mono vk-batch-key-cell">{item.plaintext}</code>
                  ) : (
                    <span style={{ color: "var(--color-danger)" }}>--</span>
                  )}
                </td>
                <td>
                  {item.error ? (
                    <span
                      style={{ color: "var(--color-danger)", fontSize: "0.85em" }}
                    >
                      {item.error}
                    </span>
                  ) : (
                    <span
                      style={{ color: "var(--color-success)", fontSize: "0.85em" }}
                    >
                      {t("common.success")}
                    </span>
                  )}
                </td>
                <td>
                  {item.plaintext && (
                    <button
                      type="button"
                      className={`btn btn-sm ${copiedIdx === idx ? "btn-primary" : ""}`}
                      onClick={() => onCopyIdx(item, idx)}
                      aria-label={t("common.copy")}
                      title={t("common.copy")}
                    >
                      {copiedIdx === idx ? (
                        <CheckCheck size={14} />
                      ) : (
                        <Copy size={14} />
                      )}
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </>
  );
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

export function CsvImportModal({ onClose, onCreated }: CsvImportModalProps) {
  const { t } = useTranslation();
  const toast = useToast();
  const modalRef = useRef<HTMLDivElement>(null);

  const [parsedRows, setParsedRows] = useState<ParsedRow[] | null>(null);
  const [parseErrors, setParseErrors] = useState<string[]>([]);
  const [fileName, setFileName] = useState<string>("");
  const [settings, setSettings] = useState<SharedSettings>(INITIAL_SETTINGS);
  const [submitting, setSubmitting] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [results, setResults] = useState<CreationResult[] | null>(null);
  const [copiedAll, setCopiedAll] = useState(false);
  const [copiedIdx, setCopiedIdx] = useState<number | null>(null);

  const update = useCallback(
    <K extends keyof SharedSettings>(key: K, value: SharedSettings[K]) => {
      setSettings((prev) => ({ ...prev, [key]: value }));
    },
    [],
  );

  const handleFile = useCallback(
    (file: File) => {
      setError(null);
      setResults(null);
      setParsedRows(null);
      setParseErrors([]);

      if (!file.name.toLowerCase().endsWith(".csv")) {
        setError(t("virtualKeys.csv.notCsv"));
        return;
      }

      setFileName(file.name);
      const reader = new FileReader();
      reader.onload = (e) => {
        const text = (e.target?.result as string) ?? "";
        const result = parseCsv(text);
        setParsedRows(result.rows);
        setParseErrors(result.errors);
        if (result.rows.length === 0 && result.errors.length === 0) {
          setParseErrors([t("virtualKeys.csv.noDataRows")]);
        }
      };
      reader.onerror = () => {
        setError(t("virtualKeys.csv.readFailed"));
      };
      reader.readAsText(file);
    },
    [t],
  );

  const handleCreate = async () => {
    if (!parsedRows || parsedRows.length === 0) return;
    setError(null);

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
    const allowedModels = settings.allowedModels.trim()
      ? settings.allowedModels.split(",").map((s) => s.trim()).filter(Boolean)
      : null;
    const allowedIps = settings.allowedIps
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);

    setSubmitting(true);
    setProgress({ done: 0, total: parsedRows.length });

    const allResults: CreationResult[] = [];

    for (let i = 0; i < parsedRows.length; i += BATCH_SIZE) {
      const batch = parsedRows.slice(i, i + BATCH_SIZE);
      const promises = batch.map(async (row) => {
        try {
          const response: CreateVirtualKeyResponse = await api.virtualKeys.create({
            name: row.name,
            daily_budget_cents: dailyCents,
            monthly_budget_cents: monthlyCents,
            allowed_models: allowedModels,
            allowed_ips: allowedIps,
            rpm_limit: parsedRpm,
            tpm_limit: parsedTpm,
            expires_at: trimmedExpiry ? new Date(trimmedExpiry).toISOString() : null,
            group: row.group ?? (trimmedGroup || null),
          });
          return {
            name: response.key.name,
            group: response.key.group,
            plaintext: response.plaintext,
            error: null,
          } as CreationResult;
        } catch (err) {
          return {
            name: row.name,
            group: row.group,
            plaintext: "",
            error: err instanceof Error ? err.message : t("virtualKeys.csv.createFailed"),
          } as CreationResult;
        }
      });

      const batchResults = await Promise.all(promises);
      allResults.push(...batchResults);
      setProgress({
        done: Math.min(i + BATCH_SIZE, parsedRows.length),
        total: parsedRows.length,
      });
    }

    setResults(allResults);
    setSubmitting(false);
    setProgress(null);

    const successCount = allResults.filter((r) => r.error === null).length;
    if (successCount > 0) {
      toast.success(t("virtualKeys.csv.createdCount", { count: successCount }));
      onCreated();
    }
    const failCount = allResults.length - successCount;
    if (failCount > 0) {
      toast.error(t("virtualKeys.csv.failedCount", { count: failCount }));
    }
  };

  const handleCopyAll = async () => {
    if (!results) return;
    const text = results
      .filter((r) => r.plaintext)
      .map((r) => r.plaintext)
      .join("\n");
    try {
      await navigator.clipboard.writeText(text);
      setCopiedAll(true);
      toast.success(t("virtualKeys.batch.copiedAll"));
      setTimeout(() => setCopiedAll(false), 2000);
    } catch {
      toast.error(t("common.failed"));
    }
  };

  const handleCopyOne = async (item: CreationResult, idx: number) => {
    try {
      await navigator.clipboard.writeText(item.plaintext);
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

  useFocusTrap(modalRef, true, handleClose);

  return (
    <div
      ref={modalRef}
      className="vk-batch-backdrop"
      role="dialog"
      aria-modal="true"
      aria-labelledby="vk-csv-title"
      onClick={(e) => {
        if (e.target === e.currentTarget) handleClose();
      }}
    >
      <div className="vk-batch-modal">
        <div className="vk-batch-modal-header">
          <h3 id="vk-csv-title" className="form-title">
            {results
              ? t("virtualKeys.csv.resultsTitle")
              : t("virtualKeys.csv.title")}
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

        {/* Results view */}
        {results && (
          <>
            <ResultsTable
              results={results}
              copiedAll={copiedAll}
              copiedIdx={copiedIdx}
              onCopyAll={handleCopyAll}
              onCopyIdx={handleCopyOne}
              t={t}
            />
            <div
              style={{
                display: "flex",
                gap: "var(--space-2)",
                marginTop: "var(--space-3)",
              }}
            >
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

        {/* Setup + preview view */}
        {!results && (
          <>
            {/* File input / drop zone */}
            {!parsedRows && <FileDropzone onFileSelected={handleFile} t={t} />}

            {/* Parse errors */}
            <ParseErrorList errors={parseErrors} />

            {/* Preview table + shared settings */}
            {parsedRows && parsedRows.length > 0 && (
              <>
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "var(--space-2)",
                    marginBottom: "var(--space-2)",
                  }}
                >
                  <FileText size={16} />
                  <span style={{ fontWeight: 500 }}>{fileName}</span>
                  <span className="meta-tag">
                    {t("virtualKeys.csv.rowsFound", { count: parsedRows.length })}
                  </span>
                  <button
                    type="button"
                    className="btn btn-sm"
                    onClick={() => {
                      setParsedRows(null);
                      setFileName("");
                      setParseErrors([]);
                    }}
                  >
                    {t("virtualKeys.csv.chooseAnother")}
                  </button>
                </div>

                <div
                  className="vk-batch-results-table-wrapper"
                  style={{ maxHeight: "200px" }}
                >
                  <table className="vk-batch-results-table">
                    <thead>
                      <tr>
                        <th>#</th>
                        <th>{t("virtualKeys.keyName")}</th>
                        <th>{t("virtualKeys.group")}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {parsedRows.slice(0, 50).map((row, idx) => (
                        <tr key={idx}>
                          <td className="mono">{idx + 1}</td>
                          <td>{row.name}</td>
                          <td>{row.group ?? "-"}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
                {parsedRows.length > 50 && (
                  <div
                    style={{
                      fontSize: "0.85em",
                      color: "var(--color-text-muted)",
                      marginTop: "0.25rem",
                    }}
                  >
                    {t("virtualKeys.csv.showingFirst", {
                      shown: 50,
                      total: parsedRows.length,
                    })}
                  </div>
                )}

                {/* Shared settings */}
                <SharedSettingsForm settings={settings} onChange={update} t={t} />

                {error && <div className="form-error">{error}</div>}

                <div className="vk-plaintext-warning">
                  {t("virtualKeys.batch.plaintextWarning")}
                </div>
              </>
            )}

            {/* Action buttons */}
            <div
              style={{
                display: "flex",
                gap: "var(--space-2)",
                marginTop: "var(--space-3)",
              }}
            >
              <button
                type="button"
                className="btn btn-primary"
                onClick={handleCreate}
                disabled={submitting || !parsedRows || parsedRows.length === 0}
              >
                {submitting && progress
                  ? t("virtualKeys.csv.creating", {
                      done: progress.done,
                      total: progress.total,
                    })
                  : t("virtualKeys.csv.createButton")}
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
      </div>
    </div>
  );
}
