// Deterministic mock data generators for mock mode.
// All data is generated relative to `now` so timestamps always look fresh.

import type {
  Channel,
  DispatchLog,
  DispatchStats,
  CostStats,
  UsageBucket,
  UsageHistory,
  QuotaInfo,
  VirtualKey,
  McpServer,
  McpToolDetail,
  McpToolInfo,
  CacheStats,
  GatewayInfo,
  ProviderBudgetEntry,
} from "./api";

// ─── Time constants ─────────────────────────────────────

const SEC = 1_000;
const MIN = 60 * SEC;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

function isoOffset(now: Date, ms: number): string {
  return new Date(now.getTime() + ms).toISOString();
}

function daysAgo(now: Date, days: number, extraHours = 0): string {
  return isoOffset(now, -(days * DAY) + extraHours * HOUR);
}

// ─── Seeded PRNG (deterministic LCG) ────────────────────

class LCG {
  private s: number;
  constructor(seed: number) {
    this.s = seed >>> 0;
  }
  next(): number {
    this.s = (Math.imul(this.s, 1664525) + 1013904223) >>> 0;
    return this.s / 0xffffffff;
  }
  int(min: number, max: number): number {
    return Math.floor(this.next() * (max - min + 1)) + min;
  }
  pick<T>(arr: readonly T[]): T {
    return arr[this.int(0, arr.length - 1)];
  }
  bool(p: number): boolean {
    return this.next() < p;
  }
}

// ─── Model -> Channel mapping ───────────────────────────

interface ModelChannel {
  model: string;
  channelId: string;
  channelName: string;
  priority: number;
  baseLatency: number;
  inputCostPerMtok: number;
  outputCostPerMtok: number;
}

const MODEL_CHANNELS: readonly ModelChannel[] = [
  { model: "gpt-4o", channelId: "ch-openai-p1", channelName: "OpenAI Primary", priority: 1, baseLatency: 900, inputCostPerMtok: 2.5, outputCostPerMtok: 10.0 },
  { model: "gpt-4o-mini", channelId: "ch-openai-p1", channelName: "OpenAI Primary", priority: 1, baseLatency: 500, inputCostPerMtok: 0.15, outputCostPerMtok: 0.6 },
  { model: "claude-3-5-sonnet", channelId: "ch-anthropic-p1", channelName: "Anthropic Primary", priority: 1, baseLatency: 1000, inputCostPerMtok: 3.0, outputCostPerMtok: 15.0 },
  { model: "claude-3-5-haiku", channelId: "ch-anthropic-p1", channelName: "Anthropic Primary", priority: 1, baseLatency: 600, inputCostPerMtok: 0.8, outputCostPerMtok: 4.0 },
  { model: "gemini-2.0-flash", channelId: "ch-gemini-p2", channelName: "Gemini Secondary", priority: 2, baseLatency: 1100, inputCostPerMtok: 0.075, outputCostPerMtok: 0.3 },
  { model: "llama-3.3-70b", channelId: "ch-groq-p1", channelName: "Groq Fast", priority: 1, baseLatency: 350, inputCostPerMtok: 0.59, outputCostPerMtok: 0.79 },
];

// ─── Channels ───────────────────────────────────────────

