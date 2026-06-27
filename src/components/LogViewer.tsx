import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ClipboardList, Search } from "lucide-react";
import { api, type DispatchLog } from "../lib/api";
import { formatRelativeTime, latencyColor } from "../lib/format";
import "../styles/log-viewer.css";

type FilterValue = "all" | "success" | "failed";

function matchesSearch(log: DispatchLog, query: string): boolean {
  if (!query) return true;
  const q = query.toLowerCase();
  return (
    log.request_model.toLowerCase().includes(q) ||
    log.channel_name.toLowerCase().includes(q)
  );
}

export function LogViewer() {
  const { t } = useTranslation();
  const [filter, setFilter] = useState<FilterValue>("all");
  const [search, setSearch] = useState("");

  const { data: logs = [], isLoading: loading, refetch } = useQuery({
    queryKey: ["logs", 0, 100],
    queryFn: () => api.logs(0, 100),
    refetchInterval: 3_000,
    // Silently ignore fetch failures; UI will show stale data
    retry: false,
  });

  const fetchLogs = async () => {
    await refetch();
  };

  // Pre-compute counts for badge display (unfiltered)
  const totalCount = logs.length;
  const successCount = logs.filter((l) => l.success).length;
  const failedCount = logs.filter((l) => !l.success).length;

  // Apply filter then search
  const visibleLogs = useMemo(() => {
    const filtered =
      filter === "all"
        ? logs
        : logs.filter((l) => l.success === (filter === "success"));
    return filtered.filter((l) => matchesSearch(l, search));
  }, [logs, filter, search]);

  // Summary stats for visible logs
  const summary = useMemo(() => {
    const total = visibleLogs.length;
    const successes = visibleLogs.filter((l) => l.success).length;
    const successRate = total > 0 ? (successes / total) * 100 : 0;
    const avgLatency =
      total > 0
        ? Math.round(
            visibleLogs.reduce((sum, l) => sum + l.latency_ms, 0) / total,
          )
        : 0;
    return { total, successes, successRate, avgLatency };
  }, [visibleLogs]);

  // Determine which empty state to show
  const hasNoData = !loading && logs.length === 0;
  const hasNoResults = !loading && logs.length > 0 && visibleLogs.length === 0;

  if (loading) return <div className="panel-loading">{t("logs.loading")}</div>;

  return (
    <section>
      <div className="panel-header">
        <h2 className="panel-title">{t("logs.title")}</h2>
        <button className="btn btn-sm" onClick={fetchLogs}>
          {t("common.refresh")}
        </button>
      </div>

      <div className="log-filter-bar">
        <button
          className={`log-filter-btn${filter === "all" ? " active" : ""}`}
          onClick={() => setFilter("all")}
        >
          {t("common.all")}<span className="badge">({totalCount})</span>
        </button>
        <button
          className={`log-filter-btn${filter === "success" ? " active" : ""}`}
          onClick={() => setFilter("success")}
        >
          {t("common.success")}<span className="badge">({successCount})</span>
        </button>
        <button
          className={`log-filter-btn${filter === "failed" ? " active" : ""}`}
          onClick={() => setFilter("failed")}
        >
          {t("common.failed")}<span className="badge">({failedCount})</span>
        </button>

        <input
          className="log-search-input"
          type="text"
          placeholder={t("logs.searchPlaceholder")}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>

      {logs.length > 0 && (
        <div className="log-summary-stats">
          <span className="log-summary-stat">
            {t("logs.requestsLabel")}{" "}
            <span className="log-summary-stat-value">{summary.total}</span>
          </span>
          <div className="log-summary-stat-divider" />
          <span className="log-summary-stat">
            {t("logs.successRateLabel")}{" "}
            <span className="log-summary-stat-value success-color">
              {summary.successRate.toFixed(1)}%
            </span>
          </span>
          <div className="log-summary-stat-divider" />
          <span className="log-summary-stat">
            {t("logs.avgLatencyLabel")}{" "}
            <span className="log-summary-stat-value">
              {summary.avgLatency}ms
            </span>
          </span>
        </div>
      )}

      {hasNoData && (
        <div className="empty-state">
          <div className="log-empty-icon">
            <ClipboardList size={32} />
          </div>
          <div className="empty-state-title">{t("logs.empty")}</div>
          <div className="empty-state-description">
            {t("logs.emptyHint")}
          </div>
        </div>
      )}

      {hasNoResults && (
        <div className="empty-state">
          <div className="log-empty-icon">
            <Search size={32} />
          </div>
          <div className="empty-state-title">{t("logs.noResults")}</div>
          <div className="empty-state-description">
            {t("logs.noResultsHint")}
          </div>
        </div>
      )}

      {visibleLogs.length > 0 && (
        <div className="log-table-wrap">
          <table className="log-table">
            <thead>
              <tr>
                <th>{t("logs.timestamp")}</th>
                <th>{t("logs.model")}</th>
                <th>{t("logs.channel")}</th>
                <th>{t("logs.retries")}</th>
                <th>{t("logs.reason")}</th>
                <th>{t("logs.latency")}</th>
                <th>{t("logs.tokens")}</th>
                <th>{t("common.status")}</th>
              </tr>
            </thead>
            <tbody>
              {visibleLogs.map((log) => {
                const rel = formatRelativeTime(log.timestamp);
                return (
                  <tr
                    key={log.id}
                    className={
                      log.success ? "log-row-success" : "log-row-failure"
                    }
                  >
                    <td className="mono">
                      {new Date(log.timestamp).toLocaleTimeString()}
                      {rel && (
                        <span className="log-time-relative">{rel}</span>
                      )}
                    </td>
                    <td>{log.request_model}</td>
                    <td>{log.channel_name}</td>
                    <td>{log.retry_count}</td>
                    <td>{log.trigger_reason || "-"}</td>
                    <td className="mono" style={{ color: latencyColor(log.latency_ms) }}>
                      {log.latency_ms}ms
                    </td>
                    <td className="mono">
                      {log.input_tokens != null || log.output_tokens != null
                        ? `${log.input_tokens ?? 0}/${log.output_tokens ?? 0}`
                        : "-"}
                    </td>
                    <td>
                      <span
                        className={`log-status ${log.success ? "success" : "failure"}`}
                      >
                        {log.success ? t("common.ok") : t("common.fail")}
                      </span>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
