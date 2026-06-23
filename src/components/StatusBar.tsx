import { useEffect, useState, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import { api, type DispatchStats, type GwStatus, invokeTauri } from "../lib/api";
import { isTauri, API_BASE } from "../lib/runtime";
import { useQuota } from "../hooks/useQuota";

export function StatusBar() {
  const { t } = useTranslation();
  const [gw, setGw] = useState<GwStatus | null>(null);
  const [stats, setStats] = useState<DispatchStats | null>(null);
  const [busy, setBusy] = useState(false);
  const [networkError, setNetworkError] = useState(false);
  const { totalBalance, channelsWithData, lowBalanceCount } = useQuota();

  // Backoff tracking for adaptive polling in web mode
  const backoffRef = useRef(3000); // Start at 3s

  const pollStatus = useCallback(async () => {
    if (isTauri) {
      // Desktop mode: use Tauri IPC for gateway lifecycle
      backoffRef.current = 3000;
      try {
        const s = await invokeTauri<GwStatus>("gateway_status");
        setGw(s);
        if (s.running) {
          try {
            setStats(await api.stats());
          } catch {
            setStats(null);
          }
        } else {
          setStats(null);
        }
      } catch {
        setGw((prev) => prev ?? { running: false, host: "127.0.0.1", port: 8080 });
      }
    } else {
      // Web mode: gateway is already serving the page. Poll /health.
      try {
        const res = await fetch(`${API_BASE}/health`);
        setNetworkError(false);
        backoffRef.current = 3000;
        setGw({
          running: res.ok,
          host: window.location.hostname,
          port: Number(window.location.port) || 80,
        });
        if (res.ok) {
          try {
            setStats(await api.stats());
          } catch {
            setStats(null);
          }
        }
      } catch {
        setNetworkError(true);
        backoffRef.current = Math.min(backoffRef.current * 1.5, 30000);
        setGw((prev) => prev ?? { running: false, host: "", port: 0 });
      }
    }
  }, []);

  useEffect(() => {
    pollStatus();
    // Use a recursive timeout instead of fixed interval for adaptive backoff
    let timeoutId: ReturnType<typeof setTimeout>;
    const scheduleNext = () => {
      timeoutId = setTimeout(async () => {
        await pollStatus();
        scheduleNext();
      }, backoffRef.current);
    };
    scheduleNext();
    return () => clearTimeout(timeoutId);
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
        <span>{isTauri ? `${t("status.gateway")}: ${addr}` : t("status.webConsole")}</span>
      </div>
      {networkError && !isTauri && (
        <div className="status-bar-error">
          <span className="status-bar-error-dot" />
          <span>{t("status.connectionLost")}</span>
          <span className="status-bar-reconnecting">{t("status.reconnecting")}</span>
        </div>
      )}
      {isTauri && (
        <div className="status-bar-center">
          <button
            className="gw-btn"
            onClick={() => handleAction("gateway_start")}
            disabled={running || busy}
            title={t("status.startGateway")}
          >
            {t("common.start")}
          </button>
          <button
            className="gw-btn"
            onClick={() => handleAction("gateway_stop")}
            disabled={!running || busy}
            title={t("status.stopGateway")}
          >
            {t("common.stop")}
          </button>
          <button
            className="gw-btn"
            onClick={() => handleAction("gateway_restart")}
            disabled={!running || busy}
            title={t("status.restartGateway")}
          >
            {t("common.restart")}
          </button>
        </div>
      )}
      <div className="status-bar-right">
        {channelsWithData > 0 && (
          <span className="status-bar-quota">
            <span className={lowBalanceCount > 0 ? "text-danger" : "text-success"}>
              ${Number.isFinite(totalBalance) ? totalBalance.toFixed(2) : "\u2014"}
            </span>
            <span className="status-bar-quota-label">{t("status.balance")}</span>
          </span>
        )}
        {stats && (
          <>
            <span>{t("status.req")}: {stats.total_requests}</span>
            <span className="text-success">{t("status.ok")}: {stats.successes}</span>
            <span className="text-danger">{t("status.fail")}: {stats.failures}</span>
            <span>{stats.avg_latency_ms}ms</span>
          </>
        )}
      </div>
    </footer>
  );
}
