import { vi } from "vitest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, params?: Record<string, unknown>) =>
      params
        ? Object.entries(params).reduce(
            (s, [k, v]) => s.replace(`{${k}}`, String(v)),
            key,
          )
        : key,
    i18n: { language: "en", changeLanguage: vi.fn() },
  }),
}));

vi.mock("../lib/mock-flag", () => ({
  isMockMode: () => false,
  setMockMode: vi.fn(),
}));

vi.mock("../lib/api", () => ({
  api: {
    cacheStats: vi.fn(),
    gatewayInfo: vi.fn(),
    providerBudgets: vi.fn(),
    completionRatios: vi.fn(),
    flushCache: vi.fn(),
    reloadConfig: vi.fn(),
    setProviderBudget: vi.fn(),
    deleteProviderBudget: vi.fn(),
    updateCompletionRatios: vi.fn(),
  },
}));

import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactElement } from "react";
import { SettingsPanel } from "./SettingsPanel";
import { api } from "../lib/api";
import type {
  CacheStats,
  GatewayInfo,
  ProviderBudgetEntry,
} from "../lib/api";

function renderWithProviders(ui: ReactElement) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>,
  );
}

const pendingPromise = <T,>() => new Promise<T>(() => {});

const mockCache: CacheStats = {
  entries: 10,
  mode: "memory",
  hits: 5,
  misses: 5,
  evictions: 0,
  hit_rate_percent: 50,
  total_requests: 10,
};

const mockGateway: GatewayInfo = {
  version: "1.0.0",
  uptime_seconds: 3600,
  uptime_formatted: "1h",
  total_channels: 5,
  healthy_channels: 4,
  active_requests: 0,
  cache_entries: 10,
  routing_strategy: "priority",
  max_retries: 3,
};

const mockBudgets: ProviderBudgetEntry[] = [];

describe("SettingsPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders_loading_state_initially", () => {
    vi.mocked(api.cacheStats).mockImplementation(() => pendingPromise());
    vi.mocked(api.gatewayInfo).mockImplementation(() => pendingPromise());
    vi.mocked(api.providerBudgets).mockImplementation(() => pendingPromise());
    vi.mocked(api.completionRatios).mockImplementation(() => pendingPromise());

    renderWithProviders(<SettingsPanel />);
    expect(screen.getByText("settings.loading")).toBeTruthy();
  });

  it("renders_settings_sections_when_loaded", async () => {
    vi.mocked(api.cacheStats).mockResolvedValue(mockCache);
    vi.mocked(api.gatewayInfo).mockResolvedValue(mockGateway);
    vi.mocked(api.providerBudgets).mockResolvedValue(mockBudgets);
    vi.mocked(api.completionRatios).mockResolvedValue({});

    renderWithProviders(<SettingsPanel />);

    // Wait for loading to finish and content to appear
    await waitFor(() => {
      expect(screen.getByText("settings.title")).toBeTruthy();
      expect(screen.getByText("settings.demoMode")).toBeTruthy();
      expect(screen.getByText("settings.config")).toBeTruthy();
      expect(screen.getByText("settings.providerBudgets")).toBeTruthy();
      expect(screen.getByText("settings.completionRatios")).toBeTruthy();
      expect(screen.getByText("settings.gatewayInfo")).toBeTruthy();
      expect(screen.getByText("settings.cacheManagement")).toBeTruthy();
    });
  });

  it("renders_reload_config_button_when_loaded", async () => {
    vi.mocked(api.cacheStats).mockResolvedValue(mockCache);
    vi.mocked(api.gatewayInfo).mockResolvedValue(mockGateway);
    vi.mocked(api.providerBudgets).mockResolvedValue(mockBudgets);
    vi.mocked(api.completionRatios).mockResolvedValue({});

    renderWithProviders(<SettingsPanel />);

    await waitFor(() => {
      expect(screen.getByText("settings.reloadConfig")).toBeTruthy();
    });
  });
});
