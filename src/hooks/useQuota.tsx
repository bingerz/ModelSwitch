import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import { api, type Channel, type QuotaInfo, type UsageHistory } from "../lib/api";
import {
  computeTotalBalance,
  countChannelsWithData,
  countLowBalance,
  countErrors,
} from "./quota-utils";

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
  /** Whether the first fetch is still in progress */
  loading: boolean;
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
  loading: true,
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
  const [loading, setLoading] = useState(true);

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
    } finally {
      setLoading(false);
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

  const totalBalance = useMemo(() => computeTotalBalance(quotas), [quotas]);
  const channelsWithData = useMemo(() => countChannelsWithData(quotas), [quotas]);
  const lowBalanceCount = useMemo(() => countLowBalance(quotas), [quotas]);
  const errorCount = useMemo(() => countErrors(quotas), [quotas]);

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
      loading,
      totalChannels: channels.length,
    }),
    [quotas, channels, usageHistory, refresh, fetchUsageHistory, totalBalance, channelsWithData, lowBalanceCount, errorCount, fetchError, loading],
  );

  return (
    <QuotaContext.Provider value={value}>
      {children}
    </QuotaContext.Provider>
  );
}
