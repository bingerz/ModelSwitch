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
import { modelsApi } from "./models";

import { isMockMode } from "../mock-flag";

export { fetchProviderModels, type FetchedModel, type FetchModelsParams } from "./models";

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
  channelDiagnostics: typeof channelsApi.channelDiagnostics;
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
  updateCacheMode: typeof configApi.updateCacheMode;
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
  updateRoutingStrategy: typeof configApi.updateRoutingStrategy;
  modelRouting: typeof configApi.modelRouting;
  sanitizerConfig: typeof configApi.sanitizerConfig;
  updateSanitizer: typeof configApi.updateSanitizer;
  authStatus: typeof configApi.authStatus;

  // MCP (flat)
  mcpHealth: typeof mcpHealth;

  // Nested namespaces
  mcp: typeof mcpApi;
  models: typeof modelsApi;
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
  models: modelsApi,
  virtualKeys: virtualKeysApi,
  redemptionCodes: redemptionCodesApi,
  reports: reportsApi,
};

// ─── Mock mode ──────────────────────────────────────────
// When mock mode is enabled, replace api methods with mock implementations.
// The mock module is loaded dynamically so the 1.6k lines of mock data + mock
// API stay in a separate chunk and out of the production bundle. The dynamic
// import is kicked off at module init; in dev the chunk resolves within a
// few milliseconds (fast loopback fetch), and by the time any React effect
// fires an API call the mock methods are installed on `api`.
if (isMockMode()) {
  void import("../mock").then(({ mockApi }) => {
    Object.assign(api, mockApi);
  });
}
