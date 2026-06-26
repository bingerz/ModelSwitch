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

import { describe, it, expect } from "vitest";
import { render } from "@testing-library/react";
import { LiveBadge } from "./LiveBadge";

describe("LiveBadge", () => {
  it("renders_badge_element", () => {
    const { container } = render(
      <LiveBadge lastUpdated={new Date()} />,
    );
    expect(container.querySelector(".dsh-live-badge")).not.toBeNull();
  });

  it("has_correct_css_class", () => {
    const { container } = render(
      <LiveBadge lastUpdated={new Date()} />,
    );
    expect(container.querySelector(".dsh-live-dot")).not.toBeNull();
    expect(container.querySelector(".dsh-live-label")).not.toBeNull();
    expect(container.querySelector(".dsh-live-relative")).not.toBeNull();
  });

  it("displays_live_label_text", () => {
    const { container } = render(
      <LiveBadge lastUpdated={new Date()} />,
    );
    const label = container.querySelector(".dsh-live-label");
    expect(label?.textContent).toBe("common.live");
  });
});
