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
  useQuota: vi.fn(),
}));

vi.mock("../../lib/api", () => ({
  api: {
    deleteChannel: vi.fn(),
    pingChannel: vi.fn(),
    testAllChannels: vi.fn(),
    testChannel: vi.fn(),
    updateChannel: vi.fn(),
  },
}));

import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactElement } from "react";
import { ChannelPanel } from "./ChannelPanel";
import { useQuota } from "../../hooks/useQuota";

const mockedUseQuota = vi.mocked(useQuota);

function renderWithProviders(ui: ReactElement) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>,
  );
}

function mockQuota(
  overrides: Partial<ReturnType<typeof useQuota>> = {},
): ReturnType<typeof useQuota> {
  const value = {
    quotas: [],
    channels: [],
    usageHistory: null,
    refresh: vi.fn(),
    fetchUsageHistory: vi.fn(),
    totalBalance: 0,
    channelsWithData: 0,
    lowBalanceCount: 0,
    errorCount: 0,
    fetchError: null,
    loading: false,
    totalChannels: 0,
    ...overrides,
  };
  mockedUseQuota.mockReturnValue(value);
  return value;
}

describe("ChannelPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders_loading_state_when_quota_loading", () => {
    mockQuota({ loading: true });
    renderWithProviders(<ChannelPanel />);
    expect(screen.getByText("channels.loadingChannels")).toBeTruthy();
  });

  it("renders_panel_header_and_controls_when_loaded", () => {
    mockQuota({ loading: false, channels: [], totalChannels: 0 });
    renderWithProviders(<ChannelPanel />);
    expect(screen.getByText("channels.title")).toBeTruthy();
    expect(screen.getByText("channels.create")).toBeTruthy();
    expect(screen.getByText("channels.testAll")).toBeTruthy();
    expect(
      screen.getByPlaceholderText("channels.searchPlaceholder"),
    ).toBeTruthy();
  });

  it("renders_priority_tier_labels_when_loaded", () => {
    mockQuota({ loading: false, channels: [], totalChannels: 0 });
    renderWithProviders(<ChannelPanel />);
    // Three priority tiers render with i18n keys (mock returns the key text).
    expect(screen.getByText("channels.priority1Label")).toBeTruthy();
    expect(screen.getByText("channels.priority2Label")).toBeTruthy();
    expect(screen.getByText("channels.priority3Label")).toBeTruthy();
  });
});
