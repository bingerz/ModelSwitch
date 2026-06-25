import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../lib/api";
import { useToast } from "../Toast";

interface BatchOperationsBarProps {
  selectedIds: string[];
  onClear: () => void;
  onDone: () => void;
}

export function BatchOperationsBar({ selectedIds, onClear, onDone }: BatchOperationsBarProps) {
  const { t } = useTranslation();
  const toast = useToast();
  const [loading, setLoading] = useState(false);
  const [showTagInput, setShowTagInput] = useState(false);
  const [tagValue, setTagValue] = useState("");

  if (selectedIds.length === 0) return null;

  const handleBatch = async (
    action: () => Promise<{ updated?: number; deleted?: number }>,
    successKey: string
  ) => {
    setLoading(true);
    try {
      const result = await action();
      toast.success(
        t(successKey, {
          count: result.updated ?? result.deleted ?? 0,
        })
      );
      onClear();
      onDone();
    } catch {
      toast.error(t("channels.batchFailed"));
    } finally {
      setLoading(false);
    }
  };

  const handleTags = async () => {
    const tags = tagValue.split(",").map((s) => s.trim()).filter(Boolean);
    if (tags.length === 0) return;
    setLoading(true);
    try {
      const result = await api.batchUpdateTags(selectedIds, tags);
      toast.success(t("channels.batchTagsUpdated", { count: result.updated }));
      setShowTagInput(false);
      setTagValue("");
      onClear();
      onDone();
    } catch {
      toast.error(t("channels.batchFailed"));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div
      className="batch-operations-bar"
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--space-3)",
        padding: "var(--space-2) var(--space-3)",
        background: "var(--color-bg-secondary)",
        borderRadius: "8px",
        marginBottom: "var(--space-3)",
        flexWrap: "wrap",
      }}
    >
      <span className="mono" style={{ fontSize: "var(--text-sm)" }}>
        {t("channels.selected", { count: selectedIds.length })}
      </span>

      <button
        className="btn btn-sm"
        onClick={() => handleBatch(() => api.batchEnableChannels(selectedIds), "channels.batchEnabled")}
        disabled={loading}
      >
        {t("common.enable")}
      </button>

      <button
        className="btn btn-sm"
        onClick={() => handleBatch(() => api.batchDisableChannels(selectedIds), "channels.batchDisabled")}
        disabled={loading}
      >
        {t("common.disable")}
      </button>

      <button
        className="btn btn-sm"
        onClick={() => setShowTagInput(!showTagInput)}
        disabled={loading}
      >
        {t("channels.setTag")}
      </button>

      {showTagInput && (
        <>
          <input
            type="text"
            className="settings-input"
            value={tagValue}
            onChange={(e) => setTagValue(e.target.value)}
            placeholder={t("channels.tagCommaSeparated")}
            style={{ width: "200px" }}
            onKeyDown={(e) => e.key === "Enter" && handleTags()}
          />
          <button className="btn btn-sm btn-primary" onClick={handleTags} disabled={loading || !tagValue.trim()}>
            {t("common.confirm")}
          </button>
        </>
      )}

      <button
        className="btn btn-sm btn-danger"
        onClick={() => handleBatch(() => api.batchDeleteChannels(selectedIds), "channels.batchDeleted")}
        disabled={loading}
      >
        {t("common.delete")}
      </button>

      <button className="btn btn-sm" onClick={onClear} disabled={loading}>
        {t("common.clear")}
      </button>
    </div>
  );
}
