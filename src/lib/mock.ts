// Mock mode: when enabled, all api calls return realistic mock data.
// Zero changes needed in consumer code — api.ts augments itself.

import type { api } from "./api";
import type {
  Channel,
  UpdateChannelData,
  CreateMcpServerData,
  UpdateMcpServerData,
  CreateVirtualKeyData,
  CreateVirtualKeyResponse,
  UpdateVirtualKeyData,
  GuardrailsConfig,
  RedemptionCode,
  NotificationConfig,
  ProviderBudgetEntry,
  ListVirtualKeysParams,
  BatchCreateVirtualKeyData,
  BatchCreateVirtualKeyItem,
} from "./api";
import * as mockData from "./mock-data";

// ─── Helpers ────────────────────────────────────────────

function delay(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

/** Simulate network latency (150–400ms, non-deterministic). */
function simDelay(): Promise<void> {
  return delay(150 + Math.random() * 250);
}

function uuid(): string {
  if (typeof crypto !== "undefined" && crypto.randomUUID) {
    return crypto.randomUUID();
  }
  return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    const v = c === "x" ? r : (r & 0x3) | 0x8;
    return v.toString(16);
  });
}

// ─── Mock mode detection / toggle ───────────────────────

export function isMockMode(): boolean {
  try {
    const url = new URL(window.location.href);
    const mockParam = url.searchParams.get("mock");
    if (mockParam === "true" || mockParam === "1" || mockParam === "") return true;
  } catch {
    // SSR or non-browser env
  }
  try {
    return localStorage.getItem("msw-mock") === "1";
  } catch {
    return false;
  }
}

export function setMockMode(enabled: boolean): void {
  try {
    if (enabled) {
      localStorage.setItem("msw-mock", "1");
    } else {
      localStorage.removeItem("msw-mock");
    }
  } catch {
    // localStorage not available
  }
  window.location.reload();
}

// ─── Mock API ───────────────────────────────────────────

