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
    return new Response("{}");
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
      return new Response("{}");
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
    list: async () => {
      await simDelay();
      return mockData.buildVirtualKeys(new Date());
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
        },
        plaintext: `${prefix}${uuid().replace(/-/g, "").slice(0, 12)}`,
      };
      return key;
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
      return new Response("{}");
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
};