export function buildChannels(now: Date): Channel[] {
  return [
    {
      id: "ch-openai-p1",
      name: "OpenAI Primary",
      provider: "openai",
      priority: 1,
      weight: 70,
      cost_per_token: null,
      input_cost_per_mtok: 2.5,
      output_cost_per_mtok: 10.0,
      enabled: true,
      status: "healthy",
      circuit_open_until: null,
      base_url: "https://api.openai.com/v1",
      model_mapping: { "gpt-4o": "gpt-4o", "gpt-4o-mini": "gpt-4o-mini" },
      created_at: daysAgo(now, 28, 4),
      updated_at: daysAgo(now, 3, 2),
      avg_latency_ms: 850,
      consecutive_failures: 0,
      cooldown_minutes: null,
      rpm_limit: 5000,
      tpm_limit: 2_000_000,
      tags: [],
      account_group: null,
      max_concurrent: null,
      excluded_models: [],
      api_keys: [],
      proxy_url: null,
      headers: {},
      max_retries: null,
      models_endpoint: null,
      models_refresh_interval_secs: 0,
    },
    {
      id: "ch-openai-p2",
      name: "OpenAI Secondary",
      provider: "openai",
      priority: 2,
      weight: 30,
      cost_per_token: null,
      input_cost_per_mtok: 2.5,
      output_cost_per_mtok: 10.0,
      enabled: true,
      status: "healthy",
      circuit_open_until: null,
      base_url: "https://api.openai.com/v1",
      model_mapping: { "gpt-4o": "gpt-4o", "gpt-4o-mini": "gpt-4o-mini" },
      created_at: daysAgo(now, 25, 8),
      updated_at: daysAgo(now, 1, 6),
      avg_latency_ms: 1200,
      consecutive_failures: 0,
      cooldown_minutes: null,
      rpm_limit: 3000,
      tpm_limit: 1_000_000,
      tags: [],
      account_group: null,
      max_concurrent: null,
      excluded_models: [],
      api_keys: [],
      proxy_url: null,
      headers: {},
      max_retries: null,
      models_endpoint: null,
      models_refresh_interval_secs: 0,
    },
    {
      id: "ch-anthropic-p1",
      name: "Anthropic Primary",
      provider: "anthropic",
      priority: 1,
      weight: 65,
      cost_per_token: null,
      input_cost_per_mtok: 3.0,
      output_cost_per_mtok: 15.0,
      enabled: true,
      status: "healthy",
      circuit_open_until: null,
      base_url: "https://api.anthropic.com",
      model_mapping: { "claude-3-5-sonnet": "claude-3-5-sonnet-20241022", "claude-3-5-haiku": "claude-3-5-haiku-20241022" },
      created_at: daysAgo(now, 22, 2),
      updated_at: daysAgo(now, 5, 10),
      avg_latency_ms: 920,
      consecutive_failures: 0,
      cooldown_minutes: null,
      rpm_limit: null,
      tpm_limit: null,
      tags: [],
      account_group: null,
      max_concurrent: null,
      excluded_models: [],
      api_keys: [],
      proxy_url: null,
      headers: {},
      max_retries: null,
      models_endpoint: null,
      models_refresh_interval_secs: 0,
    },
    {
      id: "ch-anthropic-p2",
      name: "Anthropic Secondary",
      provider: "anthropic",
      priority: 2,
      weight: 35,
      cost_per_token: null,
      input_cost_per_mtok: 3.0,
      output_cost_per_mtok: 15.0,
      enabled: true,
      status: "circuit_open",
      circuit_open_until: isoOffset(now, 5 * MIN),
      base_url: "https://api.anthropic.com",
      model_mapping: { "claude-3-5-sonnet": "claude-3-5-sonnet-20241022", "claude-3-5-haiku": "claude-3-5-haiku-20241022" },
      created_at: daysAgo(now, 20, 14),
      updated_at: daysAgo(now, 0, 0),
      avg_latency_ms: 2100,
      consecutive_failures: 3,
      cooldown_minutes: 5,
      rpm_limit: null,
      tpm_limit: null,
      tags: [],
      account_group: null,
      max_concurrent: null,
      excluded_models: [],
      api_keys: [],
      proxy_url: null,
      headers: {},
      max_retries: null,
      models_endpoint: null,
      models_refresh_interval_secs: 0,
    },
    {
      id: "ch-gemini-p2",
      name: "Gemini Secondary",
      provider: "google",
      priority: 2,
      weight: 40,
      cost_per_token: null,
      input_cost_per_mtok: 0.075,
      output_cost_per_mtok: 0.3,
      enabled: true,
      status: "healthy",
      circuit_open_until: null,
      base_url: "https://generativelanguage.googleapis.com/v1beta",
      model_mapping: { "gemini-2.0-flash": "gemini-2.0-flash-exp" },
      created_at: daysAgo(now, 18, 6),
      updated_at: daysAgo(now, 2, 4),
      avg_latency_ms: 1100,
      consecutive_failures: 0,
      cooldown_minutes: null,
      rpm_limit: 1000,
      tpm_limit: null,
      tags: [],
      account_group: null,
      max_concurrent: null,
      excluded_models: [],
      api_keys: [],
      proxy_url: null,
      headers: {},
      max_retries: null,
      models_endpoint: null,
      models_refresh_interval_secs: 0,
    },
    {
      id: "ch-groq-p1",
      name: "Groq Fast",
      provider: "groq",
      priority: 1,
      weight: 55,
      cost_per_token: null,
      input_cost_per_mtok: 0.59,
      output_cost_per_mtok: 0.79,
      enabled: true,
      status: "healthy",
      circuit_open_until: null,
      base_url: "https://api.groq.com/openai/v1",
      model_mapping: { "llama-3.3-70b": "llama-3.3-70b-versatile" },
      created_at: daysAgo(now, 15, 10),
      updated_at: daysAgo(now, 4, 8),
      avg_latency_ms: 380,
      consecutive_failures: 0,
      cooldown_minutes: null,
      rpm_limit: 30,
      tpm_limit: 15_000,
      tags: [],
      account_group: null,
      max_concurrent: null,
      excluded_models: [],
      api_keys: [],
      proxy_url: null,
      headers: {},
      max_retries: null,
      models_endpoint: null,
      models_refresh_interval_secs: 0,
    },
    {
      id: "ch-openrouter-p3",
      name: "OpenRouter Tertiary",
      provider: "openrouter",
      priority: 3,
      weight: 20,
      cost_per_token: null,
      input_cost_per_mtok: 5.0,
      output_cost_per_mtok: 15.0,
      enabled: true,
      status: "healthy",
      circuit_open_until: null,
      base_url: "https://openrouter.ai/api/v1",
      model_mapping: { "gpt-4o": "openai/gpt-4o", "claude-3-5-sonnet": "anthropic/claude-3.5-sonnet" },
      created_at: daysAgo(now, 12, 3),
      updated_at: daysAgo(now, 6, 12),
      avg_latency_ms: 1450,
      consecutive_failures: 0,
      cooldown_minutes: null,
      rpm_limit: null,
      tpm_limit: null,
      tags: [],
      account_group: null,
      max_concurrent: null,
      excluded_models: [],
      api_keys: [],
      proxy_url: null,
      headers: {},
      max_retries: null,
      models_endpoint: null,
      models_refresh_interval_secs: 0,
    },
    {
      id: "ch-deepseek-p2",
      name: "DeepSeek Secondary",
      provider: "deepseek",
      priority: 2,
      weight: 25,
      cost_per_token: null,
      input_cost_per_mtok: 0.14,
      output_cost_per_mtok: 0.28,
      enabled: false,
      status: "disabled",
      circuit_open_until: null,
      base_url: "https://api.deepseek.com",
      model_mapping: { "deepseek-chat": "deepseek-chat" },
      created_at: daysAgo(now, 10, 16),
      updated_at: daysAgo(now, 7, 2),
      avg_latency_ms: 0,
      consecutive_failures: 0,
      cooldown_minutes: null,
      rpm_limit: null,
      tpm_limit: null,
      tags: [],
      account_group: null,
      max_concurrent: null,
      excluded_models: [],
      api_keys: [],
      proxy_url: null,
      headers: {},
      max_retries: null,
      models_endpoint: null,
      models_refresh_interval_secs: 0,
    },
  ];
}

