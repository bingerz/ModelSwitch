import { useCallback, useEffect, useMemo, useState } from "react";
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
  const [logs, setLogs] = useState<DispatchLog[]>([]);
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState<FilterValue>("all");
  const [search, setSearch] = useState("");

  const fetchLogs = useCallback(async () => {
    try {
      const data = await api.logs(0, 100);
      setLogs(data);
    } catch {
      // Silently ignore fetch failures; UI will show stale data
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchLogs();
    const interval = setInterval(fetchLogs, 3000);
    return () => clearInterval(interval);
  }, [fetchLogs]);

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
    return { total, successRate, avgLatency };
  }, [visibleLogs]);

  // Determine which empty state to show
  const hasNoData = !loading && logs.length === 0;
  const hasNoResults = !loading && logs.length > 0 && visibleLogs.length === 0;

  if (loading) return <div className="panel-loading">Loading logs...</div>;

  return (
    <section>
      <div className="panel-header">
        <h2 className="panel-title">Dispatch Logs</h2>
        <button className="btn btn-sm" onClick={fetchLogs}>
          Refresh
        </button>
      </div>

      <div className="log-filter-bar">
        <button
          className={`log-filter-btn${filter === "all" ? " active" : ""}`}
          onClick={() => setFilter("all")}
        >
          All<span className="badge">({totalCount})</span>
        </button>
        <button
          className={`log-filter-btn${filter === "success" ? " active" : ""}`}
          onClick={() => setFilter("success")}
        >
          Success<span className="badge">({successCount})</span>
        </button>
        <button
          className={`log-filter-btn${filter === "failed" ? " active" : ""}`}
          onClick={() => setFilter("failed")}
        >
          Failed<span className="badge">({failedCount})</span>
        </button>

        <input
          className="log-search-input"
          type="text"
          placeholder="Search model or channel..."
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>

      {logs.length > 0 && (
        <div className="log-summary-stats">
          <span className="log-summary-stat">
            Requests:{" "}
            <span className="log-summary-stat-value">{summary.total}</span>
          </span>
          <div className="log-summary-stat-divider" />
          <span className="log-summary-stat">
            Success rate:{" "}
            <span className="log-summary-stat-value success-color">
              {summary.successRate.toFixed(1)}%
            </span>
          </span>
          <div className="log-summary-stat-divider" />
          <span className="log-summary-stat">
            Avg latency:{" "}
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
          <div className="empty-state-title">No dispatch logs yet</div>
          <div className="empty-state-description">
            Dispatch logs will appear here once the gateway starts routing
            requests to your channels.
          </div>
        </div>
      )}

      {hasNoResults && (
        <div className="empty-state">
          <div className="log-empty-icon">
            <Search size={32} />
          </div>
          <div className="empty-state-title">No matching logs</div>
          <div className="empty-state-description">
            Try adjusting your filters or search query.
          </div>
        </div>
      )}

      {visibleLogs.length > 0 && (
        <div className="log-table-wrap">
          <table className="log-table">
            <thead>
              <tr>
                <th>Time</th>
                <th>Model</th>
                <th>Channel</th>
                <th>Retries</th>
                <th>Reason</th>
                <th>Latency</th>
                <th>Tokens</th>
                <th>Status</th>
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
                        {log.success ? "OK" : "FAIL"}
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
