import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Copy, CheckCheck } from "lucide-react";
import { useToast } from "../Toast";

/**
 * A single row in the batch results table.
 *
 * `id` is used as the React key. `key` is the plaintext key (empty string when
 * the row represents a failure). `error` is `null` or `undefined` for success
 * rows; when set, the row is treated as a failure and the message is shown in
 * the status column.
 */
export interface BatchResult {
  id: string;
  name: string;
  key: string;
  error?: string | null;
}

export interface BatchResultsTableProps {
  results: BatchResult[];
  /**
   * React node rendered inside the toolbar's meta tag (e.g. "3 / 5 succeeded"
   * or "5 keys"). Each modal composes its own i18n string.
   */
  summary: ReactNode;
  /**
   * Whether to render the status column. Defaults to `true`. Set to `false`
   * when all rows are guaranteed successes (e.g. BatchCreateModal, where the
   * API call throws on failure).
   */
  showStatusColumn?: boolean;
}

const COPY_RESET_MS = 2000;

export function BatchResultsTable({
  results,
  summary,
  showStatusColumn = true,
}: BatchResultsTableProps) {
  const { t } = useTranslation();
  const toast = useToast();
  const [copiedAll, setCopiedAll] = useState(false);
  const [copiedIdx, setCopiedIdx] = useState<number | null>(null);

  const hasAnyKey = results.some((r) => Boolean(r.key));

  const handleCopyAll = async () => {
    const text = results
      .filter((r) => r.key)
      .map((r) => r.key)
      .join("\n");
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopiedAll(true);
      toast.success(t("virtualKeys.batch.copiedAll"));
      setTimeout(() => setCopiedAll(false), COPY_RESET_MS);
    } catch {
      toast.error(t("common.failed"));
    }
  };

  const handleCopyOne = async (key: string, idx: number) => {
    try {
      await navigator.clipboard.writeText(key);
      setCopiedIdx(idx);
      toast.success(t("common.copiedToClipboard"));
      setTimeout(() => setCopiedIdx(null), COPY_RESET_MS);
    } catch {
      toast.error(t("common.failed"));
    }
  };

  return (
    <>
      <div className="vk-batch-results-toolbar">
        <span className="meta-tag">{summary}</span>
        {hasAnyKey && (
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
        )}
      </div>
      <div className="vk-batch-results-table-wrapper">
        <table className="vk-batch-results-table">
          <thead>
            <tr>
              <th>#</th>
              <th>{t("virtualKeys.keyName")}</th>
              <th>{t("virtualKeys.plaintextKey")}</th>
              {showStatusColumn && <th>{t("common.status")}</th>}
              <th></th>
            </tr>
          </thead>
          <tbody>
            {results.map((item, idx) => (
              <tr key={item.id}>
                <td className="mono">{idx + 1}</td>
                <td>{item.name}</td>
                <td>
                  {item.key ? (
                    <code className="mono vk-batch-key-cell">{item.key}</code>
                  ) : (
                    <span className="vk-csv-danger-text">--</span>
                  )}
                </td>
                {showStatusColumn && (
                  <td>
                    {item.error ? (
                      <span className="vk-csv-cell-error">{item.error}</span>
                    ) : (
                      <span className="vk-csv-cell-success">
                        {t("common.success")}
                      </span>
                    )}
                  </td>
                )}
                <td>
                  {item.key && (
                    <button
                      type="button"
                      className={`btn btn-sm ${copiedIdx === idx ? "btn-primary" : ""}`}
                      onClick={() => handleCopyOne(item.key, idx)}
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