// ─── Dispatch Stats ─────────────────────────────────────

export function buildStats(): DispatchStats {
  return {
    total_requests: 12847,
    successes: 12611,
    failures: 236,
    avg_latency_ms: 947,
  };
}

// ─── Dispatch Logs ──────────────────────────────────────

export function buildLogs(now: Date): DispatchLog[] {
  const rng = new LCG(42);
  const logs: DispatchLog[] = [];
  const twoHoursMs = 2 * HOUR;

  for (let i = 0; i < 80; i++) {
    const mc = rng.pick(MODEL_CHANNELS);
    const success = rng.bool(0.85);
    const latency = Math.max(150, mc.baseLatency + rng.int(-200, 800));
    const inputTokens = rng.int(50, 8000);
    const outputTokens = success ? rng.int(50, 4000) : 0;
    const cacheHit = rng.bool(0.3) ? rng.int(0, Math.floor(inputTokens * 0.5)) : 0;
    const cacheMiss = inputTokens - cacheHit;
    const cost =
      success
        ? +((inputTokens * mc.inputCostPerMtok + outputTokens * mc.outputCostPerMtok) / 1_000_000).toFixed(6)
        : 0;

    let triggerReason: string | null = null;
    if (!success) {
      triggerReason = rng.pick(["error", "timeout", "rate_limit"] as const);
    } else if (rng.bool(0.08)) {
      triggerReason = rng.pick(["fallback", "retry", "circuit_open"] as const);
    }

    // Distribute timestamps evenly across 2h window with jitter
    const fraction = i / 80;
    const jitter = rng.int(0, 45 * SEC);
    const timestamp = isoOffset(now, -Math.floor(fraction * twoHoursMs) - jitter);

    logs.push({
      id: `log-${String(i + 1).padStart(4, "0")}`,
      timestamp,
      request_model: mc.model,
      channel_id: mc.channelId,
      channel_name: mc.channelName,
      channel_priority: mc.priority,
      retry_count: triggerReason === "retry" ? 1 : 0,
      trigger_reason: triggerReason,
      latency_ms: latency,
      success,
      estimated_cost: success ? cost : null,
      input_tokens: inputTokens,
      output_tokens: outputTokens,
      cache_hit_tokens: cacheHit,
      cache_miss_tokens: cacheMiss,
    });
  }

  logs.sort((a, b) => b.timestamp.localeCompare(a.timestamp));
  return logs;
}

