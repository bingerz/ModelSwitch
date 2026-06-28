import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  CheckCircle2,
  Download,
  FileJson,
  Upload,
} from "lucide-react";
import { api, type Channel } from "../../lib/api";
import { useToast } from "../Toast";

/** Shape of a channel entry serialized into a backup file. */
interface BackupChannel {
  name: string;
  provider: string;
  priority: number;
  weight: number;
  base_url: string;
  model_mapping: Record<string, string>;
  enabled: boolean;
  cost_per_token: number | null;
  input_cost_per_mtok: number | null;
  output_cost_per_mtok: number | null;
  rpm_limit: number | null;
  tpm_limit: number | null;
  tags: string[];
  account_group: string | null;
  excluded_models: string[];
  headers: Record<string, string>;
  proxy_url: string | null;
  max_retries: number | null;
  models_endpoint: string | null;
  models_refresh_interval_secs: number;
}

interface ConfigExport {
  version: string;
  exported_at: string;
  channels: BackupChannel[];
  completion_ratios: Record<string, number>;
  /** Only present when the export was NOT redacted. */
  api_keys?: string[];
  has_sensitive: boolean;
}

const BACKUP_VERSION = "1.0";

function toBackupChannel(ch: Channel, redact: boolean): BackupChannel {
  return {
    name: ch.name,
    provider: ch.provider,
    priority: ch.priority,
    weight: ch.weight,
    base_url: ch.base_url,
    model_mapping: ch.model_mapping,
    enabled: ch.enabled,
    cost_per_token: ch.cost_per_token,
    input_cost_per_mtok: ch.input_cost_per_mtok,
    output_cost_per_mtok: ch.output_cost_per_mtok,
    rpm_limit: ch.rpm_limit,
    tpm_limit: ch.tpm_limit,
    tags: ch.tags,
    account_group: ch.account_group,
    excluded_models: ch.excluded_models,
    // `headers` may carry Authorization values — blank them when redacting.
    headers: redact ? {} : ch.headers,
    proxy_url: ch.proxy_url,
    max_retries: ch.max_retries,
    models_endpoint: ch.models_endpoint,
    models_refresh_interval_secs: ch.models_refresh_interval_secs,
  };
}

