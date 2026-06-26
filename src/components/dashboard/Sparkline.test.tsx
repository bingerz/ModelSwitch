import { vi, describe, it, expect } from "vitest";

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

import { render } from "@testing-library/react";
import { Sparkline } from "./Sparkline";

describe("Sparkline", () => {
  it("renders_svg_element", () => {
    const { container } = render(<Sparkline values={[1, 2, 3]} />);
    const svg = container.querySelector("svg");
    expect(svg).not.toBeNull();
  });

  it("renders_empty_state_for_no_values", () => {
    const { container } = render(<Sparkline values={[]} />);
    const svg = container.querySelector("svg");
    expect(svg).not.toBeNull();
    expect(svg?.className.baseVal ?? svg?.getAttribute("class") ?? "").toContain(
      "dsh-sparkline-empty",
    );
  });

  it("renders_polypath_for_non_empty_values", () => {
    const { container } = render(<Sparkline values={[1, 2, 3]} />);
    const pathElements = container.querySelectorAll("path");
    expect(pathElements.length).toBeGreaterThanOrEqual(1);
  });

  it("uses_custom_dimensions", () => {
    const { container } = render(
      <Sparkline values={[1, 2, 3]} width={200} height={50} />,
    );
    const svg = container.querySelector("svg") as SVGSVGElement;
    expect(svg.getAttribute("viewBox")).toBe("0 0 200 50");
  });
});