// ─── Usage History ──────────────────────────────────────

export function buildUsageHistory(now: Date): UsageHistory {
  const rng = new LCG(123);
  const buckets: UsageBucket[] = [];
  let totalInput = 0;
  let totalOutput = 0;
  let totalRequests = 0;
  let totalCost = 0;

  for (let h = 23; h >= 0; h--) {
    const bucketTime = isoOffset(now, -h * HOUR - 30 * MIN);
    const hourOfDay = new Date(now.getTime() - h * HOUR).getHours();
    const isLateNight = hourOfDay >= 22 || hourOfDay < 6;
    const multiplier = isLateNight ? 0.25 : 1.0;

    for (const mc of MODEL_CHANNELS) {
      const isSmall = mc.model.includes("mini") || mc.model.includes("haiku") || mc.model.includes("flash");
      const baseRequests = isSmall ? rng.int(30, 80) : rng.int(15, 50);
      const requests = Math.max(1, Math.floor(baseRequests * multiplier));
      const inputTokens = Math.floor(rng.int(500, 5000) * multiplier);
      const outputTokens = Math.floor(rng.int(200, 3000) * multiplier);
      const cacheHit = Math.floor(inputTokens * 0.3);
      const cacheMiss = inputTokens - cacheHit;
      const cost = +(
        (inputTokens * mc.inputCostPerMtok + outputTokens * mc.outputCostPerMtok) /
        1_000_000
      ).toFixed(6);

      buckets.push({
        timestamp: bucketTime,
        channel_id: mc.channelId,
        channel_name: mc.channelName,
        model: mc.model,
        input_tokens: inputTokens,
        output_tokens: outputTokens,
        cache_hit_tokens: cacheHit,
        cache_miss_tokens: cacheMiss,
        request_count: requests,
        estimated_cost: cost,
      });

      totalInput += inputTokens;
      totalOutput += outputTokens;
      totalRequests += requests;
      totalCost += cost;
    }
  }

  return {
    buckets,
    total_input_tokens: totalInput,
    total_output_tokens: totalOutput,
    total_requests: totalRequests,
    total_cost: +totalCost.toFixed(4),
  };
}

// ─── Cost Stats ─────────────────────────────────────────

export function buildCostStats(): CostStats {
  return {
    total_requests: 12847,
    total_estimated_cost: 8.4723,
    priority_breakdown: [
      { priority: 1, requests: 6000, estimated_cost: 2.1 },
      { priority: 2, requests: 4500, estimated_cost: 3.8 },
      { priority: 3, requests: 2347, estimated_cost: 2.5 },
    ],
    model_counts: {
      "gpt-4o": 3812,
      "gpt-4o-mini": 2934,
      "claude-3-5-sonnet": 2104,
      "claude-3-5-haiku": 1823,
      "gemini-2.0-flash": 1102,
      "llama-3.3-70b": 1072,
    },
    total_input_tokens: 2_412_000,
    total_output_tokens: 1_089_000,
  };
}

// ─── Quota Info ─────────────────────────────────────────

