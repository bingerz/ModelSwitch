import { useEffect, useState, useCallback } from "react";
import { api, type DispatchStats, type GwStatus, invokeTauri } from "../lib/api";
import { useQuota } from "../hooks/useQuota";

export function StatusBar() {
  const [gw, setGw] = useState<GwStatus | null>(null);
  const [stats, setStats] = useState<DispatchStats | null>(null);
  const [busy, setBusy] = useState(false);
  const { totalBalance, channelsWithData, lowBalanceCount } = useQuota();

  const pollStatus = useCallback(async () => {
    try {
      const s = await invokeTauri<GwStatus>("gateway_status");
      setGw(s);
      // If running, also fetch stats over HTTP
      if (s.running) {
        try {
          const d = await api.stats();
          setStats(d);
        } catch {
          setStats(null);
        }
      } else {
        setStats(null);
      }
    } catch {
      // Tauri API not available (browser dev mode)
      setGw((prev) => prev ?? { running: false, host: "127.0.0.1", port: 8080 });
    }
  }, []);

  useEffect(() => {
    pollStatus();
    const interval = setInterval(pollStatus, 3000);
    return () => clearInterval(interval);
  }, [pollStatus]);

  const handleAction = async (cmd: "gateway_start" | "gateway_stop" | "gateway_restart") => {
    setBusy(true);
    try {
      await invokeTauri(cmd);
    } catch (e) {
      console.error("Gateway action failed:", e);
    }
    // Immediate re-poll after action
    setTimeout(() => {
      pollStatus();
      setBusy(false);
    }, 500);
  };

  const running = gw?.running ?? false;
  const addr = gw ? `${gw.host}:${gw.port}` : "127.0.0.1:8080";

  return (
    <footer className="status-bar">
      <div className="status-bar-left">
        <span className={`status-bar-indicator ${running ? "" : "status-bar-indicator-off"}`} />
        <span>Gateway: {addr}</span>
      </div>
      <div className="status-bar-center">
        <button
          className="gw-btn"
          onClick={() => handleAction("gateway_start")}
          disabled={running || busy}
          title="Start gateway"
        >
          Start
        </button>
        <button
          className="gw-btn"
          onClick={() => handleAction("gateway_stop")}
          disabled={!running || busy}
          title="Stop gateway"
        >
          Stop
        </button>
        <button
          className="gw-btn"
          onClick={() => handleAction("gateway_restart")}
          disabled={!running || busy}
          title="Restart gateway"
        >
          Restart
        </button>
      </div>
      <div className="status-bar-right">
        {channelsWithData > 0 && (
          <span className="status-bar-quota">
            <span className={lowBalanceCount > 0 ? "text-danger" : "text-success"}>
              ${Number.isFinite(totalBalance) ? totalBalance.toFixed(2) : "—"}
            </span>
            <span className="status-bar-quota-label">balance</span>
          </span>
        )}
        {stats && (
          <>
            <span>Req: {stats.total_requests}</span>
            <span className="text-success">Ok: {stats.successes}</span>
            <span className="text-danger">Fail: {stats.failures}</span>
            <span>{stats.avg_latency_ms}ms</span>
          </>
        )}
      </div>
    </footer>
  );
}
