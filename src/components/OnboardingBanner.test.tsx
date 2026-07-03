import { vi } from "vitest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: "en", changeLanguage: vi.fn() },
  }),
}));

import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import { OnboardingBanner } from "./OnboardingBanner";

describe("OnboardingBanner", () => {
  let store: Record<string, string> = {};

  beforeEach(() => {
    store = {};
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => store[key] ?? null,
      setItem: (key: string, value: string) => {
        store[key] = value;
      },
      removeItem: (key: string) => {
        delete store[key];
      },
      clear: () => {
        store = {};
      },
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    cleanup();
  });

  it("renders_when_not_onboarded", () => {
    render(<OnboardingBanner onNavigate={vi.fn()} />);
    expect(screen.getByText("onboarding.title")).toBeTruthy();
    expect(screen.getByText("onboarding.steps.channels")).toBeTruthy();
    expect(screen.getByText("onboarding.steps.virtualKeys")).toBeTruthy();
    expect(screen.getByText("onboarding.steps.playground")).toBeTruthy();
  });

  it("hides_after_dismiss", () => {
    render(<OnboardingBanner onNavigate={vi.fn()} />);
    fireEvent.click(screen.getByLabelText("common.dismiss"));
    expect(screen.queryByText("onboarding.title")).toBeNull();
    expect(localStorage.getItem("modelswitch_onboarded")).toBe("true");
  });

  it("does_not_render_when_already_onboarded", () => {
    localStorage.setItem("modelswitch_onboarded", "true");
    render(<OnboardingBanner onNavigate={vi.fn()} />);
    expect(screen.queryByText("onboarding.title")).toBeNull();
  });

  it("navigates_on_step_click", () => {
    const onNavigate = vi.fn();
    render(<OnboardingBanner onNavigate={onNavigate} />);
    fireEvent.click(screen.getByText("onboarding.steps.channels"));
    expect(onNavigate).toHaveBeenCalledWith("channels");
  });
});
