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

vi.mock("../../lib/api", () => ({
  api: {
    virtualKeys: {
      list: vi.fn(),
      groups: vi.fn(),
      delete: vi.fn(),
      update: vi.fn(),
    },
  },
}));

import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactElement } from "react";
import { VirtualKeysPanel } from "./VirtualKeysPanel";
import { api } from "../../lib/api";
import type { PaginatedVirtualKeys } from "../../lib/api";

function renderWithProviders(ui: ReactElement) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>,
  );
}

const pendingPromise = <T,>() => new Promise<T>(() => {});

const emptyPage: PaginatedVirtualKeys = {
  data: [],
  total: 0,
  page: 1,
  limit: 20,
};

describe("VirtualKeysPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders_loading_state_initially", () => {
    vi.mocked(api.virtualKeys.list).mockImplementation(() => pendingPromise());
    vi.mocked(api.virtualKeys.groups).mockImplementation(() => pendingPromise());

    renderWithProviders(<VirtualKeysPanel />);
    expect(screen.getByText("virtualKeys.loading")).toBeTruthy();
  });

  it("renders_empty_state_when_no_keys", async () => {
    vi.mocked(api.virtualKeys.list).mockResolvedValue(emptyPage);
    vi.mocked(api.virtualKeys.groups).mockResolvedValue([]);

    renderWithProviders(<VirtualKeysPanel />);

    await waitFor(() => {
      expect(screen.getByText("virtualKeys.empty")).toBeTruthy();
    });
    expect(screen.getByText("virtualKeys.title")).toBeTruthy();
  });

  it("renders_create_search_and_csv_controls_when_loaded", async () => {
    vi.mocked(api.virtualKeys.list).mockResolvedValue(emptyPage);
    vi.mocked(api.virtualKeys.groups).mockResolvedValue([]);

    renderWithProviders(<VirtualKeysPanel />);

    await waitFor(() => {
      expect(screen.getByText("virtualKeys.create")).toBeTruthy();
    });
    expect(
      screen.getByPlaceholderText("virtualKeys.searchPlaceholder"),
    ).toBeTruthy();
    expect(screen.getByText("virtualKeys.csv.button")).toBeTruthy();
    expect(screen.getByText("virtualKeys.batch.button")).toBeTruthy();
  });

  it("invokes_list_and_groups_on_mount", async () => {
    vi.mocked(api.virtualKeys.list).mockResolvedValue(emptyPage);
    vi.mocked(api.virtualKeys.groups).mockResolvedValue([]);

    renderWithProviders(<VirtualKeysPanel />);

    await waitFor(() => {
      expect(api.virtualKeys.list).toHaveBeenCalledTimes(1);
    });
    expect(api.virtualKeys.groups).toHaveBeenCalledTimes(1);
  });
});
