import { useCallback, useEffect, useState } from "react";
import { api, type DispatchLog } from "../lib/api";

export function LogViewer() {
  const [logs, setLogs] = useState<DispatchLog[]>([]);
  const [loading, setLoading] = useState(true);

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

  if (loading) return <div className="panel-loading">Loading logs...</div>;

  return (
    <section>
      <div className="panel-header">
        <h2 className="panel-title">Dispatch Logs</h2>
        <button className="btn btn-sm" onClick={fetchLogs}>
          Refresh
        </button>
      </div>

      {logs.length === 0 ? (
        <p className="empty-state">No dispatch logs yet. Requests will appear here.</p>
      ) : (
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
              {logs.map((log) => (
                <tr key={log.id}>
                  <td className="mono">
                    {new Date(log.timestamp).toLocaleTimeString()}
                  </td>
                  <td>{log.request_model}</td>
                  <td>{log.channel_name}</td>
                  <td>{log.retry_count}</td>
                  <td>{log.trigger_reason || "-"}</td>
                  <td className="mono">{log.latency_ms}ms</td>
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
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
