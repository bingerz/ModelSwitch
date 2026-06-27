import { createContext, useCallback, useContext, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
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
  const [usageHours, setUsageHours] = useState(24);

  const quotasQuery = useQuery({
    queryKey: ["quota"],
    queryFn: () => api.quota(),
    refetchInterval: 10_000,
  });

  const channelsQuery = useQuery({
    queryKey: ["channels"],
    queryFn: () => api.listChannels(),
    refetchInterval: 10_000,
  });

  const usageQuery = useQuery({
    queryKey: ["usageHistory", usageHours],
    queryFn: () => api.usageHistory(usageHours),
    refetchInterval: 30_000,
  });

  const quotas = quotasQuery.data ?? [];
  const channels = channelsQuery.data ?? [];
  const usageHistory = usageQuery.data ?? null;

  const refresh = useCallback(async () => {
    await Promise.all([quotasQuery.refetch(), channelsQuery.refetch()]);
  }, [quotasQuery, channelsQuery]);

  // Changing `usageHours` updates the queryKey, which triggers a fresh fetch.
  // React Query clears data when the key changes (no keepPreviousData), so
  // stale data from the previous window does not flash.
  const fetchUsageHistory = useCallback(async (hours: number) => {
    setUsageHours(hours);
  }, []);

  const totalBalance = useMemo(() => computeTotalBalance(quotas), [quotas]);
  const channelsWithData = useMemo(() => countChannelsWithData(quotas), [quotas]);
  const lowBalanceCount = useMemo(() => countLowBalance(quotas), [quotas]);
  const errorCount = useMemo(() => countErrors(quotas), [quotas]);

  const fetchError =
    quotasQuery.error?.message ?? channelsQuery.error?.message ?? null;
  const loading = quotasQuery.isLoading || channelsQuery.isLoading;

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
