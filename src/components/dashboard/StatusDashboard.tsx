import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, type Channel, type DispatchLog, type DispatchStats, type UsageHistory } from "../../lib/api";
import { useQuota } from "../../hooks/useQuota";
import "../../styles/status-dashboard.css";
import { ActivityChart } from "./ActivityChart";
import { ChannelHealth } from "./ChannelHealth";
import { LiveBadge } from "./LiveBadge";
import { PerformanceCard } from "./PerformanceCard";
import { QuotaSummary } from "./QuotaSummary";
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
  const [channels, setChannels] = useState<Channel[]>([]);
  const [stats, setStats] = useState<DispatchStats | null>(null);
  const [logs, setLogs] = useState<DispatchLog[]>([]);
  const [usage, setUsage] = useState<UsageHistory | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [lastUpdated, setLastUpdated] = useState<Date>(new Date());
  const [refreshing, setRefreshing] = useState(false);
  const loadingRef = useRef(true);
  const { totalBalance, channelsWithData, lowBalanceCount, errorCount, totalChannels } =
    useQuota();

  const fetchData = useCallback(async () => {
    try {
      const [ch, st, lg, us] = await Promise.all([
        api.listChannels(),
        api.stats(),
        api.logs(0, LOG_LIMIT),
        api.usageHistory(24),
      ]);
      setChannels(ch);
      setStats(st);
      setLogs([...lg].reverse());
      setUsage(us);
      setError(null);
      if (loadingRef.current) {
        loadingRef.current = false;
        setLoading(false);
      }
      setLastUpdated(new Date());
    } catch {
      // Only show error if we previously had data (connection lost).
      // During initial gateway startup, keep showing the loading spinner.
      if (!loadingRef.current) {
        setError("Connection lost — retrying...");
      }
    } finally {
      setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    let timeoutId: ReturnType<typeof setTimeout>;

    const poll = async () => {
      await fetchData();
      if (!cancelled) {
        // Retry faster while waiting for gateway, slower once connected
        const delay = loadingRef.current ? 1500 : 5000;
        timeoutId = setTimeout(poll, delay);
      }
    };

    poll();

    return () => {
      cancelled = true;
      clearTimeout(timeoutId);
    };
  }, [fetchData]);

  const handleRefresh = () => {
    setRefreshing(true);
    fetchData();
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
          <h2 className="panel-title">Status Dashboard</h2>
        </div>
        <div className="dsh-loading">
          <div className="dsh-loading-spinner" />
          <span className="dsh-loading-text">Connecting to gateway...</span>
        </div>
      </section>
    );
  }

  if (error) {
    return (
      <section>
        <div className="panel-header">
          <h2 className="panel-title">Status Dashboard</h2>
        </div>
        <div className="dsh-error">
          <span className="dsh-error-icon">
            <TriangleAlert size={24} />
          </span>
          <div className="dsh-error-text">{error}</div>
          <button className="dsh-error-btn" onClick={fetchData}>
            Retry
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
        <h2 className="dsh-title">Status Dashboard</h2>
        <div className="dsh-header-actions">
          <LiveBadge lastUpdated={lastUpdated} />
          <button
            className="dsh-refresh-btn"
            onClick={handleRefresh}
            disabled={refreshing}
            title="Refresh now"
          >
            <RefreshCw size={14} className={refreshing ? "dsh-spin" : ""} />
            <span>Refresh</span>
          </button>
        </div>
      </div>

      <div className="dashboard-grid">
        {/* Hero row: total requests (3 cols) + quota (1 col) */}
        <div className="dsh-hero-cell">
          <StatCard
            icon={Zap}
            value={stats ? formatNumber(stats.total_requests) : "0"}
            label="Total Requests"
            accent="blue"
            size="mega"
            sparkline={requestSpark}
            subtitle={`${healthyCount} healthy · ${circuitOpenCount + disabledCount} broken`}
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
    </section>
  );
}
