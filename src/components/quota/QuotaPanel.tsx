import { useMemo, useState } from "react";
import { useQuota } from "../../hooks/useQuota";
import { formatBalance, formatTokens, type FilterMode, type TimeWindow } from "./types";
import { QuotaCard } from "./QuotaCard";
import { PendingCard } from "./PendingCard";
import { UsageChart } from "./UsageChart";
import { Wallet, CheckCircle, AlertTriangle, TrendingDown, BarChart3 } from "lucide-react";
import { StatTile } from "../ui/StatTile";
import { SectionHeader } from "../ui/SectionHeader";
import { EmptyState } from "../ui/EmptyState";

export function QuotaPanel() {
  const {
    quotas,
    channels,
    usageHistory,
    refresh,
    fetchUsageHistory,
    totalBalance,
    channelsWithData,
    lowBalanceCount,
    errorCount,
    totalChannels,
  } = useQuota();
  const [filter, setFilter] = useState<FilterMode>("all");
  const [refreshing, setRefreshing] = useState(false);
  const [usageWindow, setUsageWindow] = useState<TimeWindow>(24);

  const handleRefresh = async () => {
    setRefreshing(true);
    await refresh();
    setRefreshing(false);
  };

  // Filter quotas — no longer hiding errored entries
  const filtered = quotas.filter((q) => {
    switch (filter) {
      case "balance":
        return q.balance != null && q.error === null;
      case "rate_limit":
        return q.rate_limit_remaining_req != null && q.error === null;
      case "usage":
        return (q.total_input_tokens != null || q.total_output_tokens != null) && q.error === null;
      case "error":
        return q.error !== null;
      case "low":
        return q.balance != null && q.limit != null && q.limit > 0
          ? q.balance / q.limit < 0.2
          : false;
      default:
        return true;
    }
  });

  // Find channels that have no quota entry at all
  const quotaChannelIds = new Set(quotas.map((q) => q.channel_id));
  const missingChannels = channels.filter(
    (c) => !quotaChannelIds.has(c.id),
  );

  const filterCounts = useMemo(() => ({
    balance: quotas.filter((q) => q.balance != null && q.error === null).length,
    rateLimit: quotas.filter((q) => q.rate_limit_remaining_req != null && q.error === null).length,
    usage: quotas.filter((q) => (q.total_input_tokens != null || q.total_output_tokens != null) && q.error === null).length,
  }), [quotas]);

  const filterOptions: { id: FilterMode; label: string }[] = [
    { id: "all", label: `All (${quotas.length})` },
    {
      id: "balance",
      label: `Balance (${filterCounts.balance})`,
    },
    {
      id: "rate_limit",
      label: `Rate-Limit (${filterCounts.rateLimit})`,
    },
    {
      id: "usage",
      label: `Usage (${filterCounts.usage})`,
    },
    ...(errorCount > 0
      ? [{ id: "error" as FilterMode, label: `Errors (${errorCount})` }]
      : []),
    ...(lowBalanceCount > 0
      ? [{ id: "low" as FilterMode, label: `Low (${lowBalanceCount})` }]
      : []),
  ];

  const hasAnyContent = filtered.length > 0 || (filter === "all" && missingChannels.length > 0);

  return (
    <section>
      <SectionHeader
        title="Token Quota"
        icon={Wallet}
        onRefresh={handleRefresh}
        refreshing={refreshing}
      />

      {/* Summary cards */}
      <div className="quota-summary-grid">
        <StatTile
          icon={Wallet}
          value={formatBalance(totalBalance, "$0")}
          label="Total Balance"
          accent="green"
        />
        <StatTile
          icon={CheckCircle}
          value={`${channelsWithData}/${totalChannels}`}
          label="Channels with Data"
          accent="blue"
        />
        {errorCount > 0 && (
          <StatTile
            icon={AlertTriangle}
            value={errorCount}
            label="Errors"
            accent="red"
          />
        )}
        {lowBalanceCount > 0 && (
          <StatTile
            icon={TrendingDown}
            value={lowBalanceCount}
            label="Low Balance (<20%)"
            accent="amber"
          />
        )}
      </div>

      {/* Usage history chart */}
      {usageHistory && (
        <div className="usage-chart-section">
          <h3 className="usage-chart-title">Usage</h3>
          <div className="usage-chart-summary">
            <span className="mono">{formatTokens(usageHistory.total_input_tokens + usageHistory.total_output_tokens)}</span> tokens
            {" across "}
            <span className="mono">{usageHistory.total_requests}</span> requests
            {usageHistory.total_cost > 0 && (
              <>{" "}&middot; <span className="mono">${usageHistory.total_cost.toFixed(4)}</span> est. cost</>
            )}
          </div>
          <UsageChart buckets={usageHistory.buckets} window={usageWindow} onWindowChange={(w) => { setUsageWindow(w); fetchUsageHistory(w); }} />
        </div>
      )}

      {/* Filter bar */}
      <div className="quota-filter-bar">
        {filterOptions.map((opt) => (
          <button
            key={opt.id}
            className={`quota-filter-btn ${filter === opt.id ? "active" : ""}`}
            onClick={() => setFilter(opt.id)}
          >
            {opt.label}
          </button>
        ))}
      </div>

      {/* Quota cards grid */}
      {hasAnyContent ? (
        <div className="quota-grid">
          {filtered.map((q) => (
            <QuotaCard key={q.channel_id} q={q} onScrape={() => refresh()} />
          ))}
          {filter === "all" && missingChannels.map((c) => (
            <PendingCard key={c.id} name={c.name} provider={c.provider} />
          ))}
        </div>
      ) : (
        <EmptyState
          icon={BarChart3}
          title="No quota data"
          description={
            totalChannels === 0
              ? "No channels configured yet. Add channels in the Channels tab to start monitoring quota."
              : filter === "error"
                ? "No channels with errors."
                : filter === "usage"
                  ? "No token usage data yet. Usage will appear once channels start processing requests."
                  : "No quota data matching this filter. Quota information will appear here once channels start reporting balance, rate-limit, or usage data."
          }
        />
      )}
    </section>
  );
}
