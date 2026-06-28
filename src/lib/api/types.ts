// Shared API types and interfaces.
// All domain modules import from here; the index re-exports for consumers.

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
export interface PaginatedEnvelope<T> {
  data: T;
  total: number;
  offset: number;
  limit: number;
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
  rpm_limit: number | null;
  tpm_limit: number | null;
  expires_at: string | null;
  group: string | null;
}

export interface CreateVirtualKeyResponse {
  key: VirtualKey;
  plaintext: string;
}

export interface PaginatedVirtualKeys {
  data: VirtualKey[];
  total: number;
  page: number;
  limit: number;
}

export interface ListVirtualKeysParams {
  page?: number;
  limit?: number;
  search?: string;
  group?: string;
}

export interface BatchCreateVirtualKeyData {
  count: number;
  name_prefix: string;
  daily_budget_cents?: number | null;
  monthly_budget_cents?: number | null;
  allowed_models?: string[] | null;
  allowed_ips?: string[];
  rpm_limit?: number | null;
  tpm_limit?: number | null;
  expires_at?: string | null;
  group?: string | null;
}

export interface BatchCreateVirtualKeyItem {
  id: string;
  name: string;
  key: string;
  key_prefix: string;
  daily_budget_cents: number | null;
  monthly_budget_cents: number | null;
  enabled: boolean;
  created_at: string;
  spend: VirtualKeySpend;
  allowed_models: string[] | null;
  denied_models: string[];
  allowed_ips: string[];
}

export interface CreateVirtualKeyData {
  name: string;
  daily_budget_cents?: number | null;
  monthly_budget_cents?: number | null;
  allowed_ips?: string[];
  allowed_models?: string[] | null;
  denied_models?: string[];
  rpm_limit?: number | null;
  tpm_limit?: number | null;
  expires_at?: string | null;
  group?: string | null;
}

export interface UpdateVirtualKeyData {
  name?: string;
  daily_budget_cents?: number | null;
  monthly_budget_cents?: number | null;
  enabled?: boolean;
  allowed_ips?: string[];
  allowed_models?: string[] | null;
  denied_models?: string[];
  rpm_limit?: number | null;
  tpm_limit?: number | null;
  expires_at?: string | null;
  group?: string | null;
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
  smtp_enabled: boolean;
  smtp_host: string | null;
  smtp_port: number | null;
  smtp_username: string | null;
  smtp_password: string | null;
  smtp_from: string | null;
  smtp_admin_email: string | null;
  smtp_use_tls: boolean;
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

// ─── Reports Types ──────────────────────────────────────

export interface UsageReportRow {
  date: string;
  key_id: string | null;
  key_name: string | null;
  group: string | null;
  requests: number;
  total_tokens: number;
  estimated_cost_cents: number;
  input_tokens: number;
  output_tokens: number;
}

export interface UsageReport {
  rows: UsageReportRow[];
  summary: {
    total_requests: number;
    total_tokens: number;
    total_cost_cents: number;
    avg_daily_cost_cents: number;
  };
}

export interface UsageReportParams {
  key_id?: string;
  group?: string;
  from?: string;  // ISO date
  to?: string;
  group_by?: 'day' | 'week' | 'month';
}

// ─── Audit / Health composite types (inline in api.ts) ──

export interface AuditLogEntry {
  timestamp: string;
  action: string;
  actor: string;
  target: string;
  details: string | null;
}

export interface ChannelTestResult {
  channel_id: string;
  channel_name: string;
  healthy: boolean;
  latency_ms: number | null;
  error: string | null;
}

export interface SingleChannelTestResult {
  healthy: boolean;
  latency_ms: number | null;
  error: string | null;
}

export interface ChannelDiagnosticsResult {
  channel_id: string;
  channel_name: string;
  auth_status: "authenticated" | "unauthenticated" | "error";
  available_models: string[];
  status_code: number | null;
  latency_ms: number;
  error: string | null;
  tested_at: string;
}

export interface McpHealthEntry {
  name: string;
  healthy: boolean;
  last_check: string | null;
  last_error: string | null;
  consecutive_failures: number;
}

export interface ChannelCooldownInfo {
  channel_id: string;
  channel_name: string;
  in_cooldown: boolean;
  cooldown_remaining_secs: number;
  circuit_open_until: string | null;
  model_cooldowns: Record<string, string | null>;
}

export interface ChannelStatusInfo {
  status: string;
  circuit_open_until: string | null;
}

export interface PingResult {
  success: boolean;
  latency_ms: number;
}

export interface FlushCacheResult {
  flushed: boolean;
  remaining: number;
}

export interface ReloadConfigResult {
  created: number;
  updated: number;
  removed: number;
}
