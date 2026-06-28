// Gateway config endpoints: cache management, gateway info, provider budgets,
// completion ratios, notification config, model registry, reload.
// All flat methods on the top-level `api` object.

import { request } from "./client";
import type {
  CacheStats,
  FlushCacheResult,
  GatewayInfo,
  ModelRegistryResponse,
  NotificationConfig,
  ProviderBudgetEntry,
  ReloadConfigResult,
} from "./types";

export const configApi = {
  cacheStats: () => request<CacheStats>("/api/cache/stats"),

  flushCache: () =>
    request<FlushCacheResult>("/api/cache/flush", { method: "POST" }),

  reloadConfig: () =>
    request<ReloadConfigResult>("/api/config/reload", { method: "POST" }),

  gatewayInfo: () => request<GatewayInfo>("/api/gateway/info"),

  // Provider budgets
  providerBudgets: () => request<ProviderBudgetEntry[]>("/api/provider-budgets"),

  setProviderBudget: (
    provider: string,
    budget: { daily_budget_cents?: number | null; monthly_budget_cents?: number | null },
  ) =>
    request<ProviderBudgetEntry>(
      `/api/provider-budgets/${encodeURIComponent(provider)}`,
      {
        method: "PUT",
        body: JSON.stringify(budget),
      },
    ),

  deleteProviderBudget: (provider: string) =>
    request<void>(`/api/provider-budgets/${encodeURIComponent(provider)}`, {
      method: "DELETE",
    }),

  // Notification config
  notificationConfig: () => request<NotificationConfig>("/api/notifications"),

  updateNotification: (config: Partial<NotificationConfig>) =>
    request<NotificationConfig>("/api/notifications", {
      method: "PUT",
      body: JSON.stringify(config),
    }),

  // Completion ratios
  completionRatios: () =>
    request<Record<string, number>>("/api/completion-ratios"),

  updateCompletionRatios: (ratios: Record<string, number>) =>
    request<Record<string, number>>("/api/completion-ratios", {
      method: "PUT",
      body: JSON.stringify(ratios),
    }),

  // Model registry inspection
  modelRegistry: () =>
    request<ModelRegistryResponse>("/api/model-registry"),

  // Routing strategy
  updateRoutingStrategy: (strategy: string) =>
    request<string>("/api/gateway/routing-strategy", {
      method: "PUT",
      body: JSON.stringify({ strategy }),
    }),
};
