import { isTauri, API_BASE } from "./runtime";

export { isTauri };

export interface Channel {
  id: string;
  name: string;
  provider: string;
  priority: number;
  weight: number;
  cost_per_token: number | null;
  input_cost_per_mtok: number | null;
  output_cost_per_mtok: number | null;
  enabled: boolean;
  status: "healthy" | "circuit_open" | "disabled";
  circuit_open_until: string | null;
  base_url: string;
  model_mapping: Record<string, string>;
  created_at: string;
  updated_at: string;
  avg_latency_ms: number;
  consecutive_failures: number;
  cooldown_minutes: number | null;
  rpm_limit: number | null;
  tpm_limit: number | null;
  tags: string[];
  account_group: string | null;
  max_concurrent: number | null;
  excluded_models: string[];
  api_keys: string[];
  proxy_url: string | null;
  headers: Record<string, string>;
  max_retries: number | null;
  models_endpoint: string | null;
  models_refresh_interval_secs: number;
}

export interface DispatchLog {
  id: string;
  timestamp: string;
  request_model: string;
  channel_id: string;
  channel_name: string;
  channel_priority: number;
  retry_count: number;
  trigger_reason: string | null;
  latency_ms: number;
  success: boolean;
  estimated_cost: number | null;
  input_tokens: number | null;
  output_tokens: number | null;
  cache_hit_tokens: number | null;
  cache_miss_tokens: number | null;
}

export interface DispatchStats {
  total_requests: number;
  successes: number;
  failures: number;
  avg_latency_ms: number;
}

export interface PriorityStats {
  priority: number;
  requests: number;
  estimated_cost: number;
}

export interface CostStats {
  total_requests: number;
  total_estimated_cost: number;
  priority_breakdown: PriorityStats[];
  model_counts: Record<string, number>;
  total_input_tokens: number;
  total_output_tokens: number;
}

export interface UsageBucket {
  timestamp: string;
  channel_id: string;
  channel_name: string;
  model: string;
  input_tokens: number;
  output_tokens: number;
  cache_hit_tokens: number;
  cache_miss_tokens: number;
  request_count: number;
  estimated_cost: number;
}

export interface UsageHistory {
  buckets: UsageBucket[];
  total_input_tokens: number;
  total_output_tokens: number;
  total_requests: number;
  total_cost: number;
}

export interface QuotaItem {
  label: string;
  value: string;
}

export interface QuotaGroup {
  window: string;
  utilization_pct: number | null;
  resets_at: string | null;
}

export interface QuotaInfo {
  channel_id: string;
  channel_name: string;
  provider: string;
  balance: number | null;
  limit: number | null;
  usage: number | null;
  remaining_tokens: number | null;
  remaining_requests: number | null;
  plan_status: string | null;
  expires_at: string | null;
  items: QuotaItem[];
  groups: QuotaGroup[];
  compact_text: string | null;
  rate_limit_remaining_req: number | null;
  rate_limit_limit_req: number | null;
  rate_limit_remaining_tok: number | null;
  rate_limit_limit_tok: number | null;
  rate_limit_updated_at: string | null;
  total_input_tokens: number | null;
  total_output_tokens: number | null;
  total_cache_hit_tokens: number | null;
  total_cache_miss_tokens: number | null;
  total_requests_counted: number | null;
  total_estimated_cost: number | null;
  source: string;
  updated_at: string;
  error: string | null;
}

/** Backend PaginatedResponse<T> envelope: `{ data, total, offset, limit }` */
interface PaginatedEnvelope<T> {
  data: T;
  total: number;
  offset: number;
  limit: number;
}

