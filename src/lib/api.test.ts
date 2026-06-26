import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// Create the fetch mock and assign it before importing api so that api's
// module-level code sees our spy.
const mockFetch = vi.fn();

// Provide a concrete localStorage mock. Node 22+ ships an experimental
// bare `localStorage` that is undefined by default and shadows jsdom's.
// We replace it with a fully-functional in-memory implementation.
function createStorage() {
  const store = new Map<string, string>();
  return {
    getItem: vi.fn((key: string) => store.get(key) ?? null),
    setItem: vi.fn((key: string, value: string) => {
      store.set(key, String(value));
    }),
    removeItem: vi.fn((key: string) => {
      store.delete(key);
    }),
    clear: vi.fn(() => {
      store.clear();
    }),
    key: vi.fn((index: number) => Array.from(store.keys())[index] ?? null),
    get length() {
      return store.size;
    },
  };
}

const localStorageMock = createStorage();
const sessionStorageMock = createStorage();

vi.stubGlobal("localStorage", localStorageMock);
vi.stubGlobal("sessionStorage", sessionStorageMock);
vi.stubGlobal("fetch", mockFetch);

// jsdom's window.location.reload() can reset internal state; stub it to a no-op
// so our storage mocks survive the 401 handler.
vi.stubGlobal("location", {
  ...window.location,
  reload: () => {},
});

import { api, request, validateChannelForm } from "./api";

function jsonResponse(body: unknown, init?: { ok?: boolean; status?: number }) {
  return {
    ok: init?.ok ?? true,
    status: init?.status ?? 200,
    json: async () => body,
  };
}

