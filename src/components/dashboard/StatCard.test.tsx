import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import type { ComponentType } from "react";
import { StatCard } from "./StatCard";

const MockIcon: ComponentType<{ size?: number; className?: string }> = () => (
  <svg data-testid="mock-icon" />
);

describe("StatCard", () => {
  it("renders_value_and_label", () => {
    render(
      <StatCard icon={MockIcon} value={42} label="Total" accent="blue" />,
    );
    expect(screen.getByText("42")).toBeTruthy();
    expect(screen.getByText("Total")).toBeTruthy();
  });

  it("renders_string_value", () => {
    render(
      <StatCard
        icon={MockIcon}
        value="$1.23k"
        label="Cost"
        accent="green"
      />,
    );
    expect(screen.getByText("$1.23k")).toBeTruthy();
  });

  it("renders_subtitle_when_provided", () => {
    render(
      <StatCard
        icon={MockIcon}
        value={42}
        label="Total"
        accent="blue"
        subtitle="extra"
      />,
    );
    expect(screen.getByText("extra")).toBeTruthy();
  });

  it("does_not_render_subtitle_when_absent", () => {
    render(
      <StatCard icon={MockIcon} value={42} label="Total" accent="blue" />,
    );
    expect(screen.queryByText("extra")).toBeNull();
  });

  it("renders_trend_up_indicator", () => {
    render(
      <StatCard
        icon={MockIcon}
        value={42}
        label="Total"
        accent="blue"
        trend="up"
        trendLabel="+12%"
      />,
    );
    expect(screen.getByText("+12%")).toBeTruthy();
  });

  it("renders_default_size", () => {
    const { container } = render(
      <StatCard icon={MockIcon} value={42} label="Total" accent="blue" />,
    );
    // Should render without error; the container should have a child
    expect(
      container.querySelector('[class*="dsh-stat-card"]'),
    ).not.toBeNull();
  });
});