// Public entrypoint for the API surface.
//
// Re-exports all types, the transport layer, and assembles the flat `api`
// object so existing `import { api, request, Channel } from "../lib/api"`
// imports keep working without changes.

export type * from "./types";

export {
  request,
  createRequestFn,
  createRawTextFn,
  fetchText,
  buildAuthHeaders,
  invokeTauri,
  isTauri,
  type RequestFnConfig,
} from "./client";

export { PRIORITY_TIERS, validateChannelForm } from "./presets";

import { channelsApi } from "./channels";
import { virtualKeysApi } from "./virtualkeys";
import { mcpApi, mcpHealth } from "./mcp";
import { quota, redemptionCodesApi } from "./quota";
import { reportsFlatApi, reportsApi, logs } from "./reports";
import { guardrailsApi } from "./guardrails";
import { configApi } from "./config";

import { isMockMode, mockApi } from "../mock";

export interface Api {
  // Channels (flat)
  listChannels: typeof channelsApi.listChannels;
  createChannel: typeof channelsApi.createChannel;
  updateChannel: typeof channelsApi.updateChannel;
  deleteChannel: typeof channelsApi.deleteChannel;
  pingChannel: typeof channelsApi.pingChannel;
  channelStatus: typeof channelsApi.channelStatus;
  channelCooldown: typeof channelsApi.channelCooldown;
  resetCircuit: typeof channelsApi.resetCircuit;
  testChannel: typeof channelsApi.testChannel;
  testAllChannels: typeof channelsApi.testAllChannels;
  batchEnableChannels: typeof channelsApi.batchEnableChannels;
  batchDisableChannels: typeof channelsApi.batchDisableChannels;
  batchDeleteChannels: typeof channelsApi.batchDeleteChannels;
  batchUpdateTags: typeof channelsApi.batchUpdateTags;
  getPayloadRules: typeof channelsApi.getPayloadRules;
  updatePayloadRules: typeof channelsApi.updatePayloadRules;

  // Reports / stats (flat)
  logs: typeof logs;
  stats: typeof reportsFlatApi.stats;
  costStats: typeof reportsFlatApi.costStats;
  usageHistory: typeof reportsFlatApi.usageHistory;
  auditLog: typeof reportsFlatApi.auditLog;
  metrics: typeof reportsFlatApi.metrics;

  // Quota (flat)
  quota: typeof quota;

  // Guardrails (flat)
  guardrailsConfig: typeof guardrailsApi.guardrailsConfig;
  updateGuardrails: typeof guardrailsApi.updateGuardrails;

  // Config / gateway (flat)
  cacheStats: typeof configApi.cacheStats;
  flushCache: typeof configApi.flushCache;
  reloadConfig: typeof configApi.reloadConfig;
  gatewayInfo: typeof configApi.gatewayInfo;
  providerBudgets: typeof configApi.providerBudgets;
  setProviderBudget: typeof configApi.setProviderBudget;
  deleteProviderBudget: typeof configApi.deleteProviderBudget;
  notificationConfig: typeof configApi.notificationConfig;
  updateNotification: typeof configApi.updateNotification;
  completionRatios: typeof configApi.completionRatios;
  updateCompletionRatios: typeof configApi.updateCompletionRatios;
  modelRegistry: typeof configApi.modelRegistry;

  // MCP (flat)
  mcpHealth: typeof mcpHealth;

  // Nested namespaces
  mcp: typeof mcpApi;
  virtualKeys: typeof virtualKeysApi;
  redemptionCodes: typeof redemptionCodesApi;
  reports: typeof reportsApi;
}

/** Backward-compatible flat `api` object. Method names/signatures unchanged. */
export const api: Api = {
  ...channelsApi,
  ...reportsFlatApi,
  logs,
  quota,
  ...guardrailsApi,
  ...configApi,
  mcpHealth,
  mcp: mcpApi,
  virtualKeys: virtualKeysApi,
  redemptionCodes: redemptionCodesApi,
  reports: reportsApi,
};

// ─── Mock mode ──────────────────────────────────────────
// When mock mode is enabled, replace api methods with mock implementations.
// This allows zero changes to consumer code — all existing imports of { api }
// automatically pick up mock data.
if (isMockMode()) {
  Object.assign(api, mockApi);
}