export async function request<T>(path: string, options?: RequestInit): Promise<T> {
  const token = localStorage.getItem("admin_token");
  const res = await fetch(`${API_BASE}${path}`, {
    ...options,
    headers: {
      "Content-Type": "application/json",
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      ...options?.headers,
    },
  });
  if (res.status === 401) {
    // Token invalid or expired — clear and redirect to login
    localStorage.removeItem("admin_token");
    sessionStorage.setItem("auth_expired", "1");
    window.location.reload();
    throw new Error("Unauthorized");
  }
  if (!res.ok) throw new Error(`API error: ${res.status}`);
  const json = await res.json();
  // Auto-unwrap ApiResponse<T> envelope
  if (json && typeof json === "object" && "ok" in json && "data" in json) {
    if (!json.ok) {
      throw new Error(json.error?.message ?? "Unknown API error");
    }
    return json.data as T;
  }
  return json as T;
}

export interface UpdateChannelData {
  name: string;
  provider: string;
  priority: number;
  weight: number;
  cost_per_token: number | null;
  base_url: string;
  enabled: boolean;
  model_mapping: Record<string, string>;
  cooldown_minutes: number | null;
  credential_type?: string;
  credential_value?: string;
  input_cost_per_mtok?: number | null;
  output_cost_per_mtok?: number | null;
  rpm_limit?: number | null;
  tpm_limit?: number | null;
  tags?: string[];
  account_group?: string | null;
  max_concurrent?: number | null;
  excluded_models?: string[];
  proxy_url?: string | null;
  headers?: Record<string, string>;
  max_retries?: number | null;
  models_endpoint?: string | null;
  models_refresh_interval_secs?: number;
}

// ─── Payload Rules Types ─────────────────────────────────

/** A single per-model payload rule with optional model/protocol matching. */
export interface ModelPayloadRule {
  /** Model name patterns (supports `*` / `?` wildcards). Empty matches all. */
  models: string[];
  /** Protocol restriction: "openai", "anthropic", "gemini", or null for all. */
  protocol?: string | null;
  /** Default params to merge (using dotted JSON paths). */
  defaults?: Record<string, unknown>;
  /** Override params that always replace (using dotted JSON paths). */
  overrides?: Record<string, unknown>;
  /** Paths to strip. */
  strip?: string[];
}

/** Per-channel payload manipulation rules. All keys use dotted JSON path notation. */
export interface PayloadRulesConfig {
  /** Default params merged into the request if absent. */
  defaults?: Record<string, unknown>;
  /** Override params that always replace existing values. */
  overrides?: Record<string, unknown>;
  /** Parameter paths to strip from the outgoing request. */
  strip?: string[];
  /** Optional per-model rules applied after channel-level rules. */
  model_rules?: ModelPayloadRule[];
}

export interface GwStatus {
  running: boolean;
  host: string;
  port: number;
}

export async function invokeTauri<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri) {
    throw new Error(`Command "${cmd}" is only available in desktop mode`);
  }
  const { invoke: tauriInvoke } = await import("@tauri-apps/api/core");
  return tauriInvoke(cmd, args);
}

export const PRIORITY_TIERS: Record<number, { color: string }> = {
  1: { color: "var(--color-success)" },
  2: { color: "var(--color-warning)" },
  3: { color: "var(--color-danger)" },
};

export function validateChannelForm(fields: { name: string; baseUrl: string }): string | null {
  if (!fields.name.trim()) return "channels.nameRequired";
  if (!fields.baseUrl.trim()) return "channels.baseUrlRequired";
  if (!fields.baseUrl.startsWith("http://") && !fields.baseUrl.startsWith("https://"))
    return "channels.baseUrlInvalid";
  return null;
}

// ─── MCP Types ──────────────────────────────────────────

/** Serde externally-tagged enum: "stopped" | { running: { tool_count } } | { error: { message } } */
export type McpServerStatus =
  | "stopped"
  | { running: { tool_count: number } }
  | { error: { message: string } };

export interface McpServer {
  id: string;
  name: string;
  command: string;
  args: string[];
  env: Record<string, string>;
  cwd: string | null;
  enabled: boolean;
  expose_tools: boolean;
  status: McpServerStatus;
}

