import { vi, describe, it, expect, beforeEach } from "vitest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, params?: Record<string, unknown>) => {
      if (params) {
        return Object.entries(params).reduce(
          (str, [k, v]) => str.replace(`{${k}}`, String(v)),
          key,
        );
      }
      return key;
    },
    i18n: { language: "en", changeLanguage: vi.fn() },
  }),
}));

import { render, screen, fireEvent } from "@testing-library/react";
import type { VirtualKey } from "../../lib/api";
import { VirtualKeyCard } from "./VirtualKeyCard";

const mockVk: VirtualKey = {
  id: "vk-1",
  name: "Test Key",
  key_prefix: "msw-abcd",
  daily_budget_cents: null,
  monthly_budget_cents: 5000,
  enabled: true,
  spend: {
    today: { date: "2024-01-01", cents: 1000 },
    this_month: { month: "Jan", cents: 2000 },
    total_cents: 3000,
  },
  created_at: "2024-01-01T00:00:00Z",
  allowed_ips: [],
  allowed_models: null,
  denied_models: [],
  rpm_limit: null,
  tpm_limit: null,
  expires_at: null,
  group: null,
};

function renderCard(overrides: Partial<Parameters<typeof VirtualKeyCard>[0]> = {}) {
  const onEdit = vi.fn();
  const onDelete = vi.fn();
  const onToggleEnabled = vi.fn();
  const utils = render(
    <VirtualKeyCard
      vk={mockVk}
      confirmDelete={false}
      actionLoading={false}
      onEdit={onEdit}
      onDelete={onDelete}
      onToggleEnabled={onToggleEnabled}
      {...overrides}
    />,
  );
  return { ...utils, onEdit, onDelete, onToggleEnabled };
}

describe("VirtualKeyCard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders_key_name_and_prefix", () => {
    renderCard();
    expect(screen.getByText("Test Key")).toBeTruthy();
    expect(screen.getByText(/msw-abcd/)).toBeTruthy();
  });

  it("shows_delete_confirmation_when_confirmDelete_is_true", () => {
    renderCard({ confirmDelete: true });
    // With i18n mock, t("common.confirmQuestion") returns "common.confirmQuestion"
    expect(screen.getByText("common.confirmQuestion")).toBeTruthy();
  });

  it("does_not_show_confirmation_by_default", () => {
    renderCard({ confirmDelete: false });
    expect(screen.queryByText("common.confirmQuestion")).toBeNull();
    expect(screen.getByText("common.delete")).toBeTruthy();
  });

  it("disables_buttons_when_actionLoading", () => {
    renderCard({ actionLoading: true });
    const buttons = screen.getAllByRole("button");
    for (const btn of buttons) {
      expect((btn as HTMLButtonElement).disabled).toBe(true);
    }
  });

  it("calls_onEdit_when_edit_button_clicked", () => {
    const { onEdit } = renderCard();
    const editBtn = screen.getByText("common.edit");
    fireEvent.click(editBtn);
    expect(onEdit).toHaveBeenCalledTimes(1);
  });

  it("calls_onDelete_when_delete_clicked", () => {
    const { onDelete } = renderCard();
    const deleteBtn = screen.getByText("common.delete");
    fireEvent.click(deleteBtn);
    expect(onDelete).toHaveBeenCalledTimes(1);
  });
});