export function buildQuota(now: Date): QuotaInfo[] {
  const updated = isoOffset(now, -2 * MIN);
  const resetsAt = isoOffset(now, 3600 * SEC);

  return [
    {
      channel_id: "ch-openai-p1",
      channel_name: "OpenAI Primary",
      provider: "openai",
      balance: 47.32,
      limit: 100,
      usage: 52.68,
      remaining_tokens: null,
      remaining_requests: 4231,
      plan_status: "Pay-as-you-go",
      expires_at: null,
      items: [
        { label: "Plan", value: "Pay-as-you-go" },
        { label: "Tier", value: "Tier 2" },
        { label: "Hard limit", value: "$100.00" },
      ],
      groups: [
        { window: "RPM", utilization_pct: 84.6, resets_at: null },
        { window: "TPM", utilization_pct: 60.0, resets_at: null },
      ],
      compact_text: null,
      rate_limit_remaining_req: 4231,
      rate_limit_limit_req: 5000,
      rate_limit_remaining_tok: 1_200_000,
      rate_limit_limit_tok: 2_000_000,
      rate_limit_updated_at: updated,
      total_input_tokens: 1_180_000,
      total_output_tokens: 530_000,
      total_cache_hit_tokens: 340_000,
      total_cache_miss_tokens: 840_000,
      total_requests_counted: 5800,
      total_estimated_cost: 4.21,
      source: "openai_dashboard",
      updated_at: updated,
      error: null,
    },
    {
      channel_id: "ch-openai-p2",
      channel_name: "OpenAI Secondary",
      provider: "openai",
      balance: 47.32,
      limit: 100,
      usage: 52.68,
      remaining_tokens: null,
      remaining_requests: 4231,
      plan_status: "Pay-as-you-go",
      expires_at: null,
      items: [
        { label: "Plan", value: "Pay-as-you-go" },
        { label: "Tier", value: "Tier 2" },
        { label: "Hard limit", value: "$100.00" },
      ],
      groups: [
        { window: "RPM", utilization_pct: 84.6, resets_at: null },
        { window: "TPM", utilization_pct: 60.0, resets_at: null },
      ],
      compact_text: null,
      rate_limit_remaining_req: 4231,
      rate_limit_limit_req: 5000,
      rate_limit_remaining_tok: 1_200_000,
      rate_limit_limit_tok: 2_000_000,
      rate_limit_updated_at: updated,
      total_input_tokens: 1_180_000,
      total_output_tokens: 530_000,
      total_cache_hit_tokens: 340_000,
      total_cache_miss_tokens: 840_000,
      total_requests_counted: 5800,
      total_estimated_cost: 4.21,
      source: "openai_dashboard",
      updated_at: updated,
      error: null,
    },
    {
      channel_id: "ch-anthropic-p1",
      channel_name: "Anthropic Primary",
      provider: "anthropic",
      balance: 22.18,
      limit: 50,
      usage: 27.82,
      remaining_tokens: null,
      remaining_requests: null,
      plan_status: "Tier 1 Subscription",
      expires_at: null,
      items: [
        { label: "Plan", value: "Tier 1 Subscription" },
        { label: "Monthly limit", value: "$50.00" },
      ],
      groups: [],
      compact_text: null,
      rate_limit_remaining_req: null,
      rate_limit_limit_req: null,
      rate_limit_remaining_tok: null,
      rate_limit_limit_tok: null,
      rate_limit_updated_at: null,
      total_input_tokens: 680_000,
      total_output_tokens: 310_000,
      total_cache_hit_tokens: 180_000,
      total_cache_miss_tokens: 500_000,
      total_requests_counted: 3200,
      total_estimated_cost: 3.45,
      source: "anthropic_api",
      updated_at: updated,
      error: null,
    },
    {
      channel_id: "ch-anthropic-p2",
      channel_name: "Anthropic Secondary",
      provider: "anthropic",
      balance: null,
      limit: null,
      usage: null,
      remaining_tokens: null,
      remaining_requests: null,
      plan_status: null,
      expires_at: null,
      items: [],
      groups: [],
      compact_text: null,
      rate_limit_remaining_req: null,
      rate_limit_limit_req: null,
      rate_limit_remaining_tok: null,
      rate_limit_limit_tok: null,
      rate_limit_updated_at: null,
      total_input_tokens: null,
      total_output_tokens: null,
      total_cache_hit_tokens: null,
      total_cache_miss_tokens: null,
      total_requests_counted: null,
      total_estimated_cost: null,
      source: "anthropic_api",
      updated_at: updated,
      error: "Failed to fetch quota: invalid API key",
    },
    {
      channel_id: "ch-gemini-p2",
      channel_name: "Gemini Secondary",
      provider: "google",
      balance: 4.21,
      limit: 20,
      usage: 15.79,
      remaining_tokens: null,
      remaining_requests: null,
      plan_status: "Free Tier",
      expires_at: null,
      items: [
        { label: "Plan", value: "Free Tier" },
        { label: "Monthly limit", value: "$20.00" },
      ],
      groups: [
        { window: "Daily", utilization_pct: 79.0, resets_at: resetsAt },
      ],
      compact_text: null,
      rate_limit_remaining_req: null,
      rate_limit_limit_req: null,
      rate_limit_remaining_tok: null,
      rate_limit_limit_tok: null,
      rate_limit_updated_at: null,
      total_input_tokens: 320_000,
      total_output_tokens: 140_000,
      total_cache_hit_tokens: 60_000,
      total_cache_miss_tokens: 260_000,
      total_requests_counted: 1102,
      total_estimated_cost: 0.03,
      source: "gemini_api",
      updated_at: updated,
      error: null,
    },
    {
      channel_id: "ch-groq-p1",
      channel_name: "Groq Fast",
      provider: "groq",
      balance: null,
      limit: null,
      usage: null,
      remaining_tokens: null,
      remaining_requests: 2,
      plan_status: "Free Tier",
      expires_at: null,
      items: [
        { label: "Plan", value: "Free Tier" },
        { label: "RPM limit", value: "30" },
        { label: "TPM limit", value: "15000" },
      ],
      groups: [
        { window: "RPM", utilization_pct: 93.3, resets_at: isoOffset(now, 60 * SEC) },
        { window: "TPM", utilization_pct: 93.3, resets_at: isoOffset(now, 60 * SEC) },
      ],
      compact_text: null,
      rate_limit_remaining_req: 2,
      rate_limit_limit_req: 30,
      rate_limit_remaining_tok: 1_000,
      rate_limit_limit_tok: 15_000,
      rate_limit_updated_at: updated,
      total_input_tokens: 230_000,
      total_output_tokens: 110_000,
      total_cache_hit_tokens: 0,
      total_cache_miss_tokens: 230_000,
      total_requests_counted: 1072,
      total_estimated_cost: 0.25,
      source: "groq_api",
      updated_at: updated,
      error: null,
    },
    {
      channel_id: "ch-openrouter-p3",
      channel_name: "OpenRouter Tertiary",
      provider: "openrouter",
      balance: 15.5,
      limit: 50,
      usage: 34.5,
      remaining_tokens: null,
      remaining_requests: null,
      plan_status: "Pay-as-you-go",
      expires_at: null,
      items: [
        { label: "Plan", value: "Pay-as-you-go" },
        { label: "Credit limit", value: "$50.00" },
      ],
      groups: [],
      compact_text: null,
      rate_limit_remaining_req: null,
      rate_limit_limit_req: null,
      rate_limit_remaining_tok: null,
      rate_limit_limit_tok: null,
      rate_limit_updated_at: null,
      total_input_tokens: 0,
      total_output_tokens: 0,
      total_cache_hit_tokens: 0,
      total_cache_miss_tokens: 0,
      total_requests_counted: 0,
      total_estimated_cost: 0,
      source: "openrouter_api",
      updated_at: updated,
      error: null,
    },
    {
      channel_id: "ch-deepseek-p2",
      channel_name: "DeepSeek Secondary",
      provider: "deepseek",
      balance: null,
      limit: null,
      usage: null,
      remaining_tokens: null,
      remaining_requests: null,
      plan_status: null,
      expires_at: null,
      items: [],
      groups: [],
      compact_text: null,
      rate_limit_remaining_req: null,
      rate_limit_limit_req: null,
      rate_limit_remaining_tok: null,
      rate_limit_limit_tok: null,
      rate_limit_updated_at: null,
      total_input_tokens: null,
      total_output_tokens: null,
      total_cache_hit_tokens: null,
      total_cache_miss_tokens: null,
      total_requests_counted: null,
      total_estimated_cost: null,
      source: "deepseek_api",
      updated_at: updated,
      error: "Channel disabled",
    },
  ];
}