export interface McpToolDetail {
  name: string;
  description: string | null;
}

export interface McpToolInfo {
  server_id: string;
  name: string;
  description: string | null;
}

export interface CreateMcpServerData {
  id: string;
  name: string;
  command: string;
  args?: string[];
  env?: Record<string, string>;
  cwd?: string | null;
  enabled?: boolean;
  expose_tools?: boolean;
}

export interface UpdateMcpServerData {
  name: string;
  command: string;
  args?: string[];
  env?: Record<string, string>;
  cwd?: string | null;
  enabled?: boolean;
  expose_tools?: boolean;
}

// ─── Virtual Key Types ──────────────────────────────────

export interface VirtualKeySpend {
  today: { date: string; cents: number };
  this_month: { month: string; cents: number };
  total_cents: number;
}

export interface VirtualKey {
  id: string;
  name: string;
  key_prefix: string;
  daily_budget_cents: number | null;
  monthly_budget_cents: number | null;
  enabled: boolean;
  spend: VirtualKeySpend;
  created_at: string;
  allowed_ips: string[];
  allowed_models: string[] | null;
  denied_models: string[];
}

export interface CreateVirtualKeyResponse {
  key: VirtualKey;
  plaintext: string;
}

export interface CreateVirtualKeyData {
  name: string;
  daily_budget_cents?: number | null;
  monthly_budget_cents?: number | null;
  allowed_ips?: string[];
  allowed_models?: string[] | null;
  denied_models?: string[];
}

export interface UpdateVirtualKeyData {
  name?: string;
  daily_budget_cents?: number | null;
  monthly_budget_cents?: number | null;
  enabled?: boolean;
  allowed_ips?: string[];
  allowed_models?: string[] | null;
  denied_models?: string[];
}

export interface CacheStats {
  entries: number;
  mode: string;
  hits: number;
  misses: number;
  evictions: number;
  hit_rate_percent: number;
  total_requests: number;
}

export interface GatewayInfo {
  version: string;
  uptime_seconds: number;
  uptime_formatted: string;
  total_channels: number;
  healthy_channels: number;
  active_requests: number;
  cache_entries: number;
  routing_strategy: string;
  max_retries: number;
}

export interface ProviderBudgetEntry {
  provider: string;
  daily_budget_cents: number | null;
  monthly_budget_cents: number | null;
  spend: {
    today: { date: string; cents: number };
    this_month: { month: string; cents: number };
    total_cents: number;
  };
}

export interface GuardrailsConfig {
  enabled: boolean;
  blocked_patterns: string[];
  allowed_patterns: string[];
  max_request_chars: number | null;
  block_message: string;
}

export interface RedemptionCode {
  code: string;
  credits_cents: number;
  used: boolean;
  used_by: string | null;
  used_at: string | null;
  created_at: string;
  expires_at: string | null;
}

export interface NotificationConfig {
  webhook_url: string | null;
  webhook_secret: string | null;
  bark_url: string | null;
  budget_threshold_pct: number;
}

// ─── Model Registry Types ───────────────────────────────

export type ThinkingFormat = "None" | "Budget" | "Level" | "Hybrid";

export interface ModelRegistryItem {
  name: string;
  /** `"builtin"` for hardcoded entries, `"discovered"` for upstream-polled ones. */
  source_type: "builtin" | "discovered";
  /** Endpoint URL the model was discovered from, when not built-in. */
  source: string | null;
  /** Channel ID that owns the discovery endpoint, when applicable. */
  channel_id: string | null;
  /** Channel name that owns the discovery endpoint, when applicable. */
  channel_name: string | null;
  /** Epoch seconds of the last successful refresh, or null for built-in. */
  last_refreshed_secs: number | null;
  supports_thinking: boolean;
  supports_vision: boolean;
  supports_tools: boolean;
  max_context_tokens: number | null;
  thinking_format: ThinkingFormat;
}

export interface ModelRegistryResponse {
  models: ModelRegistryItem[];
  total: number;
}

