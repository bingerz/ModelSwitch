import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, params?: Record<string, unknown>) => {
      if (params) {
        return Object.entries(params).reduce(
          (str, [k, v]) => str.replace(`{{${k}}}`, String(v)),
          key,
        );
      }
      return key;
    },
    i18n: { language: "en", changeLanguage: vi.fn() },
  }),
}));

vi.mock("../../hooks/useQuota", () => ({
  useQuota: () => ({
    channels: [],
    loading: false,
  }),
}));

vi.mock("../../lib/api", () => ({
  api: {
    virtualKeys: {
      list: vi.fn().mockResolvedValue({ keys: [], total: 0, page: 1, limit: 1 }),
    },
  },
  buildAuthHeaders: (token: string | null) =>
    token ? { Authorization: `Bearer ${token}` } : {},
}));

vi.mock("../../lib/runtime", () => ({
  API_BASE: "http://localhost:8080",
}));

import { Playground } from "./Playground";

function renderPlayground(onNavigate = vi.fn()) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return {
    onNavigate,
    ...render(
      <QueryClientProvider client={client}>
        <Playground onNavigate={onNavigate} />
      </QueryClientProvider>,
    ),
  };
}

describe("Playground", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sessionStorage.clear();
  });

  it("shows_setup_checklist_and_channel_cta_when_empty", async () => {
    const { onNavigate } = renderPlayground();
    expect(screen.getByText("playground.readyTitle")).toBeTruthy();
    expect(
      await screen.findByText("playground.readyChannelsMissing"),
    ).toBeTruthy();
    const goChannels = screen.getAllByText("playground.goChannels")[0];
    fireEvent.click(goChannels);
    expect(onNavigate).toHaveBeenCalledWith("channels");
  });

  it("shows_quick_test_and_send_buttons", () => {
    renderPlayground();
    expect(screen.getAllByText("playground.quickTest").length).toBeGreaterThan(
      0,
    );
    expect(screen.getByText("playground.send")).toBeTruthy();
  });
});