function downloadJson(filename: string, payload: unknown): void {
  const blob = new Blob([JSON.stringify(payload, null, 2)], {
    type: "application/json",
  });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

/** Backup & Restore section: export gateway config to JSON, import it back. */
export function BackupRestore() {
  const { t } = useTranslation();
  const toast = useToast();
  const [redactSensitive, setRedactSensitive] = useState(true);
  const [importing, setImporting] = useState(false);
  const [importPreview, setImportPreview] = useState<ConfigExport | null>(null);
  const [importError, setImportError] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);

  const handleExport = async () => {
    setExporting(true);
    try {
      const [channels, ratios] = await Promise.all([
        api.listChannels(),
        api.completionRatios().catch(() => ({} as Record<string, number>)),
      ]);

      const config: ConfigExport = {
        version: BACKUP_VERSION,
        exported_at: new Date().toISOString(),
        channels: channels.map((ch) => toBackupChannel(ch, redactSensitive)),
        completion_ratios: ratios,
        has_sensitive: !redactSensitive,
        // Only include raw api_keys when the user explicitly opted in.
        ...(redactSensitive
          ? {}
          : {
              api_keys: channels.flatMap((ch) => ch.api_keys ?? []),
            }),
      };

      downloadJson(
        `modelswitch-config-${new Date().toISOString().slice(0, 10)}.json`,
        config,
      );

      toast.success(
        t("backup.exportSuccess", { count: config.channels.length }),
      );
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("backup.exportFailed"));
    } finally {
      setExporting(false);
    }
  };

  const handleFileSelect = useCallback((file: File) => {
    setImportError(null);
    setImportPreview(null);
    const reader = new FileReader();
    reader.onload = (e) => {
      try {
        const text = e.target?.result as string;
        const parsed = JSON.parse(text) as ConfigExport;
        if (!parsed.version || !Array.isArray(parsed.channels)) {
          throw new Error("Invalid config format");
        }
        setImportPreview(parsed);
      } catch (err) {
        setImportError(err instanceof Error ? err.message : "Invalid JSON");
      }
    };
    reader.onerror = () => setImportError("Failed to read file");
    reader.readAsText(file);
  }, []);

  const handleImport = async () => {
    if (!importPreview) return;
    setImporting(true);
    let created = 0;
    let skipped = 0;
    try {
      const existing = await api.listChannels();
      const existingNames = new Set(existing.map((c) => c.name));

      for (const ch of importPreview.channels) {
        if (existingNames.has(ch.name)) {
          skipped++;
          continue;
        }
        try {
          await api.createChannel({
            name: ch.name,
            provider: ch.provider,
            priority: ch.priority,
            weight: ch.weight,
            base_url: ch.base_url,
            model_mapping: ch.model_mapping,
            enabled: ch.enabled,
            cost_per_token: ch.cost_per_token,
            input_cost_per_mtok: ch.input_cost_per_mtok,
            output_cost_per_mtok: ch.output_cost_per_mtok,
            rpm_limit: ch.rpm_limit,
            tpm_limit: ch.tpm_limit,
            tags: ch.tags,
            account_group: ch.account_group,
            excluded_models: ch.excluded_models,
            headers: ch.headers,
            proxy_url: ch.proxy_url,
            max_retries: ch.max_retries,
            models_endpoint: ch.models_endpoint,
            models_refresh_interval_secs: ch.models_refresh_interval_secs,
            credential_value: "",
            credential_type: "api_key",
          });
          created++;
        } catch {
          skipped++;
        }
      }

      if (
        importPreview.completion_ratios &&
        Object.keys(importPreview.completion_ratios).length > 0
      ) {
        try {
          await api.updateCompletionRatios(importPreview.completion_ratios);
        } catch {
          // Non-critical — ratios update is best-effort.
        }
      }

      toast.success(t("backup.importSuccess", { created, skipped }));
      setImportPreview(null);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("backup.importFailed"));
    } finally {
      setImporting(false);
    }
  };

  const ratioCount = importPreview?.completion_ratios
    ? Object.keys(importPreview.completion_ratios).length
    : 0;

  return (
    <div className="settings-section">
      <h3 className="settings-section-title">
        <FileJson size={14} className="icon-inline" />
        {t("backup.title")}
      </h3>

      {/* Export */}
      <div className="backup-card">
        <div className="backup-card-header">
          <Download size={18} />
          <h4>{t("backup.exportTitle")}</h4>
        </div>
        <p className="backup-card-desc">{t("backup.exportDesc")}</p>
        <label className="backup-checkbox">
          <input
            type="checkbox"
            checked={redactSensitive}
            onChange={(e) => setRedactSensitive(e.target.checked)}
          />
          <span>{t("backup.redactSensitive")}</span>
        </label>
        <button
          className="btn btn-sm btn-primary"
          onClick={handleExport}
          disabled={exporting}
        >
          <Download size={14} />
          {exporting ? t("common.loading") : t("backup.exportButton")}
        </button>
      </div>

      {/* Import */}
      <div className="backup-card">
        <div className="backup-card-header">
          <Upload size={18} />
          <h4>{t("backup.importTitle")}</h4>
        </div>
        <p className="backup-card-desc">{t("backup.importDesc")}</p>

        <label className="backup-dropzone">
          <input
            type="file"
            accept="application/json,.json"
            onChange={(e) => {
              const file = e.target.files?.[0];
              if (file) handleFileSelect(file);
              // Allow re-selecting the same file.
              e.target.value = "";
            }}
            style={{ display: "none" }}
          />
          <Upload size={24} />
          <span>{t("backup.dropzone")}</span>
        </label>

        {importError && (
          <div className="backup-error">
            <AlertTriangle size={14} />
            {importError}
          </div>
        )}

        {importPreview && (
          <div className="backup-preview">
            <div className="backup-preview-header">
              <CheckCircle2 size={16} />
              <span>{t("backup.previewTitle")}</span>
            </div>
            <div className="backup-preview-stats">
              <span>
                {t("backup.channels", {
                  count: importPreview.channels.length,
                })}
              </span>
              {ratioCount > 0 && (
                <span>{t("backup.ratios", { count: ratioCount })}</span>
              )}
            </div>
            {importPreview.has_sensitive ? (
              <div className="backup-warning">
                <AlertTriangle size={12} />
                {t("backup.containsSensitive")}
              </div>
            ) : (
              <div className="backup-info">{t("backup.noCredentialsNote")}</div>
            )}
            <button
              className="btn btn-sm btn-primary"
              onClick={handleImport}
              disabled={importing}
            >
              {importing ? t("common.loading") : t("backup.applyImport")}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
