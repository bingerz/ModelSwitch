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

import { api, request } from "./api";

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
});
