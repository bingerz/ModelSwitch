import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { api, type Channel, type DispatchLog, type DispatchStats, type UsageHistory } from "../../lib/api";
import { useQuota } from "../../hooks/useQuota";
import "../../styles/status-dashboard.css";
import { ActivityChart } from "./ActivityChart";
import { ChannelHealth } from "./ChannelHealth";
import { LiveBadge } from "./LiveBadge";
import { PerformanceCard } from "./PerformanceCard";
import { QuotaSummary } from "./QuotaSummary";
import { SmartInsights } from "./SmartInsights";
import { StatCard } from "./StatCard";
import { TopEntities } from "./TopEntities";
import { RefreshCw, TriangleAlert, Zap } from "./icons";
import {
  aggregateByChannel,
  aggregateByHour,
  aggregateByModel,
  buildFailureSpark,
  buildLatencySpark,
  buildSuccessSpark,
  buildUsageByChannelMap,
  formatNumber,
} from "./helpers";

const LOG_LIMIT = 50;

export function StatusDashboard() {
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

  // Show loading spinner until all critical queries have delivered data.
  const loading =
    channelsQuery.data === undefined ||
    statsQuery.data === undefined ||
    logsQuery.data === undefined;

  // Only show error if we previously had data (connection lost).
  // During initial gateway startup, keep showing the loading spinner.
  const error = loading
    ? null
    : channelsQuery.isError || statsQuery.isError || logsQuery.isError
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

  const hourPoints = useMemo(
    () => (usage ? aggregateByHour(usage.buckets) : []),
    [usage]
  );
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

  const requestSpark = useMemo(() => hourPoints.map((h) => h.requests), [hourPoints]);
  const costSpark = useMemo(() => hourPoints.map((h) => h.cost), [hourPoints]);
  const successSpark = useMemo(
    () => buildSuccessSpark(logs),
    [logs]
  );
  const failureSpark = useMemo(
    () => buildFailureSpark(logs),
    [logs]
  );
  const latencySpark = useMemo(
    () => buildLatencySpark(logs),
    [logs]
  );

  if (loading) {
    return (
      <section>
        <div className="panel-header">
          <h2 className="panel-title">{t("dashboard.title")}</h2>
        </div>
        <div className="dsh-loading">
          <div className="dsh-loading-spinner" />
          <span className="dsh-loading-text">{t("dashboard.connecting")}</span>
        </div>
      </section>
    );
  }

  if (error) {
    return (
      <section>
        <div className="panel-header">
          <h2 className="panel-title">{t("dashboard.title")}</h2>
        </div>
        <div className="dsh-error">
          <span className="dsh-error-icon">
            <TriangleAlert size={24} />
          </span>
          <div className="dsh-error-text">{error}</div>
          <button className="dsh-error-btn" onClick={handleRetry}>
            {t("common.retry")}
          </button>
        </div>
      </section>
    );
  }

  if (channels.length === 0) {
    return (
      <section>
        <div className="panel-header">
          <h2 className="panel-title">{t("dashboard.title")}</h2>
        </div>
        <div className="empty-state">
          <div className="empty-state-icon">🚀</div>
          <div className="empty-state-title">{t("dashboard.emptyTitle")}</div>
          <div className="empty-state-description">{t("dashboard.emptyHint")}</div>
          <button
            className="btn btn-primary"
            onClick={() => {
              const event = new CustomEvent("navigate-to-tab", { detail: "channels" });
              window.dispatchEvent(event);
            }}
            style={{ marginTop: "var(--space-3)" }}
          >
            {t("dashboard.addFirstChannel")}
          </button>
        </div>
      </section>
    );
  }

const healthyCount = channels.filter((c) => c.status === "healthy").length;
  const circuitOpenCount = channels.filter((c) => c.status === "circuit_open").length;
  const disabledCount = channels.filter((c) => c.status === "disabled").length;
  const totalCost = usage?.total_cost ?? 0;

  return (
    <section className="dsh-root">
      <div className="dsh-header">
        <h2 className="dsh-title">{t("dashboard.title")}</h2>
        <div className="dsh-header-actions">
          <LiveBadge lastUpdated={lastUpdated} />
          <button
            className="dsh-refresh-btn"
            onClick={handleRefresh}
            disabled={refreshing}
            title={t("common.refreshNow")}
          >
            <RefreshCw size={14} className={refreshing ? "dsh-spin" : ""} />
            <span>{t("common.refresh")}</span>
          </button>
        </div>
      </div>

      <div className="dashboard-grid">
        {/* Hero row: total requests (3 cols) + quota (1 col) */}
        <div className="dsh-hero-cell">
          <StatCard
            icon={Zap}
            value={stats ? formatNumber(stats.total_requests) : "0"}
            label={t("dashboard.totalRequests")}
            accent="blue"
            size="mega"
            sparkline={requestSpark}
            subtitle={`${t("common.healthyCount", { count: healthyCount })} \u00B7 ${t("common.brokenCount", { count: circuitOpenCount + disabledCount })}`}
          />
        </div>
        <div className="dsh-quota-cell">
          <QuotaSummary
            totalBalance={totalBalance}
            channelsWithData={channelsWithData}
            totalChannels={totalChannels}
            lowBalance={lowBalanceCount}
            errors={errorCount}
          />
        </div>

        {/* Performance consolidation: 4 metrics in one card */}
        <div className="dsh-performance-cell">
          <PerformanceCard
            successes={stats?.successes ?? 0}
            failures={stats?.failures ?? 0}
            avgLatencyMs={stats?.avg_latency_ms ?? null}
            totalCost={totalCost}
            totalTokens={(usage?.total_input_tokens ?? 0) + (usage?.total_output_tokens ?? 0)}
            successSpark={successSpark}
            failureSpark={failureSpark}
            latencySpark={latencySpark}
            costSpark={costSpark}
          />
        </div>

        {/* Health + Top entities row */}
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

        {/* Activity row (full width) */}
        <div className="dsh-activity-cell">
          <ActivityChart logs={logs} />
        </div>
      </div>

      <SmartInsights channels={channels} stats={stats} logs={logs} />
    </section>
  );
}
