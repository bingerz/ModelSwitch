import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import { api, type Channel, type QuotaInfo, type UsageHistory } from "../lib/api";

interface QuotaContextValue {
  quotas: QuotaInfo[];
  channels: Channel[];
  usageHistory: UsageHistory | null;
  refresh: () => Promise<void>;
  fetchUsageHistory: (hours: number) => Promise<void>;
  /** Total balance across all channels (USD) */
  totalBalance: number;
  /** Number of channels with quota data */
  channelsWithData: number;
  /** Number of channels with low balance (< 20% of limit) */
  lowBalanceCount: number;
  /** Last fetch error, if any */
  fetchError: string | null;
  /** Number of channels that have quota errors */
  errorCount: number;
  /** Total number of configured channels */
  totalChannels: number;
}

const QuotaContext = createContext<QuotaContextValue>({
  quotas: [],
  channels: [],
  usageHistory: null,
  refresh: async () => {},
  fetchUsageHistory: async () => {},
  totalBalance: 0,
  channelsWithData: 0,
  fetchError: null,
  lowBalanceCount: 0,
  errorCount: 0,
  totalChannels: 0,
});

export function useQuota() {
  return useContext(QuotaContext);
}

export function QuotaProvider({ children }: { children: React.ReactNode }) {
  const [quotas, setQuotas] = useState<QuotaInfo[]>([]);
  const [channels, setChannels] = useState<Channel[]>([]);
  const [usageHistory, setUsageHistory] = useState<UsageHistory | null>(null);
  const [fetchError, setFetchError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [quotaData, channelData] = await Promise.all([
        api.quota(),
        api.listChannels(),
      ]);
      setQuotas(quotaData);
      setChannels(channelData);
      setFetchError(null);
    } catch (e) {
      setFetchError(e instanceof Error ? e.message : "Failed to fetch quota data");
    }
  }, []);

  const fetchUsageHistory = useCallback(async (hours: number) => {
    setUsageHistory(null); // Clear stale data to avoid wrong-window flash
    try {
      const historyData = await api.usageHistory(hours);
      setUsageHistory(historyData);
    } catch {
      // Usage history is non-critical, keep existing data
    }
  }, []);

  useEffect(() => {
    refresh();
    fetchUsageHistory(24);
    const interval = setInterval(refresh, 10000);
    const usageInterval = setInterval(() => fetchUsageHistory(24), 30000);
    return () => {
      clearInterval(interval);
      clearInterval(usageInterval);
    };
  }, [refresh, fetchUsageHistory]);

  const totalBalance = useMemo(
    () => quotas.reduce(
      (sum, q) => sum + (q.error === null ? (q.balance ?? 0) : 0),
      0,
    ),
    [quotas],
  );

  const channelsWithData = useMemo(
    () => quotas.filter(
      (q) => q.error === null && (
        q.balance != null
        || q.rate_limit_remaining_req != null
        || q.total_input_tokens != null
        || q.total_output_tokens != null
      ),
    ).length,
    [quotas],
  );

  const lowBalanceCount = useMemo(
    () => quotas.filter((q) => {
      if (q.balance == null || q.limit == null || q.limit <= 0) return false;
      return (q.balance / q.limit) < 0.2;
    }).length,
    [quotas],
  );

  const errorCount = useMemo(
    () => quotas.filter((q) => q.error !== null).length,
    [quotas],
  );

  const value = useMemo(
    () => ({
      quotas,
      channels,
      usageHistory,
      refresh,
      fetchUsageHistory,
      totalBalance,
      channelsWithData,
      lowBalanceCount,
      errorCount,
      fetchError,
      totalChannels: channels.length,
    }),
    [quotas, channels, usageHistory, refresh, fetchUsageHistory, totalBalance, channelsWithData, lowBalanceCount, errorCount, fetchError],
  );

  return (
    <QuotaContext.Provider value={value}>
      {children}
    </QuotaContext.Provider>
  );
}
