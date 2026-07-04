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

vi.mock("../../hooks/useQuota", () => ({
  useQuota: () => ({
    totalBalance: 0,
    channelsWithData: 0,
    lowBalanceCount: 0,
    errorCount: 0,
    totalChannels: 0,
  }),
}));

vi.mock("../../lib/api", () => ({
  api: {
    listChannels: vi.fn(),
    stats: vi.fn(),
    logs: vi.fn(),
    usageHistory: vi.fn(),
  },
}));

import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactElement } from "react";
import { StatusDashboard } from "./StatusDashboard";
import { api } from "../../lib/api";
import type {
  Channel,
  DispatchLog,
  DispatchStats,
  UsageHistory,
} from "../../lib/api";

function renderWithProviders(ui: ReactElement) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>,
  );
}

const pendingPromise = <T,>() => new Promise<T>(() => {});

const mockChannels: Channel[] = [
  {
    id: "ch1",
    name: "Test Channel",
    provider: "openai",
    priority: 1,
    weight: 1,
    cost_per_token: null,
    input_cost_per_mtok: null,
    output_cost_per_mtok: null,
    enabled: true,
    status: "healthy",
    circuit_open_until: null,
    base_url: "https://api.openai.com/v1",
    model_mapping: {},
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    avg_latency_ms: 100,
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
  },
];
const mockStats: DispatchStats = {
  total_requests: 100,
  successes: 90,
  failures: 10,
  avg_latency_ms: 200,
};
const mockLogs: DispatchLog[] = [];
const mockUsage: UsageHistory = {
  buckets: [],
  total_input_tokens: 0,
  total_output_tokens: 0,
  total_requests: 0,
  total_cost: 0,
};

describe("StatusDashboard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders_loading_state_initially", () => {
    vi.mocked(api.listChannels).mockImplementation(() => pendingPromise());
    vi.mocked(api.stats).mockImplementation(() => pendingPromise());
    vi.mocked(api.logs).mockImplementation(() => pendingPromise());
    vi.mocked(api.usageHistory).mockImplementation(() => pendingPromise());

    renderWithProviders(<StatusDashboard />);
    expect(screen.getByText("dashboard.title")).toBeTruthy();
    expect(screen.getByText("dashboard.connecting")).toBeTruthy();
  });

  it("renders_dashboard_with_data", async () => {
    vi.mocked(api.listChannels).mockResolvedValue(mockChannels);
    vi.mocked(api.stats).mockResolvedValue(mockStats);
    vi.mocked(api.logs).mockResolvedValue(mockLogs);
    vi.mocked(api.usageHistory).mockResolvedValue(mockUsage);

    renderWithProviders(<StatusDashboard />);

    await waitFor(() => {
      expect(screen.getByText("common.refresh")).toBeTruthy();
    });
    expect(screen.getByText("dashboard.title")).toBeTruthy();
    expect(screen.getByText("dashboard.totalRequests")).toBeTruthy();
  });

  it("invokes_api_methods_on_mount", async () => {
    vi.mocked(api.listChannels).mockResolvedValue(mockChannels);
    vi.mocked(api.stats).mockResolvedValue(mockStats);
    vi.mocked(api.logs).mockResolvedValue(mockLogs);
    vi.mocked(api.usageHistory).mockResolvedValue(mockUsage);

    renderWithProviders(<StatusDashboard />);

    await waitFor(() => {
      expect(api.listChannels).toHaveBeenCalledTimes(1);
    });
    expect(api.stats).toHaveBeenCalledTimes(1);
    expect(api.logs).toHaveBeenCalledWith(0, 50);
    expect(api.usageHistory).toHaveBeenCalledWith(24);
  });
});
