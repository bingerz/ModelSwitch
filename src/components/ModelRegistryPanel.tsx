import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Database, Search } from "lucide-react";
import { SectionHeader } from "./ui/SectionHeader";
import { api, type ModelRegistryItem } from "../lib/api";

type SourceFilter = "all" | "builtin" | "discovered";

function formatRelative(epochSecs: number | null, t: (key: string, opts?: Record<string, unknown>) => string): string {
  if (epochSecs === null) return "—";
  const nowSecs = Math.floor(Date.now() / 1000);
  const delta = Math.max(0, nowSecs - epochSecs);
  if (delta < 60) return t("registry.justNow");
  if (delta < 3600) return t("registry.minutesAgo", { count: Math.floor(delta / 60) });
  if (delta < 86400) return t("registry.hoursAgo", { count: Math.floor(delta / 3600) });
  return t("registry.daysAgo", { count: Math.floor(delta / 86400) });
}

function formatContextSize(tokens: number | null): string {
  if (tokens === null) return "—";
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`;
  if (tokens >= 1_000) return `${Math.round(tokens / 1000)}K`;
  return String(tokens);
}

export function ModelRegistryPanel() {
  const { t } = useTranslation();
  const [items, setItems] = useState<ModelRegistryItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [query, setQuery] = useState("");
  const [sourceFilter, setSourceFilter] = useState<SourceFilter>("all");
  const [groupByChannel, setGroupByChannel] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const data = await api.modelRegistry();
      setItems(data.models);
    } catch {
      // silently fail — section header shows refresh state
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
    const interval = setInterval(refresh, 30000);
    return () => clearInterval(interval);
  }, [refresh]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return items.filter((item) => {
      if (sourceFilter !== "all" && item.source_type !== sourceFilter) return false;
      if (!q) return true;
      return (
        item.name.toLowerCase().includes(q) ||
        (item.channel_name?.toLowerCase().includes(q) ?? false) ||
        (item.source?.toLowerCase().includes(q) ?? false)
      );
    });
  }, [items, query, sourceFilter]);

  const totalDiscovered = useMemo(
    () => items.filter((i) => i.source_type === "discovered").length,
    [items]
  );

  // Group by channel (discovered models only have channel info)
  const grouped = useMemo(() => {
    if (!groupByChannel) return null;
    const map = new Map<string, ModelRegistryItem[]>();
    for (const item of filtered) {
      const key = item.channel_name ?? (item.source_type === "builtin" ? t("registry.builtin") : t("registry.unknownSource"));
      const arr = map.get(key) ?? [];
      arr.push(item);
      map.set(key, arr);
    }
    return Array.from(map.entries()).sort(([a], [b]) => a.localeCompare(b));
  }, [filtered, groupByChannel, t]);

  const filterSelect = (
    <div className="registry-controls">
      <div className="registry-search">
        <Search size={14} className="registry-search-icon" />
        <input
          type="text"
          className="registry-search-input"
          placeholder={t("registry.searchPlaceholder")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          aria-label={t("common.search")}
        />
      </div>
      <select
        className="registry-filter-select"
        value={sourceFilter}
        onChange={(e) => setSourceFilter(e.target.value as SourceFilter)}
        aria-label={t("registry.sourceFilter")}
      >
        <option value="all">{t("registry.allSources")}</option>
        <option value="builtin">{t("registry.builtin")}</option>
        <option value="discovered">{t("registry.discovered")}</option>
      </select>
      <label className="registry-group-toggle">
        <input
          type="checkbox"
          checked={groupByChannel}
          onChange={(e) => setGroupByChannel(e.target.checked)}
        />
        <span>{t("registry.groupByChannel")}</span>
      </label>
    </div>
  );

  const renderTable = (rows: ModelRegistryItem[]) => (
    <table className="settings-table">
      <thead>
        <tr>
          <th>{t("registry.modelName")}</th>
          <th>{t("registry.source")}</th>
          <th>{t("registry.channel")}</th>
          <th>{t("registry.lastUpdated")}</th>
          <th>{t("registry.capabilities")}</th>
          <th>{t("registry.context")}</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((item) => (
          <tr key={`${item.source_type}:${item.name}`}>
            <td className="mono">{item.name}</td>
            <td>
              <span
                className="audit-action-badge"
                style={{
                  padding: "2px 8px",
                  borderRadius: "4px",
                  fontSize: "var(--text-xs)",
                  background:
                    item.source_type === "discovered"
                      ? "var(--color-success-bg, rgba(34,197,94,0.15))"
                      : "var(--color-bg-secondary)",
                }}
              >
                {item.source_type === "discovered"
                  ? t("registry.discovered")
                  : t("registry.builtin")}
              </span>
            </td>
            <td className="mono" style={{ maxWidth: "220px", overflow: "hidden", textOverflow: "ellipsis" }}>
              {item.channel_name ?? "—"}
            </td>
            <td className="mono" style={{ whiteSpace: "nowrap" }}>
              {formatRelative(item.last_refreshed_secs, t)}
            </td>
            <td>
              <div className="registry-caps">
                {item.supports_vision && (
                  <span className="registry-cap" title={t("registry.vision")}>V</span>
                )}
                {item.supports_tools && (
                  <span className="registry-cap" title={t("registry.toolUse")}>T</span>
                )}
                {item.supports_thinking && (
                  <span className="registry-cap registry-cap-thinking" title={t("registry.thinking")}>Th</span>
                )}
                {item.thinking_format !== "None" && (
                  <span className="registry-cap-format" title={t("registry.thinkingFormat")}>
                    {item.thinking_format}
                  </span>
                )}
              </div>
            </td>
            <td className="mono">{formatContextSize(item.max_context_tokens)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );

  return (
    <section>
      <SectionHeader
        title={t("registry.title")}
        icon={Database}
        onRefresh={refresh}
        refreshing={loading}
        action={
          <span className="registry-count-badge">
            {t("registry.totalModels", { count: items.length })}
            {totalDiscovered > 0 && (
              <span className="registry-count-divider">·</span>
            )}
            {totalDiscovered > 0 && (
              <span className="registry-discovered-count">
                {t("registry.discoveredCount", { count: totalDiscovered })}
              </span>
            )}
          </span>
        }
      />

      <div className="settings-hint">{t("registry.hint")}</div>

      {filterSelect}

      {filtered.length === 0 ? (
        <div className="empty-state">
          <div className="empty-state-icon">🗂</div>
          <div className="empty-state-title">
            {items.length === 0 ? t("registry.noModels") : t("registry.noResults")}
          </div>
          <div className="empty-state-description">
            {items.length === 0 ? t("registry.noModelsHint") : t("registry.noResultsHint")}
          </div>
        </div>
      ) : groupByChannel && grouped ? (
        <div className="settings-table-wrapper">
          {grouped.map(([channelName, rows]) => (
            <div key={channelName} className="registry-group">
              <div className="registry-group-header">
                <span className="registry-group-title">{channelName}</span>
                <span className="registry-group-count">
                  {t("registry.modelsCount", { count: rows.length })}
                </span>
              </div>
              {renderTable(rows)}
            </div>
          ))}
        </div>
      ) : (
        <div className="settings-table-wrapper">{renderTable(filtered)}</div>
      )}
    </section>
  );
}