describe("API client", () => {
  beforeEach(() => {
    mockFetch.mockReset();
    // Reset storage state and mock call tracking.
    localStorageMock.clear();
    sessionStorageMock.clear();
    localStorageMock.getItem.mockClear();
    localStorageMock.setItem.mockClear();
    localStorageMock.removeItem.mockClear();
    sessionStorageMock.getItem.mockClear();
    sessionStorageMock.setItem.mockClear();
    // Default successful envelope response
    mockFetch.mockResolvedValue(
      jsonResponse({ ok: true, data: [] }),
    );
  });

  afterEach(() => {
    localStorageMock.clear();
    sessionStorageMock.clear();
  });

  // ─── request() unit tests ───────────────────────────────

  describe("request()", () => {
    it("sends Content-Type application/json by default", async () => {
      await request("/api/stats");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect((opts.headers as Record<string, string>)["Content-Type"]).toBe(
        "application/json",
      );
    });

    it("omits Authorization header when no admin_token in localStorage", async () => {
      await request("/api/stats");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect((opts.headers as Record<string, string>)["Authorization"]).toBeUndefined();
    });

    it("sends Authorization Bearer header when admin_token is set", async () => {
      localStorage.setItem("admin_token", "test-secret-token");
      await request("/api/stats");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect((opts.headers as Record<string, string>)["Authorization"]).toBe(
        "Bearer test-secret-token",
      );
    });

    it("prepends API_BASE to the path", async () => {
      await request("/api/stats");
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/stats");
    });

    it("unwraps { ok, data } envelope and returns data", async () => {
      mockFetch.mockResolvedValue(
        jsonResponse({ ok: true, data: { name: "result" } }),
      );
      const result = await request<{ name: string }>("/api/test");
      expect(result).toEqual({ name: "result" });
    });

    it("throws when envelope has ok: false", async () => {
      // The envelope check requires both "ok" and "data" keys to be present.
      mockFetch.mockResolvedValue(
        jsonResponse({
          ok: false,
          data: null,
          error: { message: "Custom failure" },
        }),
      );
      await expect(request("/api/test")).rejects.toThrow("Custom failure");
    });

    it("falls back to 'Unknown API error' when ok:false without message", async () => {
      // The envelope check requires both "ok" and "data" keys to be present.
      mockFetch.mockResolvedValue(jsonResponse({ ok: false, data: null }));
      await expect(request("/api/test")).rejects.toThrow("Unknown API error");
    });

    it("returns raw json when envelope shape is absent", async () => {
      mockFetch.mockResolvedValue(jsonResponse({ notAnEnvelope: true }));
      const result = await request("/api/test");
      expect(result).toEqual({ notAnEnvelope: true });
    });

    it("throws 'Unauthorized' on 401 and clears token", async () => {
      localStorage.setItem("admin_token", "will-be-cleared");
      mockFetch.mockResolvedValue(
        jsonResponse({ ok: false, error: { message: "Unauthorized" } }, {
          ok: false,
          status: 401,
        }),
      );
      await expect(request("/api/test")).rejects.toThrow("Unauthorized");
      expect(localStorage.getItem("admin_token")).toBeNull();
      // Verify sessionStorage.setItem was called for auth-expiry flag.
      expect(sessionStorageMock.setItem).toHaveBeenCalledWith("auth_expired", "1");
    });

    it("throws on non-401 error status (e.g., 500)", async () => {
      mockFetch.mockResolvedValue(
        jsonResponse({}, { ok: false, status: 500 }),
      );
      await expect(request("/api/test")).rejects.toThrow("API error: 500");
    });

    it("throws on 429 rate limited status", async () => {
      mockFetch.mockResolvedValue(
        jsonResponse({}, { ok: false, status: 429 }),
      );
      await expect(request("/api/test")).rejects.toThrow("API error: 429");
    });

    it("propagates network errors from fetch", async () => {
      mockFetch.mockRejectedValue(new Error("Network error"));
      await expect(request("/api/test")).rejects.toThrow("Network error");
    });
  });

  // ─── api.listChannels ───────────────────────────────────

  describe("api.listChannels()", () => {
    it("calls GET /api/channels and returns the data array", async () => {
      const channels = [{ id: "c1", name: "OpenAI" }];
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: channels }));
      const result = await api.listChannels();
      expect(result).toEqual(channels);
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/channels");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBeUndefined(); // default GET
    });

    it("includes Authorization header when token is present", async () => {
      localStorage.setItem("admin_token", "abc123");
      await api.listChannels();
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect((opts.headers as Record<string, string>)["Authorization"]).toBe(
        "Bearer abc123",
      );
    });

    it("rejects on network error", async () => {
      mockFetch.mockRejectedValue(new Error("Network error"));
      await expect(api.listChannels()).rejects.toThrow("Network error");
    });

    it("rejects on 401", async () => {
      mockFetch.mockResolvedValue(
        jsonResponse({}, { ok: false, status: 401 }),
      );
      await expect(api.listChannels()).rejects.toThrow("Unauthorized");
    });

    it("rejects on 429", async () => {
      mockFetch.mockResolvedValue(
        jsonResponse({}, { ok: false, status: 429 }),
      );
      await expect(api.listChannels()).rejects.toThrow("API error: 429");
    });
  });

  // ─── api.virtualKeys ────────────────────────────────────

  describe("api.virtualKeys.list()", () => {
    it("passes page and limit as query parameters", async () => {
      mockFetch.mockResolvedValue(
        jsonResponse({ ok: true, data: { data: [], total: 0, page: 2, limit: 50 } }),
      );
      await api.virtualKeys.list({ page: 2, limit: 50 });
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("page=2");
      expect(url).toContain("limit=50");
      expect(url).toContain("/api/virtual-keys");
    });

    it("passes search and group parameters", async () => {
      mockFetch.mockResolvedValue(
        jsonResponse({ ok: true, data: { data: [], total: 0, page: 1, limit: 20 } }),
      );
      await api.virtualKeys.list({ search: "alice", group: "eng" });
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("search=alice");
      expect(url).toContain("group=eng");
    });

    it("omits query string when no params provided", async () => {
      mockFetch.mockResolvedValue(
        jsonResponse({ ok: true, data: { data: [], total: 0, page: 1, limit: 20 } }),
      );
      await api.virtualKeys.list();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).not.toContain("?");
      expect(url).toContain("/api/virtual-keys");
    });
  });

  describe("api.virtualKeys.create()", () => {
    it("sends POST to /api/virtual-keys with JSON body", async () => {
      const mockResponse = {
        ok: true,
        data: {
          key: { id: "k1", name: "test-key" },
          plaintext: "vk_secret",
        },
      };
      mockFetch.mockResolvedValue(jsonResponse(mockResponse));

      const payload = {
        name: "test-key",
        daily_budget_cents: 100,
        group: "eng",
      };
      const result = await api.virtualKeys.create(payload);

      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/virtual-keys");
      expect(url).not.toContain("?");

      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("POST");
      expect(JSON.parse(opts.body as string)).toEqual(payload);

      expect(result).toEqual(mockResponse.data);
    });
  });

  describe("api.virtualKeys.delete()", () => {
    it("sends DELETE to /api/virtual-keys/:id", async () => {
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: null }));
      await api.virtualKeys.delete("vk_123");
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/virtual-keys/vk_123");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("DELETE");
    });
  });

  // ─── api.stats ──────────────────────────────────────────

  describe("api.stats()", () => {
    it("calls GET /api/stats", async () => {
      const stats = { total_requests: 100, successes: 90, failures: 10, avg_latency_ms: 250 };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: stats }));
      const result = await api.stats();
      expect(result).toEqual(stats);
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/stats");
    });
  });

  // ─── Channel CRUD ───────────────────────────────────────

  describe("api.createChannel()", () => {
    it("sends POST to /api/channels with JSON body and returns Channel", async () => {
      const mockChannel = {
        id: "ch-1",
        name: "Test",
        provider: "openai",
        priority: 1,
        weight: 1,
        cost_per_token: null,
        input_cost_per_mtok: null,
        output_cost_per_mtok: null,
        enabled: true,
        status: "healthy" as const,
        circuit_open_until: null,
        base_url: "https://api.openai.com",
        model_mapping: {},
        created_at: "2025-01-01",
        updated_at: "2025-01-01",
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
        models_refresh_interval_secs: 3600,
      };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: mockChannel }));

      const payload = {
        name: "Test",
        provider: "openai",
        priority: 1,
        weight: 1,
        base_url: "https://api.openai.com",
        enabled: true,
        model_mapping: {},
        credential_value: "sk-test",
      };
      const result = await api.createChannel(payload);

      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/channels");
      expect(url).not.toContain("?");

      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("POST");
      const body = JSON.parse(opts.body as string);
      expect(body.name).toBe("Test");
      expect(body.credential_value).toBe("sk-test");

      expect(result).toEqual(mockChannel);
    });
  });

  describe("api.updateChannel()", () => {
    it("sends PUT to /api/channels/:id with JSON body", async () => {
      const mockChannel = { id: "ch-9", name: "Updated" };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: mockChannel }));

      const update = {
        name: "Updated",
        provider: "openai",
        priority: 2,
        weight: 1,
        cost_per_token: null,
        base_url: "https://api.openai.com",
        enabled: true,
        model_mapping: {},
        cooldown_minutes: 5,
      };
      const result = await api.updateChannel("ch-9", update);

      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/channels/ch-9");

      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("PUT");
      const body = JSON.parse(opts.body as string);
      expect(body.name).toBe("Updated");
      expect(body.priority).toBe(2);

      expect(result).toEqual(mockChannel);
    });
  });

  describe("api.deleteChannel()", () => {
    it("sends DELETE to /api/channels/:id", async () => {
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: null }));
      await api.deleteChannel("ch-3");
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/channels/ch-3");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("DELETE");
    });
  });

  describe("api.pingChannel()", () => {
    it("sends POST to /api/channels/:id/ping and returns result", async () => {
      const pingResult = { success: true, latency_ms: 42 };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: pingResult }));
      const result = await api.pingChannel("ch-7");
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/channels/ch-7/ping");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("POST");
      expect(result).toEqual(pingResult);
    });
  });

  describe("api.channelStatus()", () => {
    it("sends GET to /api/channels/:id/status", async () => {
      const status = { status: "healthy", circuit_open_until: null };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: status }));
      const result = await api.channelStatus("ch-2");
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/channels/ch-2/status");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBeUndefined();
      expect(result).toEqual(status);
    });
  });

  describe("api.channelCooldown()", () => {
    it("sends GET to /api/channels/:id/cooldown", async () => {
      const cooldown = {
        channel_id: "ch-5",
        channel_name: "OpenAI",
        in_cooldown: false,
        cooldown_remaining_secs: 0,
        circuit_open_until: null,
        model_cooldowns: {},
      };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: cooldown }));
      const result = await api.channelCooldown("ch-5");
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/channels/ch-5/cooldown");
      expect(result).toEqual(cooldown);
    });
  });

  // ─── Stats & data methods ───────────────────────────────

  describe("api.costStats()", () => {
    it("calls GET /api/stats/cost and returns CostStats", async () => {
      const costStats = {
        total_requests: 500,
        total_estimated_cost: 12.5,
        priority_breakdown: [{ priority: 1, requests: 400, estimated_cost: 10 }],
        model_counts: { "gpt-4": 300, "gpt-3.5": 200 },
        total_input_tokens: 100000,
        total_output_tokens: 50000,
      };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: costStats }));
      const result = await api.costStats();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/stats/cost");
      expect(result).toEqual(costStats);
    });
  });

  describe("api.usageHistory()", () => {
    it("calls GET /api/stats/usage with hours param", async () => {
      const history = {
        buckets: [],
        total_input_tokens: 0,
        total_output_tokens: 0,
        total_requests: 0,
        total_cost: 0,
      };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: history }));
      const result = await api.usageHistory(48);
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/stats/usage");
      expect(url).toContain("hours=48");
      expect(result).toEqual(history);
    });

    it("defaults to 24 hours", async () => {
      const history = {
        buckets: [],
        total_input_tokens: 0,
        total_output_tokens: 0,
        total_requests: 0,
        total_cost: 0,
      };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: history }));
      await api.usageHistory();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("hours=24");
    });
  });

  describe("api.quota()", () => {
    it("calls GET /api/quota and returns QuotaInfo[]", async () => {
      const quotas = [
        { channel_id: "ch-1", channel_name: "OpenAI", provider: "openai" },
      ];
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: quotas }));
      const result = await api.quota();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/quota");
      expect(result).toEqual(quotas);
    });
  });

  describe("api.logs()", () => {
    it("calls GET /api/logs with offset and limit params", async () => {
      const logs = [
        { id: "l1", timestamp: "2025-01-01", request_model: "gpt-4" },
      ];
      mockFetch.mockResolvedValue(
        jsonResponse({ ok: true, data: logs, total: 1, offset: 10, limit: 5 }),
      );
      const result = await api.logs(10, 5);
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/logs");
      expect(url).toContain("offset=10");
      expect(url).toContain("limit=5");
      expect(result).toEqual(logs);
    });

    it("defaults to offset=0 and limit=50", async () => {
      mockFetch.mockResolvedValue(
        jsonResponse({ ok: true, data: [], total: 0, offset: 0, limit: 50 }),
      );
      await api.logs();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("offset=0");
      expect(url).toContain("limit=50");
    });

    it("rejects on 401 and clears token", async () => {
      localStorage.setItem("admin_token", "will-be-cleared");
      mockFetch.mockResolvedValue(
        jsonResponse({}, { ok: false, status: 401 }),
      );
      await expect(api.logs()).rejects.toThrow("Unauthorized");
      expect(localStorage.getItem("admin_token")).toBeNull();
    });
  });

  // ─── VirtualKey lifecycle ───────────────────────────────

  describe("api.virtualKeys.update()", () => {
    it("sends PUT to /api/virtual-keys/:id with body", async () => {
      const mockKey = { id: "vk-1", name: "Updated Key" };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: mockKey }));
      const update = { name: "Updated Key", enabled: false };
      const result = await api.virtualKeys.update("vk-1", update);
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/virtual-keys/vk-1");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("PUT");
      const body = JSON.parse(opts.body as string);
      expect(body.name).toBe("Updated Key");
      expect(body.enabled).toBe(false);
      expect(result).toEqual(mockKey);
    });
  });

  describe("api.virtualKeys.batchCreate()", () => {
    it("sends POST to /api/virtual-keys/batch with body", async () => {
      const mockItems = [
        { id: "vk-1", name: "batch-1", key: "vk_s1", key_prefix: "vk_s" },
        { id: "vk-2", name: "batch-2", key: "vk_s2", key_prefix: "vk_s" },
      ];
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: mockItems }));
      const payload = {
        count: 2,
        name_prefix: "batch",
        daily_budget_cents: 100,
      };
      const result = await api.virtualKeys.batchCreate(payload);
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/virtual-keys/batch");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("POST");
      const body = JSON.parse(opts.body as string);
      expect(body.count).toBe(2);
      expect(body.name_prefix).toBe("batch");
      expect(result).toEqual(mockItems);
    });
  });

  describe("api.virtualKeys.groups()", () => {
    it("calls GET /api/virtual-keys/groups", async () => {
      const groups = ["eng", "marketing"];
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: groups }));
      const result = await api.virtualKeys.groups();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/virtual-keys/groups");
      expect(result).toEqual(groups);
    });
  });

  // ─── validateChannelForm (pure function) ────────────────

  describe("validateChannelForm()", () => {
    it("returns error key when name is empty", () => {
      expect(validateChannelForm({ name: "", baseUrl: "https://api.openai.com" })).toBe(
        "channels.nameRequired",
      );
    });

    it("returns error key when name is whitespace only", () => {
      expect(validateChannelForm({ name: "   ", baseUrl: "https://api.openai.com" })).toBe(
        "channels.nameRequired",
      );
    });

    it("returns error key when baseUrl is empty", () => {
      expect(validateChannelForm({ name: "OpenAI", baseUrl: "" })).toBe(
        "channels.baseUrlRequired",
      );
    });

    it("returns error key when baseUrl is whitespace only", () => {
      expect(validateChannelForm({ name: "OpenAI", baseUrl: "   " })).toBe(
        "channels.baseUrlRequired",
      );
    });

    it("returns error key when baseUrl lacks http(s) protocol", () => {
      expect(validateChannelForm({ name: "OpenAI", baseUrl: "api.openai.com" })).toBe(
        "channels.baseUrlInvalid",
      );
    });

    it("returns error key when baseUrl uses ftp protocol", () => {
      expect(
        validateChannelForm({ name: "OpenAI", baseUrl: "ftp://api.openai.com" }),
      ).toBe("channels.baseUrlInvalid");
    });

    it("returns null for valid http URL", () => {
      expect(
        validateChannelForm({ name: "Local", baseUrl: "http://localhost:8080" }),
      ).toBeNull();
    });

    it("returns null for valid https URL", () => {
      expect(
        validateChannelForm({ name: "OpenAI", baseUrl: "https://api.openai.com" }),
      ).toBeNull();
    });
  });

  // ─── Provider budgets ───────────────────────────────────

  describe("api.providerBudgets()", () => {
    it("calls GET /api/provider-budgets", async () => {
      const budgets = [
        {
          provider: "openai",
          daily_budget_cents: 500,
          monthly_budget_cents: 10000,
          spend: {
            today: { date: "2025-01-01", cents: 100 },
            this_month: { month: "2025-01", cents: 1000 },
            total_cents: 5000,
          },
        },
      ];
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: budgets }));
      const result = await api.providerBudgets();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/provider-budgets");
      expect(result).toEqual(budgets);
    });
  });

  describe("api.setProviderBudget()", () => {
    it("sends PUT to /api/provider-budgets/:provider with body", async () => {
      const mockBudget = {
        provider: "anthropic",
        daily_budget_cents: 300,
        monthly_budget_cents: 6000,
        spend: {
          today: { date: "2025-01-01", cents: 0 },
          this_month: { month: "2025-01", cents: 0 },
          total_cents: 0,
        },
      };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: mockBudget }));
      const result = await api.setProviderBudget("anthropic", {
        daily_budget_cents: 300,
        monthly_budget_cents: 6000,
      });
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/provider-budgets/anthropic");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("PUT");
      const body = JSON.parse(opts.body as string);
      expect(body.daily_budget_cents).toBe(300);
      expect(result).toEqual(mockBudget);
    });

    it("URL-encodes provider names with special characters", async () => {
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: null }));
      await api.setProviderBudget("azure east/us", { daily_budget_cents: 100 });
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/provider-budgets/azure%20east%2Fus");
    });
  });

  describe("api.deleteProviderBudget()", () => {
    it("sends DELETE to /api/provider-budgets/:provider", async () => {
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: null }));
      await api.deleteProviderBudget("openai");
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/provider-budgets/openai");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("DELETE");
    });

    it("URL-encodes provider names", async () => {
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: null }));
      await api.deleteProviderBudget("my provider");
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/provider-budgets/my%20provider");
    });
  });

  // ─── Batch channel operations ───────────────────────────

  describe("api.batchEnableChannels()", () => {
    it("sends POST to /api/channels/batch/enable with ids", async () => {
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: { updated: 3 } }));
      const result = await api.batchEnableChannels(["ch-1", "ch-2", "ch-3"]);
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/channels/batch/enable");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("POST");
      const body = JSON.parse(opts.body as string);
      expect(body.ids).toEqual(["ch-1", "ch-2", "ch-3"]);
      expect(result).toEqual({ updated: 3 });
    });
  });

  describe("api.batchDisableChannels()", () => {
    it("sends POST to /api/channels/batch/disable with ids", async () => {
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: { updated: 2 } }));
      const result = await api.batchDisableChannels(["ch-1", "ch-2"]);
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/channels/batch/disable");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("POST");
      const body = JSON.parse(opts.body as string);
      expect(body.ids).toEqual(["ch-1", "ch-2"]);
      expect(result).toEqual({ updated: 2 });
    });
  });

  describe("api.batchDeleteChannels()", () => {
    it("sends POST to /api/channels/batch/delete with ids", async () => {
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: { deleted: 1 } }));
      const result = await api.batchDeleteChannels(["ch-1"]);
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/channels/batch/delete");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("POST");
      expect(result).toEqual({ deleted: 1 });
    });
  });

  describe("api.batchUpdateTags()", () => {
    it("sends PUT to /api/channels/batch/tags with ids and add_tags", async () => {
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: { updated: 2 } }));
      const result = await api.batchUpdateTags(["ch-1", "ch-2"], ["prod", "fast"]);
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/channels/batch/tags");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("PUT");
      const body = JSON.parse(opts.body as string);
      expect(body.ids).toEqual(["ch-1", "ch-2"]);
      expect(body.add_tags).toEqual(["prod", "fast"]);
      expect(result).toEqual({ updated: 2 });
    });
  });

  describe("api.resetCircuit()", () => {
    it("sends POST to /api/channels/:id/reset-circuit", async () => {
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: { ok: true } }));
      const result = await api.resetCircuit("ch-4");
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/channels/ch-4/reset-circuit");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("POST");
      expect(result).toEqual({ ok: true });
    });
  });

  // ─── Cache, config, gateway ─────────────────────────────

  describe("api.cacheStats()", () => {
    it("calls GET /api/cache/stats", async () => {
      const cacheStats = {
        entries: 10,
        mode: "memory",
        hits: 50,
        misses: 5,
        evictions: 1,
        hit_rate_percent: 90.9,
        total_requests: 55,
      };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: cacheStats }));
      const result = await api.cacheStats();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/cache/stats");
      expect(result).toEqual(cacheStats);
    });
  });

  describe("api.flushCache()", () => {
    it("sends POST to /api/cache/flush", async () => {
      const flushResult = { flushed: true, remaining: 0 };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: flushResult }));
      const result = await api.flushCache();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/cache/flush");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("POST");
      expect(result).toEqual(flushResult);
    });
  });

  describe("api.reloadConfig()", () => {
    it("sends POST to /api/config/reload", async () => {
      const reloadResult = { created: 1, updated: 3, removed: 0 };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: reloadResult }));
      const result = await api.reloadConfig();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/config/reload");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("POST");
      expect(result).toEqual(reloadResult);
    });
  });

  describe("api.gatewayInfo()", () => {
    it("calls GET /api/gateway/info", async () => {
      const info = {
        version: "1.0.0",
        uptime_seconds: 3600,
        uptime_formatted: "1h",
        total_channels: 5,
        healthy_channels: 4,
        active_requests: 2,
        cache_entries: 10,
        routing_strategy: "priority",
        max_retries: 3,
      };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: info }));
      const result = await api.gatewayInfo();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/gateway/info");
      expect(result).toEqual(info);
    });
  });

  // ─── Audit log & metrics ────────────────────────────────

  describe("api.auditLog()", () => {
    it("calls GET /api/audit-log with limit param", async () => {
      const entries = [
        { timestamp: "2025-01-01", action: "create", actor: "admin", target: "ch-1", details: null },
      ];
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: entries }));
      const result = await api.auditLog(50);
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/audit-log");
      expect(url).toContain("limit=50");
      expect(result).toEqual(entries);
    });

    it("defaults to limit=100", async () => {
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: [] }));
      await api.auditLog();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("limit=100");
    });
  });

  describe("api.metrics()", () => {
    it("calls GET /metrics and returns text", async () => {
      const metricsText = "# HELP requests_total Total requests\nrequests_total 100\n";
      mockFetch.mockResolvedValue({
        ok: true,
        status: 200,
        text: async () => metricsText,
      });
      const result = await api.metrics();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/metrics");
      expect(result).toBe(metricsText);
    });

    it("throws on non-ok response", async () => {
      mockFetch.mockResolvedValue({ ok: false, status: 500, text: async () => "" });
      await expect(api.metrics()).rejects.toThrow("API error: 500");
    });

    it("includes Authorization header when token is set", async () => {
      localStorage.setItem("admin_token", "tok123");
      mockFetch.mockResolvedValue({
        ok: true,
        status: 200,
        text: async () => "metric 1",
      });
      await api.metrics();
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect((opts.headers as Record<string, string>)["Authorization"]).toBe(
        "Bearer tok123",
      );
    });
  });

  // ─── Channel auto-test ──────────────────────────────────

  describe("api.testChannel()", () => {
    it("sends POST to /api/channels/:id/test", async () => {
      const testResult = { healthy: true, latency_ms: 120, error: null };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: testResult }));
      const result = await api.testChannel("ch-1");
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/channels/ch-1/test");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("POST");
      expect(result).toEqual(testResult);
    });
  });

  describe("api.testAllChannels()", () => {
    it("sends POST to /api/channels/test-all", async () => {
      const results = [
        { channel_id: "ch-1", channel_name: "OpenAI", healthy: true, latency_ms: 50, error: null },
      ];
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: results }));
      const result = await api.testAllChannels();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/channels/test-all");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("POST");
      expect(result).toEqual(results);
    });
  });

  // ─── MCP health ─────────────────────────────────────────

  describe("api.mcpHealth()", () => {
    it("calls GET /api/mcp/health", async () => {
      const health = [
        { name: "fs", healthy: true, last_check: "2025-01-01", last_error: null, consecutive_failures: 0 },
      ];
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: health }));
      const result = await api.mcpHealth();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/mcp/health");
      expect(result).toEqual(health);
    });
  });

  // ─── MCP servers ────────────────────────────────────────

  describe("api.mcp.createServer()", () => {
    it("sends POST to /api/mcp/servers with body", async () => {
      const mockServer = { id: "fs", name: "Filesystem", command: "npx", args: [], env: {}, cwd: null, enabled: true, expose_tools: false, status: "stopped" };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: mockServer }));
      const payload = { id: "fs", name: "Filesystem", command: "npx" };
      const result = await api.mcp.createServer(payload);
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/mcp/servers");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("POST");
      const body = JSON.parse(opts.body as string);
      expect(body.id).toBe("fs");
      expect(result).toEqual(mockServer);
    });
  });

  describe("api.mcp.updateServer()", () => {
    it("sends PUT to /api/mcp/servers/:id with body", async () => {
      const mockServer = { id: "fs", name: "Updated", command: "npx", args: [], env: {}, cwd: null, enabled: false, expose_tools: false, status: "stopped" };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: mockServer }));
      const result = await api.mcp.updateServer("fs", { name: "Updated", command: "npx", enabled: false });
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/mcp/servers/fs");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("PUT");
      expect(result).toEqual(mockServer);
    });
  });

  describe("api.mcp.deleteServer()", () => {
    it("sends DELETE to /api/mcp/servers/:id", async () => {
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: null }));
      await api.mcp.deleteServer("fs");
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/mcp/servers/fs");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("DELETE");
    });
  });

  describe("api.mcp.startServer()", () => {
    it("sends POST to /api/mcp/servers/:id/start", async () => {
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: { ok: true } }));
      const result = await api.mcp.startServer("fs");
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/mcp/servers/fs/start");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("POST");
      expect(result).toEqual({ ok: true });
    });
  });

  describe("api.mcp.stopServer()", () => {
    it("sends POST to /api/mcp/servers/:id/stop", async () => {
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: { ok: true } }));
      const result = await api.mcp.stopServer("fs");
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/mcp/servers/fs/stop");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("POST");
      expect(result).toEqual({ ok: true });
    });
  });

  describe("api.mcp.listServerTools()", () => {
    it("calls GET /api/mcp/servers/:id/tools", async () => {
      const tools = [{ name: "read_file", description: "Read a file" }];
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: tools }));
      const result = await api.mcp.listServerTools("fs");
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/mcp/servers/fs/tools");
      expect(result).toEqual(tools);
    });
  });

  describe("api.mcp.listAllTools()", () => {
    it("calls GET /api/mcp/tools", async () => {
      const tools = [{ server_id: "fs", name: "read_file", description: "Read" }];
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: tools }));
      const result = await api.mcp.listAllTools();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/mcp/tools");
      expect(result).toEqual(tools);
    });
  });

  // ─── Completion ratios ──────────────────────────────────

  describe("api.completionRatios()", () => {
    it("calls GET /api/completion-ratios", async () => {
      const ratios = { "gpt-4": 0.5, "gpt-3.5": 1.0 };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: ratios }));
      const result = await api.completionRatios();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/completion-ratios");
      expect(result).toEqual(ratios);
    });
  });

  describe("api.updateCompletionRatios()", () => {
    it("sends PUT to /api/completion-ratios with body", async () => {
      const ratios = { "gpt-4": 0.3 };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: ratios }));
      const result = await api.updateCompletionRatios(ratios);
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/completion-ratios");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("PUT");
      const body = JSON.parse(opts.body as string);
      expect(body).toEqual(ratios);
      expect(result).toEqual(ratios);
    });
  });

  // ─── Model registry ─────────────────────────────────────

  describe("api.modelRegistry()", () => {
    it("calls GET /api/model-registry", async () => {
      const registry = {
        models: [
          {
            name: "gpt-4",
            source_type: "builtin",
            source: null,
            channel_id: null,
            channel_name: null,
            last_refreshed_secs: null,
            supports_thinking: false,
            supports_vision: true,
            supports_tools: true,
            max_context_tokens: 128000,
            thinking_format: "None",
          },
        ],
        total: 1,
      };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: registry }));
      const result = await api.modelRegistry();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/model-registry");
      expect(result).toEqual(registry);
    });
  });

  // ─── Payload rules ──────────────────────────────────────

  describe("api.getPayloadRules()", () => {
    it("calls GET /api/channels/:id/payload-rules", async () => {
      const rules = { defaults: { temperature: 0.7 }, overrides: {}, strip: [] };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: rules }));
      const result = await api.getPayloadRules("ch-1");
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/channels/ch-1/payload-rules");
      expect(result).toEqual(rules);
    });
  });

  describe("api.updatePayloadRules()", () => {
    it("sends PUT to /api/channels/:id/payload-rules with body", async () => {
      const rules = { strip: ["user.api_key"] };
      mockFetch.mockResolvedValue(
        jsonResponse({ ok: true, data: { channel_id: "ch-1", updated: true } }),
      );
      const result = await api.updatePayloadRules("ch-1", rules);
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/channels/ch-1/payload-rules");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("PUT");
      const body = JSON.parse(opts.body as string);
      expect(body.strip).toEqual(["user.api_key"]);
      expect(result).toEqual({ channel_id: "ch-1", updated: true });
    });
  });

  // ─── Guardrails ─────────────────────────────────────────

  describe("api.guardrailsConfig()", () => {
    it("calls GET /api/guardrails", async () => {
      const config = {
        enabled: true,
        blocked_patterns: ["sql"],
        allowed_patterns: [],
        max_request_chars: 10000,
        block_message: "Blocked",
      };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: config }));
      const result = await api.guardrailsConfig();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/guardrails");
      expect(result).toEqual(config);
    });
  });

  describe("api.updateGuardrails()", () => {
    it("sends PUT to /api/guardrails with body", async () => {
      const config = { enabled: false, blocked_patterns: [], allowed_patterns: [], max_request_chars: null, block_message: "" };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: config }));
      const result = await api.updateGuardrails({ enabled: false });
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/guardrails");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("PUT");
      const body = JSON.parse(opts.body as string);
      expect(body.enabled).toBe(false);
      expect(result).toEqual(config);
    });
  });

  // ─── Redemption codes ───────────────────────────────────

  describe("api.redemptionCodes.list()", () => {
    it("calls GET /api/redemption-codes", async () => {
      const codes = [{ code: "ABC123", credits_cents: 500, used: false }];
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: codes }));
      const result = await api.redemptionCodes.list();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/redemption-codes");
      expect(result).toEqual(codes);
    });
  });

  describe("api.redemptionCodes.create()", () => {
    it("sends POST to /api/redemption-codes with body", async () => {
      const code = { code: "XYZ789", credits_cents: 1000, used: false };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: code }));
      const result = await api.redemptionCodes.create({ credits_cents: 1000 });
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/redemption-codes");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("POST");
      const body = JSON.parse(opts.body as string);
      expect(body.credits_cents).toBe(1000);
      expect(result).toEqual(code);
    });
  });

  describe("api.redemptionCodes.redeem()", () => {
    it("sends POST to /api/redemption-codes/redeem with code and user_id", async () => {
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: { credits_cents: 500 } }));
      const result = await api.redemptionCodes.redeem("ABC123", "user-1");
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/redemption-codes/redeem");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("POST");
      const body = JSON.parse(opts.body as string);
      expect(body.code).toBe("ABC123");
      expect(body.user_id).toBe("user-1");
      expect(result).toEqual({ credits_cents: 500 });
    });
  });

  describe("api.redemptionCodes.delete()", () => {
    it("sends DELETE to /api/redemption-codes/:code", async () => {
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: null }));
      await api.redemptionCodes.delete("ABC123");
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/redemption-codes/ABC123");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("DELETE");
    });
  });

  // ─── Notification config ────────────────────────────────

  describe("api.notificationConfig()", () => {
    it("calls GET /api/notifications", async () => {
      const config = {
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
        smtp_use_tls: true,
      };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: config }));
      const result = await api.notificationConfig();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/notifications");
      expect(result).toEqual(config);
    });
  });

  describe("api.updateNotification()", () => {
    it("sends PUT to /api/notifications with body", async () => {
      const config = {
        webhook_url: "https://hook.example.com",
        webhook_secret: null,
        bark_url: null,
        budget_threshold_pct: 90,
        smtp_enabled: false,
        smtp_host: null,
        smtp_port: null,
        smtp_username: null,
        smtp_password: null,
        smtp_from: null,
        smtp_admin_email: null,
        smtp_use_tls: true,
      };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: config }));
      const result = await api.updateNotification({ budget_threshold_pct: 90 });
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/notifications");
      const opts = mockFetch.mock.calls[0][1] as RequestInit;
      expect(opts.method).toBe("PUT");
      const body = JSON.parse(opts.body as string);
      expect(body.budget_threshold_pct).toBe(90);
      expect(result).toEqual(config);
    });
  });

  // ─── Reports ────────────────────────────────────────────

  describe("api.reports.usage()", () => {
    it("calls GET /api/reports/usage with no params", async () => {
      const report = {
        rows: [],
        summary: {
          total_requests: 0,
          total_tokens: 0,
          total_cost_cents: 0,
          avg_daily_cost_cents: 0,
        },
      };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: report }));
      const result = await api.reports.usage();
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("/api/reports/usage");
      expect(url).not.toContain("?");
      expect(result).toEqual(report);
    });

    it("passes query params when provided", async () => {
      const report = {
        rows: [],
        summary: {
          total_requests: 0,
          total_tokens: 0,
          total_cost_cents: 0,
          avg_daily_cost_cents: 0,
        },
      };
      mockFetch.mockResolvedValue(jsonResponse({ ok: true, data: report }));
      await api.reports.usage({
        key_id: "vk-1",
        group: "eng",
        from: "2025-01-01",
        to: "2025-01-31",
        group_by: "day",
      });
      const url = String(mockFetch.mock.calls[0][0]);
      expect(url).toContain("key_id=vk-1");
      expect(url).toContain("group=eng");
      expect(url).toContain("from=2025-01-01");
      expect(url).toContain("to=2025-01-31");
      expect(url).toContain("group_by=day");
    });
  });
});