// ─── Virtual Keys ───────────────────────────────────────

export function buildVirtualKeys(now: Date): VirtualKey[] {
  const today = now.toISOString().slice(0, 10);
  const thisMonth = now.toISOString().slice(0, 7);
  return [
    {
      id: "vk-prod-001",
      name: "Production App",
      key_prefix: "msw_prod_",
      daily_budget_cents: 5000,
      monthly_budget_cents: 150_000,
      enabled: true,
      spend: {
        today: { date: today, cents: 1243 },
        this_month: { month: thisMonth, cents: 23_467 },
        total_cents: 182_934,
      },
      created_at: daysAgo(now, 60, 8),
      allowed_ips: [],
      allowed_models: null,
      denied_models: [],
    },
    {
      id: "vk-dev-002",
      name: "Dev Testing",
      key_prefix: "msw_dev_",
      daily_budget_cents: 500,
      monthly_budget_cents: 10_000,
      enabled: true,
      spend: {
        today: { date: today, cents: 87 },
        this_month: { month: thisMonth, cents: 2314 },
        total_cents: 8721,
      },
      created_at: daysAgo(now, 30, 4),
      allowed_ips: [],
      allowed_models: null,
      denied_models: [],
    },
    {
      id: "vk-leg-003",
      name: "Legacy Integration",
      key_prefix: "msw_leg_",
      daily_budget_cents: null,
      monthly_budget_cents: 5000,
      enabled: false,
      spend: {
        today: { date: today, cents: 0 },
        this_month: { month: thisMonth, cents: 321 },
        total_cents: 4567,
      },
      created_at: daysAgo(now, 90, 12),
      allowed_ips: [],
      allowed_models: null,
      denied_models: [],
    },
  ];
}

