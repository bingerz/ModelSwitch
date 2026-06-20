import { useCallback, useEffect, useMemo, useState } from "react";
import { api, type Channel, type DispatchLog, type DispatchStats, type UsageHistory } from "../../lib/api";
import { useQuota } from "../../hooks/useQuota";
import "../../styles/status-dashboard.css";
import { ActivityChart } from "./ActivityChart";
import { ChannelHealth } from "./ChannelHealth";
import { LiveBadge } from "./LiveBadge";
import { QuotaSummary } from "./QuotaSummary";
import { StatCard } from "./StatCard";
import { TopEntities } from "./TopEntities";
import { Activity, CircleCheck, CircleOff, CircleX, DollarSign, RefreshCw, Timer, TriangleAlert, Zap } from "./icons";
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
      setLastUpdated(new Date());
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to fetch dashboard data");
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 5000);
    return () => clearInterval(interval);
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
          <span className="dsh-loading-text">Loading dashboard data...</span>
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
            size="hero"
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

        {/* Secondary stat row: 4 cards */}
        <div className="dsh-stat-cell">
          <StatCard
            icon={CircleCheck}
            value={stats ? formatNumber(stats.successes) : "0"}
            label="Successes"
            accent="green"
            sparkline={successSpark}
          />
        </div>
        <div className="dsh-stat-cell">
          <StatCard
            icon={CircleX}
            value={stats ? formatNumber(stats.failures) : "0"}
            label="Failures"
            accent="red"
            sparkline={failureSpark}
          />
        </div>
        <div className="dsh-stat-cell">
          <StatCard
            icon={Timer}
            value={stats ? `${Math.round(stats.avg_latency_ms)}ms` : "—"}
            label="Avg Latency"
            accent="amber"
            sparkline={latencySpark}
          />
        </div>
        <div className="dsh-stat-cell">
          <StatCard
            icon={DollarSign}
            value={`$${totalCost.toFixed(2)}`}
            label="Est. Cost (24h)"
            accent="blue"
            sparkline={costSpark}
            subtitle={`${formatNumber((usage?.total_input_tokens ?? 0) + (usage?.total_output_tokens ?? 0))} tokens`}
          />
        </div>

        {/* Channel overview stat row */}
        <div className="dsh-stat-cell">
          <StatCard
            icon={Activity}
            value={channels.length}
            label="Total Channels"
            accent="blue"
          />
        </div>
        <div className="dsh-stat-cell">
          <StatCard
            icon={CircleCheck}
            value={healthyCount}
            label="Healthy"
            accent="green"
          />
        </div>
        <div className="dsh-stat-cell">
          <StatCard
            icon={TriangleAlert}
            value={circuitOpenCount}
            label="Circuit Broken"
            accent="amber"
          />
        </div>
        <div className="dsh-stat-cell">
          <StatCard
            icon={CircleOff}
            value={disabledCount}
            label="Disabled"
            accent="gray"
          />
        </div>

        {/* Activity + Health row */}
        <div className="dsh-activity-cell">
          <ActivityChart logs={logs} />
        </div>
        <div className="dsh-health-cell">
          <ChannelHealth
            channels={channels}
            usageByChannel={usageByChannel}
            logs={logs}
          />
        </div>

        {/* Top entities row */}
        <div className="dsh-top-cell">
          <TopEntities channels={channelStats} models={modelStats} />
        </div>
      </div>
    </section>
  );
}