export const mockApi: typeof api = {
  listChannels: async () => {
    await simDelay();
    return mockData.buildChannels(new Date());
  },

  createChannel: async (input) => {
    await simDelay();
    const now = new Date();
    const iso = now.toISOString();
    const channel: Channel = {
      id: `ch-mock-${uuid()}`,
      name: input.name ?? "New Channel",
      provider: input.provider ?? "openai",
      priority: input.priority ?? 2,
      weight: input.weight ?? 50,
      cost_per_token: input.cost_per_token ?? null,
      input_cost_per_mtok: input.input_cost_per_mtok ?? null,
      output_cost_per_mtok: input.output_cost_per_mtok ?? null,
      enabled: input.enabled ?? true,
      status: "healthy",
      circuit_open_until: null,
      base_url: input.base_url ?? "https://api.openai.com/v1",
      model_mapping: input.model_mapping ?? {},
      created_at: iso,
      updated_at: iso,
      avg_latency_ms: 0,
      consecutive_failures: 0,
      cooldown_minutes: input.cooldown_minutes ?? null,
      rpm_limit: input.rpm_limit ?? null,
      tpm_limit: input.tpm_limit ?? null,
      tags: input.tags ?? [],
      account_group: input.account_group ?? null,
      max_concurrent: input.max_concurrent ?? null,
      excluded_models: input.excluded_models ?? [],
      api_keys: input.api_keys ?? [],
      proxy_url: input.proxy_url ?? null,
      headers: input.headers ?? {},
      max_retries: input.max_retries ?? null,
      models_endpoint: input.models_endpoint ?? null,
      models_refresh_interval_secs: input.models_refresh_interval_secs ?? 300,
    };
    return channel;
  },

  updateChannel: async (_id, input: UpdateChannelData) => {
    await simDelay();
    const now = new Date();
    const channels = mockData.buildChannels(now);
    const base = channels[0];
    return {
      ...base,
      name: input.name,
      provider: input.provider,
      priority: input.priority,
      weight: input.weight,
      enabled: input.enabled,
      base_url: input.base_url,
      model_mapping: input.model_mapping,
      cost_per_token: input.cost_per_token,
      input_cost_per_mtok: input.input_cost_per_mtok ?? null,
      output_cost_per_mtok: input.output_cost_per_mtok ?? null,
      cooldown_minutes: input.cooldown_minutes,
      rpm_limit: input.rpm_limit ?? null,
      tpm_limit: input.tpm_limit ?? null,
      updated_at: now.toISOString(),
    };
  },

  deleteChannel: async (_id) => {
    await simDelay();
  },

  pingChannel: async (_id) => {
    await simDelay();
    return { success: true, latency_ms: 42 };
  },

  channelStatus: async (id) => {
    await simDelay();
    const channels = mockData.buildChannels(new Date());
    const ch = channels.find((c) => c.id === id);
    return {
      status: ch?.status ?? "healthy",
      circuit_open_until: ch?.circuit_open_until ?? null,
    };
  },

  channelCooldown: async (_id: string) => {
    await simDelay();
    return {
      channel_id: _id,
      channel_name: "mock",
      in_cooldown: false,
      cooldown_remaining_secs: 0,
      circuit_open_until: null,
      model_cooldowns: {},
    };
  },

  logs: async (offset = 0, limit = 50) => {
    await simDelay();
    const all = mockData.buildLogs(new Date());
    return all.slice(offset, offset + limit);
  },

  stats: async () => {
    await simDelay();
    return mockData.buildStats();
  },

  costStats: async () => {
    await simDelay();
    return mockData.buildCostStats();
  },

  quota: async () => {
    await simDelay();
    return mockData.buildQuota(new Date());
  },

  usageHistory: async (_hours = 24) => {
    await simDelay();
    return mockData.buildUsageHistory(new Date());
  },

  mcp: {
    listServers: async () => {
      await simDelay();
      return mockData.buildMcpServers();
    },

    createServer: async (input: CreateMcpServerData) => {
      await simDelay();
      return {
        id: input.id,
        name: input.name,
        command: input.command,
        args: input.args ?? [],
        env: input.env ?? {},
        cwd: input.cwd ?? null,
        enabled: input.enabled ?? true,
        expose_tools: input.expose_tools ?? true,
        status: "stopped" as const,
      };
    },

    updateServer: async (id, input: UpdateMcpServerData) => {
      await simDelay();
      const servers = mockData.buildMcpServers();
      const base = servers.find((s) => s.id === id) ?? servers[0];
      return {
        ...base,
        name: input.name,
        command: input.command,
        args: input.args ?? base.args,
        env: input.env ?? base.env,
        cwd: input.cwd ?? base.cwd,
        enabled: input.enabled ?? base.enabled,
        expose_tools: input.expose_tools ?? base.expose_tools,
      };
    },

    deleteServer: async (_id) => {
      await simDelay();
    },

    startServer: async (_id) => {
      await simDelay();
      return { ok: true };
    },

    stopServer: async (_id) => {
      await simDelay();
      return { ok: true };
    },

    listServerTools: async (id) => {
      await simDelay();
      if (id === "filesystem") return mockData.buildMcpTools();
      return [];
    },

    listAllTools: async () => {
      await simDelay();
      return mockData.buildMcpToolInfos();
    },
  },

  virtualKeys: {
    list: async (params?: ListVirtualKeysParams) => {
      await simDelay();
      const page = params?.page ?? 1;
      const limit = params?.limit ?? 50;
      const search = params?.search?.trim().toLowerCase() ?? "";
      const all = mockData.buildVirtualKeys(new Date());
      const filtered = search
        ? all.filter(
            (k) =>
              k.name.toLowerCase().includes(search) ||
              k.key_prefix.toLowerCase().includes(search),
          )
        : all;
      const start = (page - 1) * limit;
      const data = filtered.slice(start, start + limit);
      return {
        data,
        total: filtered.length,
        page,
        limit,
      };
    },

    create: async (input: CreateVirtualKeyData) => {
      await simDelay();
      const now = new Date();
      const today = now.toISOString().slice(0, 10);
      const thisMonth = now.toISOString().slice(0, 7);
      const prefix = "msw_mock_";
      const key: CreateVirtualKeyResponse = {
        key: {
          id: `vk-mock-${uuid()}`,
          name: input.name,
          key_prefix: prefix,
          daily_budget_cents: input.daily_budget_cents ?? null,
          monthly_budget_cents: input.monthly_budget_cents ?? null,
          enabled: true,
          spend: {
            today: { date: today, cents: 0 },
            this_month: { month: thisMonth, cents: 0 },
            total_cents: 0,
          },
          created_at: now.toISOString(),
          allowed_ips: input.allowed_ips ?? [],
          allowed_models: input.allowed_models ?? null,
          denied_models: input.denied_models ?? [],
        },
        plaintext: `${prefix}${uuid().replace(/-/g, "").slice(0, 12)}`,
      };
      return key;
    },

    batchCreate: async (input: BatchCreateVirtualKeyData): Promise<BatchCreateVirtualKeyItem[]> => {
      await simDelay();
      const now = new Date();
      const today = now.toISOString().slice(0, 10);
      const thisMonth = now.toISOString().slice(0, 7);
      const prefix = "msw_mock_";
      const items: BatchCreateVirtualKeyItem[] = [];
      for (let i = 0; i < input.count; i++) {
        items.push({
          id: `vk-mock-${uuid()}`,
          name: `${input.name_prefix}${i + 1}`,
          key: `${prefix}${uuid().replace(/-/g, "").slice(0, 12)}`,
          key_prefix: prefix,
          daily_budget_cents: input.daily_budget_cents ?? null,
          monthly_budget_cents: input.monthly_budget_cents ?? null,
          enabled: true,
          created_at: now.toISOString(),
          spend: {
            today: { date: today, cents: 0 },
            this_month: { month: thisMonth, cents: 0 },
            total_cents: 0,
          },
          allowed_models: input.allowed_models ?? null,
          denied_models: [],
          allowed_ips: input.allowed_ips ?? [],
        });
      }
      return items;
    },

    update: async (id, input: UpdateVirtualKeyData) => {
      await simDelay();
      const keys = mockData.buildVirtualKeys(new Date());
      const base = keys.find((k) => k.id === id) ?? keys[0];
      return {
        ...base,
        name: input.name ?? base.name,
        daily_budget_cents: input.daily_budget_cents ?? base.daily_budget_cents,
        monthly_budget_cents: input.monthly_budget_cents ?? base.monthly_budget_cents,
        enabled: input.enabled ?? base.enabled,
      };
    },

    delete: async (_id) => {
      await simDelay();
    },
  },

  cacheStats: async () => {
    await simDelay();
    return mockData.buildCacheStats();
  },

  flushCache: async () => {
    await simDelay();
    return { flushed: true, remaining: 0 };
  },

  reloadConfig: async () => {
    await simDelay();
    return { created: 0, updated: 1, removed: 0 };
  },

  gatewayInfo: async () => {
    await simDelay();
    return mockData.buildGatewayInfo();
  },

  providerBudgets: async () => {
    await simDelay();
    return mockData.buildProviderBudgets(new Date());
  },

  setProviderBudget: async (
    provider: string,
    budget: { daily_budget_cents?: number | null; monthly_budget_cents?: number | null },
  ): Promise<ProviderBudgetEntry> => {
    await simDelay();
    const now = new Date();
    const today = now.toISOString().slice(0, 10);
    const thisMonth = now.toISOString().slice(0, 7);
    return {
      provider,
      daily_budget_cents: budget.daily_budget_cents ?? null,
      monthly_budget_cents: budget.monthly_budget_cents ?? null,
      spend: {
        today: { date: today, cents: 0 },
        this_month: { month: thisMonth, cents: 0 },
        total_cents: 0,
      },
    };
  },

  deleteProviderBudget: async (_provider: string) => {
    await simDelay();
  },

  // Batch channel operations
  batchEnableChannels: async (ids: string[]) => {
    await simDelay();
    return { updated: ids.length };
  },
  batchDisableChannels: async (ids: string[]) => {
    await simDelay();
    return { updated: ids.length };
  },
  batchDeleteChannels: async (ids: string[]) => {
    await simDelay();
    return { deleted: ids.length };
  },
  batchUpdateTags: async (ids: string[], _tags: string[]) => {
    await simDelay();
    return { updated: ids.length };
  },

  // Reset circuit breaker
  resetCircuit: async (_id: string) => {
    await simDelay();
    return { ok: true };
  },

  // Audit log
  auditLog: async (_limit = 100) => {
    await simDelay();
    return [];
  },

  // Metrics (Prometheus text format)
  metrics: async () => {
    await simDelay();
    return "# HELP modelswitch_requests_total Total requests\n# TYPE modelswitch_requests_total counter\nmodelswitch_requests_total 0\n";
  },

  // Guardrails config
  guardrailsConfig: async (): Promise<GuardrailsConfig> => {
    await simDelay();
    return {
      enabled: false,
      blocked_patterns: [],
      allowed_patterns: [],
      max_request_chars: null,
      block_message: "Request blocked by guardrails",
    };
  },
  updateGuardrails: async (config: Partial<GuardrailsConfig>): Promise<GuardrailsConfig> => {
    await simDelay();
    return {
      enabled: config.enabled ?? false,
      blocked_patterns: config.blocked_patterns ?? [],
      allowed_patterns: config.allowed_patterns ?? [],
      max_request_chars: config.max_request_chars ?? null,
      block_message: config.block_message ?? "Request blocked by guardrails",
    };
  },

  // Redemption codes
  redemptionCodes: {
    list: async (): Promise<RedemptionCode[]> => {
      await simDelay();
      return [];
    },
    create: async (data: { credits_cents: number; expires_at?: string | null }): Promise<RedemptionCode> => {
      await simDelay();
      return {
        code: `RC-${uuid().slice(0, 8).toUpperCase()}`,
        credits_cents: data.credits_cents,
        used: false,
        used_by: null,
        used_at: null,
        created_at: new Date().toISOString(),
        expires_at: data.expires_at ?? null,
      };
    },
    redeem: async (_code: string, _userId?: string) => {
      await simDelay();
      return { credits_cents: 1000 };
    },
    delete: async (_code: string) => {
      await simDelay();
    },
  },

  // Notification config
  notificationConfig: async (): Promise<NotificationConfig> => {
    await simDelay();
    return {
      webhook_url: null,
      webhook_secret: null,
      bark_url: null,
      budget_threshold_pct: 80,
      smtp_enabled: false,
      smtp_host: null,
      smtp_port: null,
      smtp_username: null,
      smtp_password: null,
      smtp_from: null,
      smtp_admin_email: null,
      smtp_use_tls: false,
    };
  },
  updateNotification: async (config: Partial<NotificationConfig>): Promise<NotificationConfig> => {
    await simDelay();
    return {
      webhook_url: config.webhook_url ?? null,
      webhook_secret: config.webhook_secret ?? null,
      bark_url: config.bark_url ?? null,
      budget_threshold_pct: config.budget_threshold_pct ?? 80,
      smtp_enabled: config.smtp_enabled ?? false,
      smtp_host: config.smtp_host ?? null,
      smtp_port: config.smtp_port ?? null,
      smtp_username: config.smtp_username ?? null,
      smtp_password: config.smtp_password ?? null,
      smtp_from: config.smtp_from ?? null,
      smtp_admin_email: config.smtp_admin_email ?? null,
      smtp_use_tls: config.smtp_use_tls ?? false,
    };
  },

  // Channel auto-test
  testChannel: async (_id: string) => {
    await simDelay();
    return { healthy: true, latency_ms: 42, error: null };
  },
  testAllChannels: async () => {
    await simDelay();
    return [];
  },

  // MCP health
  mcpHealth: async () => {
    await simDelay();
    const servers = mockData.buildMcpServers();
    const now = new Date().toISOString();
    return servers.map((s, idx) => ({
      name: s.name,
      healthy: idx % 3 !== 2,
      last_check: now,
      last_error: idx % 3 === 2 ? "connection refused" : null,
      consecutive_failures: idx % 3 === 2 ? 2 : 0,
    }));
  },

  // Completion ratios
  completionRatios: async () => {
    await simDelay();
    return {};
  },
  updateCompletionRatios: async (ratios: Record<string, number>) => {
    await simDelay();
    return ratios;
  },

  // Model registry inspection
  modelRegistry: async () => {
    await simDelay();
    const now = Math.floor(Date.now() / 1000);
    return {
      models: [
        {
          name: "gpt-4o",
          source_type: "builtin" as const,
          source: null,
          channel_id: null,
          channel_name: null,
          last_refreshed_secs: null,
          supports_thinking: false,
          supports_vision: true,
          supports_tools: true,
          max_context_tokens: 128000,
          thinking_format: "Level" as const,
        },
        {
          name: "gpt-4o-mini",
          source_type: "builtin" as const,
          source: null,
          channel_id: null,
          channel_name: null,
          last_refreshed_secs: null,
          supports_thinking: false,
          supports_vision: true,
          supports_tools: true,
          max_context_tokens: 128000,
          thinking_format: "Level" as const,
        },
        {
          name: "claude-sonnet-4",
          source_type: "builtin" as const,
          source: null,
          channel_id: null,
          channel_name: null,
          last_refreshed_secs: null,
          supports_thinking: true,
          supports_vision: true,
          supports_tools: true,
          max_context_tokens: 200000,
          thinking_format: "Budget" as const,
        },
        {
          name: "deepseek-chat",
          source_type: "discovered" as const,
          source: "https://api.deepseek.com/v1/models",
          channel_id: "ch-0001",
          channel_name: "DeepSeek Primary",
          last_refreshed_secs: now - 120,
          supports_thinking: false,
          supports_vision: false,
          supports_tools: true,
          max_context_tokens: 64000,
          thinking_format: "None" as const,
        },
        {
          name: "deepseek-reasoner",
          source_type: "discovered" as const,
          source: "https://api.deepseek.com/v1/models",
          channel_id: "ch-0001",
          channel_name: "DeepSeek Primary",
          last_refreshed_secs: now - 120,
          supports_thinking: true,
          supports_vision: false,
          supports_tools: false,
          max_context_tokens: 64000,
          thinking_format: "Budget" as const,
        },
        {
          name: "gemini-2.5-pro",
          source_type: "builtin" as const,
          source: null,
          channel_id: null,
          channel_name: null,
          last_refreshed_secs: null,
          supports_thinking: true,
          supports_vision: true,
          supports_tools: true,
          max_context_tokens: 1000000,
          thinking_format: "Budget" as const,
        },
      ],
      total: 6,
    };
  },

  // Per-channel payload rules (runtime override)
  getPayloadRules: async (_channelId: string) => {
    await simDelay();
    return {};
  },
  updatePayloadRules: async (channelId: string) => {
    await simDelay();
    return { channel_id: channelId, updated: true };
  },
};
