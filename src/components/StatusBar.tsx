import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { api, type DispatchStats, type GwStatus, invokeTauri } from "../lib/api";
import { isTauri, API_BASE } from "../lib/runtime";
import { useQuota } from "../hooks/useQuota";

interface StatusResult {
  gw: GwStatus;
  stats: DispatchStats | null;
  networkError: boolean;
}

export function StatusBar() {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const queryClient = useQueryClient();
  const { totalBalance, channelsWithData, lowBalanceCount } = useQuota();

  const { data } = useQuery<StatusResult>({
    queryKey: ["status-bar"],
    queryFn: async () => {
      if (isTauri) {
        // Desktop mode: use Tauri IPC for gateway lifecycle
        try {
          const s = await invokeTauri<GwStatus>("gateway_status");
          let stats: DispatchStats | null = null;
          if (s.running) {
            try {
              stats = await api.stats();
            } catch {
              stats = null;
            }
          }
          return { gw: s, stats, networkError: false };
        } catch {
          return {
            gw: { running: false, host: "127.0.0.1", port: 8080 },
            stats: null,
            networkError: false,
          };
        }
      }
      // Web mode: gateway is already serving the page. Poll /health.
      try {
        const res = await fetch(`${API_BASE}/health`);
        let stats: DispatchStats | null = null;
        if (res.ok) {
          try {
            stats = await api.stats();
          } catch {
            stats = null;
          }
        }
        return {
          gw: {
            running: res.ok,
            host: window.location.hostname,
            port: Number(window.location.port) || 80,
          },
          stats,
          networkError: false,
        };
      } catch {
        return {
          gw: { running: false, host: "", port: 0 },
          stats: null,
          networkError: true,
        };
      }
    },
    // Poll every 3s. The QueryClient default staleTime (5s) does not affect
    // refetchInterval — the status bar always re-fetches on the interval.
    refetchInterval: 3_000,
    // Keep showing the last known status while refetching in the background
    // (the original code preserved `gw` between polls via functional setState).
    retry: false,
    refetchOnWindowFocus: true,
  });

  const gw = data?.gw ?? null;
  const stats = data?.stats ?? null;
  const networkError = data?.networkError ?? false;

  const handleAction = async (cmd: "gateway_start" | "gateway_stop" | "gateway_restart") => {
    setBusy(true);
    try {
      await invokeTauri(cmd);
    } catch (e) {
      console.error("Gateway action failed:", e);
    }
    // Immediate re-poll after action, then clear busy.
    setTimeout(async () => {
      await queryClient.invalidateQueries({ queryKey: ["status-bar"] });
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
