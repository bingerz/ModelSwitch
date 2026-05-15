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

async function request<T>(path: string, options?: RequestInit): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    headers: { "Content-Type": "application/json" },
    ...options,
  });
  if (!res.ok) throw new Error(`API error: ${res.status}`);
  return res.json();
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
  logs: (offset = 0, limit = 50) =>
    request<DispatchLog[]>(`/api/logs?offset=${offset}&limit=${limit}`),
  stats: () => request<DispatchStats>("/api/stats"),
  costStats: () => request<CostStats>("/api/stats/cost"),
  quota: () => request<QuotaInfo[]>("/api/quota"),
  usageHistory: (hours = 24) =>
    request<UsageHistory>(`/api/stats/usage?hours=${hours}`),
};
