import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
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
  const { t } = useTranslation();
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
    { id: "all", label: t("quota.all", { count: quotas.length }) },
    {
      id: "balance",
      label: t("quota.balanceFilter", { count: filterCounts.balance }),
    },
    {
      id: "rate_limit",
      label: t("quota.rateLimitFilter", { count: filterCounts.rateLimit }),
    },
    {
      id: "usage",
      label: t("quota.usageFilter", { count: filterCounts.usage }),
    },
    ...(errorCount > 0
      ? [{ id: "error" as FilterMode, label: t("quota.errorsFilter", { count: errorCount }) }]
      : []),
    ...(lowBalanceCount > 0
      ? [{ id: "low" as FilterMode, label: t("quota.lowFilter", { count: lowBalanceCount }) }]
      : []),
  ];

  const hasAnyContent = filtered.length > 0 || (filter === "all" && missingChannels.length > 0);

  return (
    <section>
      <SectionHeader
        title={t("quota.title")}
        icon={Wallet}
        onRefresh={handleRefresh}
        refreshing={refreshing}
      />

      {/* Summary cards */}
      <div className="quota-summary-grid">
        <StatTile
          icon={Wallet}
          value={formatBalance(totalBalance, "$0")}
          label={t("quota.totalBalance")}
          accent="green"
        />
        <StatTile
          icon={CheckCircle}
          value={`${channelsWithData}/${totalChannels}`}
          label={t("quota.channelsWithData")}
          accent="blue"
        />
        {errorCount > 0 && (
          <StatTile
            icon={AlertTriangle}
            value={errorCount}
            label={t("dashboard.quotaErrors")}
            accent="red"
          />
        )}
        {lowBalanceCount > 0 && (
          <StatTile
            icon={TrendingDown}
            value={lowBalanceCount}
            label={t("quota.lowBalance")}
            accent="amber"
          />
        )}
      </div>

      {/* Usage history chart */}
      {usageHistory && (
        <div className="usage-chart-section">
          <h3 className="usage-chart-title">{t("quota.usage")}</h3>
          <div className="usage-chart-summary">
            {usageHistory.total_cost > 0
              ? t("quota.tokensAcrossRequestsCost", {
                  tokens: formatTokens(usageHistory.total_input_tokens + usageHistory.total_output_tokens),
                  requests: usageHistory.total_requests,
                  cost: `$${usageHistory.total_cost.toFixed(4)}`,
                })
              : t("quota.tokensAcrossRequests", {
                  tokens: formatTokens(usageHistory.total_input_tokens + usageHistory.total_output_tokens),
                  requests: usageHistory.total_requests,
                })}
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
          title={t("quota.noData")}
          description={
            totalChannels === 0
              ? t("quota.noChannels")
              : filter === "error"
                ? t("quota.noErrors")
                : filter === "usage"
                  ? t("quota.noUsage")
                  : t("quota.noMatching")
          }
        />
      )}
    </section>
  );
}
