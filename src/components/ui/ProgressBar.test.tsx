import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { ProgressBar } from "./ProgressBar";

describe("ProgressBar", () => {
  it("renders_fill_with_correct_percentage", () => {
    const { container } = render(<ProgressBar value={50} max={100} />);
    const fill = container.querySelector(".ui-progress-fill") as HTMLElement;
    expect(fill).not.toBeNull();
    expect(fill.style.width).toBe("50%");
  });

  it("clamps_to_zero_for_negative_values", () => {
    const { container } = render(<ProgressBar value={-10} max={100} />);
    const fill = container.querySelector(".ui-progress-fill") as HTMLElement;
    expect(fill.style.width).toBe("0%");
  });

  it("clamps_to_100_for_overflow", () => {
    const { container } = render(<ProgressBar value={150} max={100} />);
    const fill = container.querySelector(".ui-progress-fill") as HTMLElement;
    expect(fill.style.width).toBe("100%");
  });

  it("handles_zero_max_safely", () => {
    const { container } = render(<ProgressBar value={50} max={0} />);
    const fill = container.querySelector(".ui-progress-fill") as HTMLElement;
    expect(fill.style.width).toBe("0%");
  });

  it("uses_threshold_color_when_within_range", () => {
    const { container } = render(
      <ProgressBar
        value={30}
        max={100}
        thresholds={[
          { upto: 50, color: "green" },
          { upto: 100, color: "red" },
        ]}
      />,
    );
    const fill = container.querySelector(".ui-progress-fill") as HTMLElement;
    expect(fill.style.background).toBe("green");
  });

  it("uses_default_color_when_no_thresholds", () => {
    const { container } = render(<ProgressBar value={50} max={100} />);
    const fill = container.querySelector(".ui-progress-fill") as HTMLElement;
    expect(fill.style.background).toBe("var(--color-accent)");
  });

  it("shows_label_when_showLabel_is_true", () => {
    render(<ProgressBar value={50} max={100} showLabel label="Custom" />);
    expect(screen.getByText("Custom")).toBeTruthy();
  });

  it("uses_custom_label_when_provided", () => {
    render(<ProgressBar value={50} max={100} showLabel label="Custom" />);
    expect(screen.getByText("Custom")).toBeTruthy();
  });

  it("shows_percentage_when_no_custom_label", () => {
    render(<ProgressBar value={42} max={100} showLabel />);
    expect(screen.getByText("42%")).toBeTruthy();
  });
});
