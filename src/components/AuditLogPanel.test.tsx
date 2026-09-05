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

vi.mock("../lib/api", () => ({
  api: {
    auditLog: vi.fn(),
  },
}));

vi.mock("../hooks/useQueryToastError", () => ({
  useQueryToastError: vi.fn(),
}));

import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactElement } from "react";
import { AuditLogPanel } from "./AuditLogPanel";
import { api } from "../lib/api";
import type { AuditLogEntry } from "../lib/api";

function renderWithProviders(ui: ReactElement) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>,
  );
}

const mockAuditEntries: AuditLogEntry[] = [
  {
    timestamp: "2024-01-01T10:00:00Z",
    action: "create_channel",
    actor: "admin",
    target: "channel-1",
    details: "Created OpenAI channel",
  },
  {
    timestamp: "2024-01-01T11:00:00Z",
    action: "delete_channel",
    actor: "admin",
    target: "channel-2",
    details: "Deleted test channel",
  },
  {
    timestamp: "2024-01-01T12:00:00Z",
    action: "create_key",
    actor: "user1",
    target: "vk-123",
    details: "Created virtual key",
  },
  {
    timestamp: "2024-01-01T13:00:00Z",
    action: "delete_key",
    actor: "admin",
    target: "vk-456",
    details: "Revoked key",
  },
];

describe("AuditLogPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders_empty_state_when_no_entries", async () => {
    vi.mocked(api.auditLog).mockResolvedValue([]);

    renderWithProviders(<AuditLogPanel />);

    await waitFor(() => {
      expect(screen.getByText("audit.empty")).toBeTruthy();
    });
    expect(screen.getByText("audit.emptyHint")).toBeTruthy();
  });

  it("renders_audit_entries_table", async () => {
    vi.mocked(api.auditLog).mockResolvedValue(mockAuditEntries);

    renderWithProviders(<AuditLogPanel />);

    await waitFor(() => {
      expect(screen.getAllByText("create_channel").length).toBeGreaterThan(0);
    });
    expect(screen.getAllByText("delete_channel").length).toBeGreaterThan(0);
    expect(screen.getAllByText("create_key").length).toBeGreaterThan(0);
    expect(screen.getAllByText("delete_key").length).toBeGreaterThan(0);
  });

  it("filters_entries_by_action", async () => {
    vi.mocked(api.auditLog).mockResolvedValue(mockAuditEntries);

    renderWithProviders(<AuditLogPanel />);

    await waitFor(() => {
      expect(screen.getAllByText("create_channel").length).toBeGreaterThan(0);
    });

    // Find and select the action filter dropdown
    const actionSelect = screen.getByLabelText("audit.filterByAction");
    fireEvent.change(actionSelect, { target: { value: "create_channel" } });

    // After filtering, only create_channel should be in table rows
    // The badge inside a table cell should exist
    const table = screen.getByRole("table");
    expect(table.textContent).toContain("create_channel");
    expect(table.textContent).not.toContain("delete_channel");
    expect(table.textContent).not.toContain("create_key");
    expect(table.textContent).not.toContain("delete_key");
  });

  it("filters_entries_by_actor", async () => {
    vi.mocked(api.auditLog).mockResolvedValue(mockAuditEntries);

    renderWithProviders(<AuditLogPanel />);

    await waitFor(() => {
      expect(screen.getAllByText("create_channel").length).toBeGreaterThan(0);
    });

    // Find and select the actor filter dropdown
    const actorSelect = screen.getByLabelText("audit.filterByActor");
    fireEvent.change(actorSelect, { target: { value: "user1" } });

    // After filtering, only user1's entry should be in table
    const table = screen.getByRole("table");
    expect(table.textContent).toContain("create_key");
    expect(table.textContent).toContain("user1");
    expect(table.textContent).not.toContain("delete_channel");
  });

  it("combines_action_and_actor_filters", async () => {
    vi.mocked(api.auditLog).mockResolvedValue(mockAuditEntries);

    renderWithProviders(<AuditLogPanel />);

    await waitFor(() => {
      expect(screen.getAllByText("create_channel").length).toBeGreaterThan(0);
    });

    // Apply both filters
    const actionSelect = screen.getByLabelText("audit.filterByAction");
    const actorSelect = screen.getByLabelText("audit.filterByActor");

    fireEvent.change(actionSelect, { target: { value: "delete_key" } });
    fireEvent.change(actorSelect, { target: { value: "admin" } });

    // Only entry matching both filters should be in table
    const table = screen.getByRole("table");
    expect(table.textContent).toContain("delete_key");
    expect(table.textContent).not.toContain("create_channel");
  });

  it("clears_filters_with_clear_button", async () => {
    vi.mocked(api.auditLog).mockResolvedValue(mockAuditEntries);

    renderWithProviders(<AuditLogPanel />);

    await waitFor(() => {
      expect(screen.getAllByText("create_channel").length).toBeGreaterThan(0);
    });

    // Apply a filter
    const actionSelect = screen.getByLabelText("audit.filterByAction");
    fireEvent.change(actionSelect, { target: { value: "create_channel" } });

    // Click clear button
    const clearButton = screen.getByText("common.clear");
    fireEvent.click(clearButton);

    // All entries should be visible in table again
    await waitFor(() => {
      const table = screen.getByRole("table");
      expect(table.textContent).toContain("create_channel");
      expect(table.textContent).toContain("delete_channel");
      expect(table.textContent).toContain("create_key");
      expect(table.textContent).toContain("delete_key");
    });
  });

  it("shows_clear_button_only_when_filters_active", async () => {
    vi.mocked(api.auditLog).mockResolvedValue(mockAuditEntries);

    renderWithProviders(<AuditLogPanel />);

    await waitFor(() => {
      expect(screen.getAllByText("create_channel").length).toBeGreaterThan(0);
    });

    // Initially no clear button
    expect(screen.queryByText("common.clear")).not.toBeInTheDocument();

    // Apply a filter
    const actionSelect = screen.getByLabelText("audit.filterByAction");
    fireEvent.change(actionSelect, { target: { value: "create_channel" } });

    // Now clear button should appear
    expect(screen.getByText("common.clear")).toBeTruthy();
  });
});
