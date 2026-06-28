// Channel endpoints: CRUD, status, cooldown, batch ops, payload rules, auto-test.
// These methods are spread onto the top-level `api` object for backward compat.

import { request } from "./client";
import type {
  Channel,
  ChannelCooldownInfo,
  ChannelDiagnosticsResult,
  ChannelStatusInfo,
  ChannelTestResult,
  PingResult,
  PayloadRulesConfig,
  SingleChannelTestResult,
  UpdateChannelData,
} from "./types";

export const channelsApi = {
  listChannels: () => request<Channel[]>("/api/channels"),

  createChannel: (
    data: Partial<Channel> & { credential_value: string; credential_type?: string },
  ) =>
    request<Channel>("/api/channels", {
      method: "POST",
      body: JSON.stringify(data),
    }),

  updateChannel: (id: string, data: UpdateChannelData) =>
    request<Channel>(`/api/channels/${id}`, {
      method: "PUT",
      body: JSON.stringify(data),
    }),

  deleteChannel: (id: string) =>
    request<void>(`/api/channels/${id}`, { method: "DELETE" }),

  pingChannel: (id: string) =>
    request<PingResult>(`/api/channels/${id}/ping`, { method: "POST" }),

  channelStatus: (id: string) =>
    request<ChannelStatusInfo>(`/api/channels/${id}/status`),

  channelCooldown: (id: string) =>
    request<ChannelCooldownInfo>(`/api/channels/${id}/cooldown`),

  resetCircuit: (id: string) =>
    request<{ ok: boolean }>(`/api/channels/${id}/reset-circuit`, {
      method: "POST",
    }),

  testChannel: (id: string) =>
    request<SingleChannelTestResult>(`/api/channels/${id}/test`, {
      method: "POST",
    }),

  testAllChannels: () =>
    request<ChannelTestResult[]>("/api/channels/test-all", { method: "POST" }),

  channelDiagnostics: (id: string) =>
    request<ChannelDiagnosticsResult>(`/api/channels/${id}/diagnostics`, {
      method: "POST",
    }),

  // Batch operations
  batchEnableChannels: (ids: string[]) =>
    request<{ updated: number }>("/api/channels/batch/enable", {
      method: "POST",
      body: JSON.stringify({ ids }),
    }),

  batchDisableChannels: (ids: string[]) =>
    request<{ updated: number }>("/api/channels/batch/disable", {
      method: "POST",
      body: JSON.stringify({ ids }),
    }),

  batchDeleteChannels: (ids: string[]) =>
    request<{ deleted: number }>("/api/channels/batch/delete", {
      method: "POST",
      body: JSON.stringify({ ids }),
    }),

  batchUpdateTags: (ids: string[], tags: string[]) =>
    request<{ updated: number }>("/api/channels/batch/tags", {
      method: "PUT",
      body: JSON.stringify({ ids, add_tags: tags }),
    }),

  // Per-channel payload rules (runtime override)
  getPayloadRules: (channelId: string) =>
    request<PayloadRulesConfig>(`/api/channels/${channelId}/payload-rules`),

  updatePayloadRules: (channelId: string, rules: PayloadRulesConfig) =>
    request<{ channel_id: string; updated: boolean }>(
      `/api/channels/${channelId}/payload-rules`,
      {
        method: "PUT",
        body: JSON.stringify(rules),
      },
    ),
};