// ─── MCP Servers ────────────────────────────────────────

export function buildMcpServers(): McpServer[] {
  return [
    {
      id: "filesystem",
      name: "filesystem",
      command: "npx",
      args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
      env: {},
      cwd: null,
      enabled: true,
      expose_tools: true,
      status: { running: { tool_count: 5 } },
    },
    {
      id: "web-fetch",
      name: "web-fetch",
      command: "npx",
      args: ["-y", "@modelcontextprotocol/server-fetch"],
      env: {},
      cwd: null,
      enabled: false,
      expose_tools: true,
      status: "stopped",
    },
  ];
}

export function buildMcpTools(): McpToolDetail[] {
  return [
    { name: "read_file", description: "Read the complete contents of a file" },
    { name: "write_file", description: "Create a new file or overwrite an existing file" },
    { name: "list_directory", description: "Get a detailed listing of files and directories" },
    { name: "move_file", description: "Move or rename files and directories" },
    { name: "search_files", description: "Recursively search for files" },
  ];
}

export function buildMcpToolInfos(): McpToolInfo[] {
  return buildMcpTools().map((t) => ({
    server_id: "filesystem",
    name: t.name,
    description: t.description,
  }));
}

// ─── Cache Stats ────────────────────────────────────────

export function buildCacheStats(): CacheStats {
  return {
    entries: 342,
    mode: "lazy",
    hits: 4127,
    misses: 8720,
    evictions: 89,
    hit_rate_percent: 32.1,
    total_requests: 12847,
  };
}

// ─── Gateway Info ───────────────────────────────────────

export function buildGatewayInfo(): GatewayInfo {
  return {
    version: "0.4.0",
    uptime_seconds: 13370,
    uptime_formatted: "3h 42m",
    total_channels: 8,
    healthy_channels: 6,
    active_requests: 2,
    cache_entries: 342,
    routing_strategy: "WeightedRandom",
    max_retries: 3,
  };
}

// ─── Provider Budgets ───────────────────────────────────

export function buildProviderBudgets(now: Date): ProviderBudgetEntry[] {
  const today = now.toISOString().slice(0, 10);
  const thisMonth = now.toISOString().slice(0, 7);
  return [
    {
      provider: "openai",
      daily_budget_cents: 1000,
      monthly_budget_cents: 30_000,
      spend: {
        today: { date: today, cents: 421 },
        this_month: { month: thisMonth, cents: 12_340 },
        total_cents: 89_200,
      },
    },
    {
      provider: "anthropic",
      daily_budget_cents: 800,
      monthly_budget_cents: 25_000,
      spend: {
        today: { date: today, cents: 345 },
        this_month: { month: thisMonth, cents: 9_870 },
        total_cents: 67_500,
      },
    },
    {
      provider: "google",
      daily_budget_cents: 200,
      monthly_budget_cents: 5_000,
      spend: {
        today: { date: today, cents: 32 },
        this_month: { month: thisMonth, cents: 890 },
        total_cents: 3_200,
      },
    },
    {
      provider: "groq",
      daily_budget_cents: null,
      monthly_budget_cents: null,
      spend: {
        today: { date: today, cents: 0 },
        this_month: { month: thisMonth, cents: 0 },
        total_cents: 0,
      },
    },
  ];
}
