import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import type { ComponentType } from "react";
import { EmptyState } from "./EmptyState";

const MockIcon: ComponentType<{ size?: number; className?: string }> = () => (
  <svg data-testid="mock-icon" />
);

describe("EmptyState", () => {
  it("renders_without_crashing", () => {
    const { container } = render(
      <EmptyState
        icon={MockIcon}
        title="Nothing here"
        description="Try adding something"
      />,
    );
    expect(container.querySelector(".ui-empty-state")).not.toBeNull();
  });

  it("renders_message_text", () => {
    render(
      <EmptyState
        icon={MockIcon}
        title="No channels yet"
        description="Create your first channel to get started"
      />,
    );
    expect(screen.getByText("No channels yet")).toBeTruthy();
    expect(
      screen.getByText("Create your first channel to get started"),
    ).toBeTruthy();
  });

  it("renders_icon_component", () => {
    render(
      <EmptyState
        icon={MockIcon}
        title="Empty"
        description="No data"
      />,
    );
    expect(screen.getByTestId("mock-icon")).toBeTruthy();
  });
});
