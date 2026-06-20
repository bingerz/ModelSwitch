const API_BASE = "http://127.0.0.1:8080";

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

async function request<T>(path: string, options?: RequestInit): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    headers: { "Content-Type": "application/json" },
    ...options,
  });
  if (!res.ok) throw new Error(`API error: ${res.status}`);
  const json = await res.json();
  // Auto-unwrap ApiResponse<T> envelope used by all admin endpoints
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
}

export interface GwStatus {
  running: boolean;
  host: string;
  port: number;
}

export async function invokeTauri<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke: tauriInvoke } = await import("@tauri-apps/api/core");
  return tauriInvoke(cmd, args);
}

export const PRIORITY_TIERS: Record<number, { label: string; desc: string; color: string }> = {
  1: { label: "Priority 1", desc: "Free / Subscription", color: "var(--color-success)" },
  2: { label: "Priority 2", desc: "Economy API", color: "var(--color-warning)" },
  3: { label: "Priority 3", desc: "Official API", color: "var(--color-danger)" },
};

export function validateChannelForm(fields: { name: string; baseUrl: string }): string | null {
  if (!fields.name.trim()) return "Name is required";
  if (!fields.baseUrl.trim()) return "Base URL is required";
  if (!fields.baseUrl.startsWith("http://") && !fields.baseUrl.startsWith("https://"))
    return "Base URL must start with http:// or https://";
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
}

export interface CreateVirtualKeyResponse {
  key: VirtualKey;
  plaintext: string;
}

export interface CreateVirtualKeyData {
  name: string;
  daily_budget_cents?: number | null;
  monthly_budget_cents?: number | null;
}

export interface UpdateVirtualKeyData {
  name?: string;
  daily_budget_cents?: number | null;
  monthly_budget_cents?: number | null;
  enabled?: boolean;
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
  deleteChannel: async (id: string) => {
    const res = await fetch(`${API_BASE}/api/channels/${id}`, { method: "DELETE" });
    if (!res.ok) throw new Error(`API error: ${res.status}`);
    return res;
  },
  pingChannel: (id: string) =>
    request<{ success: boolean; latency_ms: number }>(`/api/channels/${id}/ping`, {
      method: "POST",
    }),
  channelStatus: (id: string) =>
    request<{ status: string; circuit_open_until: string | null }>(
      `/api/channels/${id}/status`
    ),
  logs: async (offset = 0, limit = 50): Promise<DispatchLog[]> => {
    const res = await fetch(`${API_BASE}/api/logs?offset=${offset}&limit=${limit}`, {
      headers: { "Content-Type": "application/json" },
    });
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
    deleteServer: async (id: string) => {
      const res = await fetch(`${API_BASE}/api/mcp/servers/${id}`, { method: "DELETE" });
      if (!res.ok) throw new Error(`API error: ${res.status}`);
      return res;
    },
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
    delete: async (id: string) => {
      const res = await fetch(`${API_BASE}/api/virtual-keys/${id}`, { method: "DELETE" });
      if (!res.ok) throw new Error(`API error: ${res.status}`);
      return res;
    },
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
};
