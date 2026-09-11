import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Radio } from "lucide-react";
import { api, type Channel, type DispatchLog, type DispatchStats, type UsageHistory } from "../../lib/api";
import { useQuota } from "../../hooks/useQuota";
import "../../styles/status-dashboard.css";
import { EmptyState } from "../ui/EmptyState";
import { LoadingState } from "../ui/LoadingState";
import { ErrorState } from "../ui/ErrorState";
import { ActivityChart } from "./ActivityChart";
import { ChannelHealth } from "./ChannelHealth";
import { LiveBadge } from "./LiveBadge";
import { QuotaSummary } from "./QuotaSummary";
import { SmartInsights } from "./SmartInsights";
import { TopEntities } from "./TopEntities";
import { RefreshCw } from "./icons";
import {
  aggregateByChannel,
  aggregateByModel,
  buildUsageByChannelMap,
  formatNumber,
} from "./helpers";

const LOG_LIMIT = 50;

interface StatusDashboardProps {
  onNavigate?: (tab: string) => void;
}

export function StatusDashboard({ onNavigate }: StatusDashboardProps) {
  const { t } = useTranslation();
  const { totalBalance, channelsWithData, lowBalanceCount, errorCount, totalChannels } =
    useQuota();

  // Adaptive polling: retry every 1.5s while waiting for the gateway,
  // then poll every 5s once data arrives.
  const adaptiveInterval = (query: { state: { data: unknown } }) =>
    query.state.data === undefined ? 1500 : 5000;

  const channelsQuery = useQuery({
    queryKey: ["channels"],
    queryFn: () => api.listChannels(),
    refetchInterval: adaptiveInterval,
    retry: false,
  });
  const statsQuery = useQuery({
    queryKey: ["stats"],
    queryFn: () => api.stats(),
    refetchInterval: adaptiveInterval,
    retry: false,
  });
  const logsQuery = useQuery<DispatchLog[]>({
    queryKey: ["logs", LOG_LIMIT],
    queryFn: async () => {
      const lg = await api.logs(0, LOG_LIMIT);
      return [...lg].reverse();
    },
    refetchInterval: adaptiveInterval,
    retry: false,
  });
  const usageQuery = useQuery({
    queryKey: ["usageHistory", 24],
    queryFn: () => api.usageHistory(24),
    refetchInterval: adaptiveInterval,
    retry: false,
  });

  const channels: Channel[] = channelsQuery.data ?? [];
  const stats: DispatchStats | null = statsQuery.data ?? null;
  const logs: DispatchLog[] = logsQuery.data ?? [];
  const usage: UsageHistory | null = usageQuery.data ?? null;

  // Initial load only — background refetches must not replace the panel with a spinner.
  const loading =
    channelsQuery.isPending || statsQuery.isPending || logsQuery.isPending;

  const error =
    !loading &&
    (channelsQuery.isError || statsQuery.isError || logsQuery.isError)
      ? t("dashboard.connectionLost")
      : null;

  const [refreshing, setRefreshing] = useState(false);

  const lastUpdated = useMemo(() => {
    const timestamps = [
      channelsQuery.dataUpdatedAt,
      statsQuery.dataUpdatedAt,
      logsQuery.dataUpdatedAt,
      usageQuery.dataUpdatedAt,
    ].filter((ts) => ts > 0);
    return timestamps.length > 0
      ? new Date(Math.max(...timestamps))
      : new Date();
  }, [
    channelsQuery.dataUpdatedAt,
    statsQuery.dataUpdatedAt,
    logsQuery.dataUpdatedAt,
    usageQuery.dataUpdatedAt,
  ]);

  const handleRefresh = async () => {
    setRefreshing(true);
    await Promise.all([
      channelsQuery.refetch(),
      statsQuery.refetch(),
      logsQuery.refetch(),
      usageQuery.refetch(),
    ]);
    setRefreshing(false);
  };

  const handleRetry = () => {
    Promise.all([
      channelsQuery.refetch(),
      statsQuery.refetch(),
      logsQuery.refetch(),
      usageQuery.refetch(),
    ]);
  };

  const channelStats = useMemo(
    () => (usage ? aggregateByChannel(usage.buckets) : []),
    [usage]
  );
  const modelStats = useMemo(
    () => (usage ? aggregateByModel(usage.buckets) : []),
    [usage]
  );
  const usageByChannel = useMemo(
    () => buildUsageByChannelMap(usage?.buckets ?? []),
    [usage]
  );

  if (loading) {
    return (
      <section>
        <div className="panel-header">
          <h2 className="panel-title">{t("dashboard.title")}</h2>
        </div>
        <LoadingState message={t("dashboard.connecting")} />
      </section>
    );
  }

  if (error) {
    return (
      <section>
        <div className="panel-header">
          <h2 className="panel-title">{t("dashboard.title")}</h2>
        </div>
        <ErrorState
          title={error}
          action={
            <button className="btn btn-primary" onClick={handleRetry}>
              {t("common.retry")}
            </button>
          }
        />
      </section>
    );
  }

  if (channels.length === 0) {
    return (
      <section>
        <div className="panel-header">
          <h2 className="panel-title">{t("dashboard.title")}</h2>
        </div>
        <EmptyState
          icon={Radio}
          title={t("dashboard.emptyTitle")}
          description={t("dashboard.emptyHint")}
          action={
            <button
              type="button"
              className="btn btn-primary"
              onClick={() => {
                if (onNavigate) {
                  onNavigate("channels");
                } else {
                  window.dispatchEvent(
                    new CustomEvent("navigate-to-tab", { detail: "channels" })
                  );
                }
              }}
            >
              {t("dashboard.addFirstChannel")}
            </button>
          }
        />
      </section>
    );
  }

const healthyCount = channels.filter((c) => c.status === "healthy").length;
  const circuitOpenCount = channels.filter((c) => c.status === "circuit_open").length;
  const disabledCount = channels.filter((c) => c.status === "disabled").length;
  const totalCost = usage?.total_cost ?? 0;
  const totalRequests = stats?.total_requests ?? 0;
  const successes = stats?.successes ?? 0;
  const failures = stats?.failures ?? 0;
  const successRate =
    totalRequests > 0 ? Math.round((successes / totalRequests) * 1000) / 10 : null;
  const brokenCount = circuitOpenCount + disabledCount;

  return (
    <section className="dsh-root">
      <div className="dsh-header">
        <div className="dsh-header-copy">
          <h2 className="dsh-title">{t("dashboard.title")}</h2>
          <p className="dsh-subtitle">
            {t("common.healthyCount", { count: healthyCount })}
            {" · "}
            {t("common.brokenCount", { count: brokenCount })}
          </p>
        </div>
        <div className="dsh-header-actions">
          <LiveBadge lastUpdated={lastUpdated} />
          <button
            className="dsh-refresh-btn"
            onClick={handleRefresh}
            disabled={refreshing}
            aria-busy={refreshing}
            title={t("common.refreshNow")}
          >
            <RefreshCw size={14} className={refreshing ? "dsh-spin" : ""} />
            <span>{refreshing ? t("common.refreshing") : t("common.refresh")}</span>
          </button>
        </div>
      </div>

      <div className="dsh-metrics-strip" role="list">
        <div className="dsh-metric" role="listitem">
          <div className="dsh-metric-label">{t("dashboard.activeChannels")}</div>
          <div className="dsh-metric-value">
            {healthyCount}
            <span className="dsh-metric-suffix">/{channels.length}</span>
          </div>
          <div className="dsh-metric-hint">
            {brokenCount > 0
              ? t("common.brokenCount", { count: brokenCount })
              : t("dashboard.quotaHealthy")}
          </div>
        </div>
        <div className="dsh-metric" role="listitem">
          <div className="dsh-metric-label">{t("dashboard.successRateLabel")}</div>
          <div className="dsh-metric-value">
            {successRate != null ? `${successRate}%` : "—"}
          </div>
          <div className="dsh-metric-hint">
            {t("dashboard.successRate", { count: successes, total: totalRequests })}
          </div>
        </div>
        <div className="dsh-metric" role="listitem">
          <div className="dsh-metric-label">{t("dashboard.avgLatency")}</div>
          <div className="dsh-metric-value">
            {stats?.avg_latency_ms != null
              ? `${Math.round(stats.avg_latency_ms)}`
              : "—"}
            {stats?.avg_latency_ms != null && (
              <span className="dsh-metric-suffix">ms</span>
            )}
          </div>
          <div className="dsh-metric-hint">
            {t("dashboard.failures")}: {formatNumber(failures)}
          </div>
        </div>
        <div className="dsh-metric" role="listitem">
          <div className="dsh-metric-label">{t("dashboard.totalRequests")}</div>
          <div className="dsh-metric-value">{formatNumber(totalRequests)}</div>
          <div className="dsh-metric-hint">
            {t("dashboard.estCost24h")}: ${totalCost.toFixed(2)}
          </div>
        </div>
      </div>

      {(channelsWithData > 0 || lowBalanceCount > 0 || errorCount > 0) && (
        <div className="dsh-quota-strip">
          <QuotaSummary
            totalBalance={totalBalance}
            channelsWithData={channelsWithData}
            totalChannels={totalChannels}
            lowBalance={lowBalanceCount}
            errors={errorCount}
          />
        </div>
      )}

      <div className="dashboard-grid">
        <div className="dsh-activity-cell">
          <ActivityChart logs={logs} />
        </div>

        <div className="dsh-health-cell">
          <ChannelHealth
            channels={channels}
            usageByChannel={usageByChannel}
            logs={logs}
            healthyCount={healthyCount}
            circuitOpenCount={circuitOpenCount}
            disabledCount={disabledCount}
          />
        </div>
        <div className="dsh-top-cell">
          <TopEntities channels={channelStats} models={modelStats} />
        </div>
      </div>

      <SmartInsights channels={channels} stats={stats} logs={logs} />
    </section>
  );
}