export const api = {
  listChannels: () => request<Channel[]>("/api/channels"),
  createChannel: (data: Partial<Channel> & { credential_value: string; credential_type?: string }) =>
    request<Channel>("/api/channels", {
      method: "POST",
      body: JSON.stringify(data),
    }),
  updateChannel: (id: string, data: UpdateChannelData) =>
    request<Channel>(`/api/channels/${id}`, {
      method: "PUT",
      body: JSON.stringify(data),
    }),
  deleteChannel: (id: string) => request<void>(`/api/channels/${id}`, { method: "DELETE" }),
  pingChannel: (id: string) =>
    request<{ success: boolean; latency_ms: number }>(`/api/channels/${id}/ping`, {
      method: "POST",
    }),
  channelStatus: (id: string) =>
    request<{ status: string; circuit_open_until: string | null }>(
      `/api/channels/${id}/status`
    ),
  channelCooldown: (id: string) =>
    request<{
      channel_id: string;
      channel_name: string;
      in_cooldown: boolean;
      cooldown_remaining_secs: number;
      circuit_open_until: string | null;
      model_cooldowns: Record<string, string | null>;
    }>(`/api/channels/${id}/cooldown`),
  logs: async (offset = 0, limit = 50): Promise<DispatchLog[]> => {
    const token = localStorage.getItem("admin_token");
    const res = await fetch(`${API_BASE}/api/logs?offset=${offset}&limit=${limit}`, {
      headers: {
        "Content-Type": "application/json",
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
      },
    });
    if (res.status === 401) {
      localStorage.removeItem("admin_token");
      sessionStorage.setItem("auth_expired", "1");
      window.location.reload();
      throw new Error("Unauthorized");
    }
    if (!res.ok) throw new Error(`API error: ${res.status}`);
    const json: PaginatedEnvelope<DispatchLog[]> = await res.json();
    return json.data;
  },
  stats: () => request<DispatchStats>("/api/stats"),
  costStats: () => request<CostStats>("/api/stats/cost"),
  quota: () => request<QuotaInfo[]>("/api/quota"),
  usageHistory: (hours = 24) =>
    request<UsageHistory>(`/api/stats/usage?hours=${hours}`),
  mcp: {
    listServers: () => request<McpServer[]>("/api/mcp/servers"),
    createServer: (data: CreateMcpServerData) =>
      request<McpServer>("/api/mcp/servers", {
        method: "POST",
        body: JSON.stringify(data),
      }),
    updateServer: (id: string, data: UpdateMcpServerData) =>
      request<McpServer>(`/api/mcp/servers/${id}`, {
        method: "PUT",
        body: JSON.stringify(data),
      }),
    deleteServer: (id: string) => request<void>(`/api/mcp/servers/${id}`, { method: "DELETE" }),
    startServer: (id: string) =>
      request<{ ok: boolean }>(`/api/mcp/servers/${id}/start`, { method: "POST" }),
    stopServer: (id: string) =>
      request<{ ok: boolean }>(`/api/mcp/servers/${id}/stop`, { method: "POST" }),
    listServerTools: (id: string) =>
      request<McpToolDetail[]>(`/api/mcp/servers/${id}/tools`),
    listAllTools: () => request<McpToolInfo[]>("/api/mcp/tools"),
  },
  virtualKeys: {
    list: () => request<VirtualKey[]>("/api/virtual-keys"),
    create: (data: CreateVirtualKeyData) =>
      request<CreateVirtualKeyResponse>("/api/virtual-keys", {
        method: "POST",
        body: JSON.stringify(data),
      }),
    update: (id: string, data: UpdateVirtualKeyData) =>
      request<VirtualKey>(`/api/virtual-keys/${id}`, {
        method: "PUT",
        body: JSON.stringify(data),
      }),
    delete: (id: string) => request<void>(`/api/virtual-keys/${id}`, { method: "DELETE" }),
  },
  cacheStats: () => request<CacheStats>("/api/cache/stats"),
  flushCache: () =>
    request<{ flushed: boolean; remaining: number }>("/api/cache/flush", {
      method: "POST",
    }),
  reloadConfig: () =>
    request<{ created: number; updated: number; removed: number }>("/api/config/reload", {
      method: "POST",
    }),
  gatewayInfo: () => request<GatewayInfo>("/api/gateway/info"),
  providerBudgets: () => request<ProviderBudgetEntry[]>("/api/provider-budgets"),

  // Batch channel operations
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
      body: JSON.stringify({ ids, tags }),
    }),

  // Reset circuit breaker
  resetCircuit: (id: string) =>
    request<{ ok: boolean }>(`/api/channels/${id}/reset-circuit`, {
      method: "POST",
    }),

  // Audit log
  auditLog: (limit = 100) =>
    request<Array<{ timestamp: string; action: string; actor: string; target: string; details: string | null }>>(
      `/api/audit-log?limit=${limit}`
    ),

  // Metrics (Prometheus text format)
  metrics: async (): Promise<string> => {
    const token = localStorage.getItem("admin_token");
    const res = await fetch(`${API_BASE}/metrics`, {
      headers: {
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
      },
    });
    if (!res.ok) throw new Error(`API error: ${res.status}`);
    return res.text();
  },

  // Guardrails config
  guardrailsConfig: () => request<GuardrailsConfig>("/api/guardrails"),
  updateGuardrails: (config: Partial<GuardrailsConfig>) =>
    request<GuardrailsConfig>("/api/guardrails", {
      method: "PUT",
      body: JSON.stringify(config),
    }),

  // Redemption codes
  redemptionCodes: {
    list: () => request<RedemptionCode[]>("/api/redemption-codes"),
    create: (data: { credits_cents: number; expires_at?: string | null }) =>
      request<RedemptionCode>("/api/redemption-codes", {
        method: "POST",
        body: JSON.stringify(data),
      }),
    redeem: (code: string, userId?: string) =>
      request<{ credits_cents: number }>("/api/redemption-codes/redeem", {
        method: "POST",
        body: JSON.stringify({ code, user_id: userId }),
      }),
    delete: (code: string) =>
      request<void>(`/api/redemption-codes/${code}`, { method: "DELETE" }),
  },

  // Notification config
  notificationConfig: () => request<NotificationConfig>("/api/notifications"),
  updateNotification: (config: Partial<NotificationConfig>) =>
    request<NotificationConfig>("/api/notifications", {
      method: "PUT",
      body: JSON.stringify(config),
    }),

  // Channel auto-test
  testChannel: (id: string) =>
    request<{ healthy: boolean; latency_ms: number | null; error: string | null }>(
      `/api/channels/${id}/test`,
      { method: "POST" }
    ),
  testAllChannels: () =>
    request<Array<{ channel_id: string; channel_name: string; healthy: boolean; latency_ms: number | null; error: string | null }>>(
      "/api/channels/test-all",
      { method: "POST" }
    ),

  // MCP health
  mcpHealth: () =>
    request<Array<{ name: string; healthy: boolean; last_check: string | null; last_error: string | null; consecutive_failures: number }>>(
      "/api/mcp/health"
    ),

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

  // Per-channel payload rules (runtime override)
  updatePayloadRules: (channelId: string, rules: PayloadRulesConfig) =>
    request<{ channel_id: string; updated: boolean }>(
      `/api/channels/${channelId}/payload-rules`,
      {
        method: "PUT",
        body: JSON.stringify(rules),
      }
    ),
};

// ─── Mock mode ──────────────────────────────────────────
// When mock mode is enabled, replace api methods with mock implementations.
// This allows zero changes to consumer code — all existing imports of { api }
// automatically pick up mock data.
import { isMockMode, mockApi } from "./mock";

if (isMockMode()) {
  Object.assign(api, mockApi);
}
